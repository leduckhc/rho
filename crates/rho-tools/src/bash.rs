//! The `bash` tool. It runs a shell command in the session root.
//!
//! The tool streams output line by line. It enforces a timeout. On timeout or on
//! cancel it kills the whole process group, not only the direct child, so an
//! orphaned grandchild cannot keep running.

use crate::args::parse_args;
use async_trait::async_trait;
use rho_core::{Tool, ToolContext, ToolError, ToolKind, ToolOutput};
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

/// The default timeout in milliseconds.
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
/// The largest timeout a caller may set, in milliseconds.
const MAX_TIMEOUT_MS: u64 = 600_000;
/// The largest combined output stored, in bytes. Output over this is truncated.
const MAX_OUTPUT_BYTES: usize = 100_000;

/// The largest single line the reader will hold in memory.
///
/// This cap is separate from `MAX_OUTPUT_BYTES`, and it is the one that protects
/// the host. `MAX_OUTPUT_BYTES` bounds what we *keep*. This bounds what we *read*.
/// A command can emit hundreds of megabytes with no newline, for example
/// `head -c 400000000 /dev/zero | tr -d "\\0"`. A line reader with no cap grows
/// its buffer to hold all of it, so resident memory tracks the command's output.
/// A hostile or careless command then kills the process. That is fatal for a host
/// that runs many sessions at once.
///
/// A line longer than this cap is split, not dropped. So the output is still
/// correct, and memory stays bounded. `rho-plugin` guards the same way.
const MAX_LINE_BYTES: usize = 64 * 1024;

/// Arguments for `bash`.
#[derive(Debug, Serialize, Deserialize)]
struct BashArgs {
    /// The shell command to run.
    command: String,
    /// The timeout in milliseconds. The default is 120000. The maximum is 600000.
    #[serde(default)]
    timeout_ms: Option<u64>,
}

/// Runs a shell command. Mutating, so it needs approval.
pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }
    fn description(&self) -> &str {
        "Run a shell command in the session root. It streams output. It enforces \
         a timeout and kills the process group on timeout or cancel."
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to run." },
                "timeout_ms": { "type": "integer", "minimum": 1, "maximum": MAX_TIMEOUT_MS, "description": "Timeout in milliseconds. Default 120000, maximum 600000." }
            },
            "required": ["command"]
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: BashArgs = parse_args(args)?;
        let timeout_ms = args
            .timeout_ms
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);

        let mut command = tokio::process::Command::new("sh");
        command
            .arg("-c")
            .arg(&args.command)
            .current_dir(&ctx.session_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        scrub_environment(&mut command);
        // Put the child in its own process group. So a later kill on the negated
        // pid reaches every grandchild, not only the direct child.
        #[cfg(unix)]
        command.process_group(0);

        let mut child = command.spawn().map_err(|error| {
            ToolError::Io(format!(
                "cannot start the command: {error}. Check the command."
            ))
        })?;

        let pid = child.id();
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");

        // Two reader tasks merge stdout and stderr into one ordered line channel.
        let (line_tx, mut line_rx) = mpsc::channel::<String>(64);
        spawn_reader(stdout, line_tx.clone());
        spawn_reader(stderr, line_tx);

        let mut combined = String::new();
        let mut truncated = false;
        let deadline = tokio::time::sleep(Duration::from_millis(timeout_ms));
        tokio::pin!(deadline);

        let status = loop {
            tokio::select! {
                biased;
                _ = ctx.cancel.cancelled() => {
                    kill_group(pid);
                    return Err(ToolError::Canceled);
                }
                _ = &mut deadline => {
                    kill_group(pid);
                    return Err(ToolError::Timeout(Duration::from_millis(timeout_ms)));
                }
                maybe_line = line_rx.recv() => {
                    match maybe_line {
                        Some(line) => {
                            let _ = ctx.updates.send(line.clone()).await;
                            append_line(&mut combined, &line, &mut truncated);
                        }
                        None => break child.wait().await,
                    }
                }
                status = child.wait() => break status,
            }
        };

        // The process finished. Drain any line the readers still hold.
        while let Some(line) = line_rx.recv().await {
            let _ = ctx.updates.send(line.clone()).await;
            append_line(&mut combined, &line, &mut truncated);
        }

        let status = status
            .map_err(|error| ToolError::Io(format!("cannot wait for the command: {error}.")))?;

        if truncated {
            combined.push_str(&format!(
                "\n[truncated: output over {MAX_OUTPUT_BYTES} bytes.]"
            ));
        }
        let is_error = !status.success();
        if is_error {
            match status.code() {
                Some(code) => combined.push_str(&format!("\n[exit code {code}]")),
                None => combined.push_str("\n[terminated by a signal]"),
            }
        }
        Ok(ToolOutput {
            content: vec![rho_core::ContentBlock::Text { text: combined }],
            is_error,
        })
    }
}

/// Spawn a task that reads lines from `reader` and forwards them on `tx`.
fn spawn_reader<R>(reader: R, tx: mpsc::Sender<String>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = BufReader::with_capacity(8 * 1024, reader);
        let mut line: Vec<u8> = Vec::with_capacity(1024);
        loop {
            // Scan the filled buffer instead of reading one byte at a time. A byte
            // loop is correct but slow, and start-up and throughput are features.
            let chunk = match reader.fill_buf().await {
                Ok([]) => {
                    // End of stream. Send whatever is left, so a file with no final
                    // newline still reports its last line.
                    if !line.is_empty() {
                        let _ = tx.send(String::from_utf8_lossy(&line).into_owned()).await;
                    }
                    break;
                }
                Ok(chunk) => chunk,
                Err(_) => break,
            };

            // Take up to the newline, and never more than the remaining line budget.
            let budget = MAX_LINE_BYTES - line.len();
            let take = match chunk.iter().position(|byte| *byte == b'\n') {
                Some(index) if index < budget => index + 1,
                _ => budget.min(chunk.len()),
            };
            let ends_line = chunk[take - 1] == b'\n';
            line.extend_from_slice(&chunk[..take]);
            reader.consume(take);

            if ends_line {
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
            } else if line.len() < MAX_LINE_BYTES {
                // The chunk held no newline and the budget is not spent yet, so wait
                // for more bytes before emitting anything.
                continue;
            }

            // Emit either a complete line, or a segment of an over-long line. An
            // over-long line is split, never dropped, so the output stays correct
            // while memory stays bounded.
            let text = String::from_utf8_lossy(&line).into_owned();
            line.clear();
            if tx.send(text).await.is_err() {
                break;
            }
        }
    });
}

/// Append one line to the combined buffer, up to the byte cap.
fn append_line(combined: &mut String, line: &str, truncated: &mut bool) {
    if *truncated {
        return;
    }
    let addition = line.len() + 1;
    if combined.len() + addition > MAX_OUTPUT_BYTES {
        *truncated = true;
        return;
    }
    combined.push_str(line);
    combined.push('\n');
}

/// Kill the whole process group of `pid`. A negated pid names the group.
#[cfg(unix)]
fn kill_group(pid: Option<u32>) {
    if let Some(pid) = pid {
        // Safe: `kill` is a plain syscall. A stale pid returns an error, which we
        // ignore, because the process may have already exited.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
}

/// On a non-Unix host there is no process group. Kill nothing here; `kill_on_drop`
/// still reaps the direct child.
#[cfg(not(unix))]
fn kill_group(_pid: Option<u32>) {}

/// Remove every variable whose name looks like a secret from the child's environment.
///
/// The model's tool arguments are attacker-controlled input, because a prompt
/// injection can live in any file the agent reads. A security audit demonstrated the
/// risk: a model asked `bash` to print the environment and saw `OPENROUTER_API_KEY`
/// and `AWS_SECRET_ACCESS_KEY`. One network call then exfiltrates them.
///
/// The filter works on the **name**, not the value, because a value cannot be
/// recognised reliably. It removes the name as well as the value, so the mere
/// presence of a credential leaks nothing.
///
/// **This is defence in depth, not a boundary. Read this before you trust it.**
/// A tool that runs a shell command can still read a credential file on disk, for
/// example `~/.aws/credentials` or a shell profile. `bash` also has no path
/// confinement, because `cd` and an absolute path both leave the session root. So the
/// real boundary for `bash` is the approval policy. Run `--read-only` against a
/// repository you do not trust; that denies `bash` outright.
fn scrub_environment(command: &mut tokio::process::Command) {
    for (name, _) in std::env::vars_os() {
        let text = name.to_string_lossy().to_ascii_uppercase();
        if looks_like_a_secret(&text) {
            command.env_remove(&name);
        }
    }
}

/// True when a variable name suggests it holds a credential.
///
/// The list is a denylist, and that choice needs a reason. An allowlist would be
/// safer in principle, but a command legitimately needs a wide and open-ended set of
/// variables, so an allowlist would break ordinary work and users would switch it
/// off. A denylist that catches the recognisable shapes is the useful trade here.
///
/// Add a pattern when you meet a new one. The cost of a false positive is small: a
/// command loses one variable. The cost of a false negative is a leaked key.
fn looks_like_a_secret(name: &str) -> bool {
    const NEEDLES: &[&str] = &[
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "PRIVATE_KEY",
        "API_KEY",
        "APIKEY",
        "ACCESS_KEY",
        "AUTH_TOKEN",
        "SESSION_TOKEN",
        "REFRESH_TOKEN",
        "BEARER",
    ];
    if NEEDLES.iter().any(|needle| name.contains(needle)) {
        return true;
    }
    // A bare `*_TOKEN` or `*_KEY` is usually a credential. Keep a short allowlist for
    // the common names that are not, so ordinary work does not break.
    const NOT_SECRETS: &[&str] = &["SSH_AUTH_SOCK", "GPG_TTY", "KEYBOARD", "KEYMAP"];
    if NOT_SECRETS.contains(&name) {
        return false;
    }
    name.ends_with("_TOKEN") || name.ends_with("_KEY")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive `spawn_reader` over an in-memory input and collect what it emits.
    async fn read_all(input: Vec<u8>) -> Vec<String> {
        let (tx, mut rx) = mpsc::channel(1024);
        spawn_reader(std::io::Cursor::new(input), tx);
        let mut out = Vec::new();
        while let Some(line) = rx.recv().await {
            out.push(line);
        }
        out
    }

    #[tokio::test]
    async fn reader_splits_a_line_that_never_ends() {
        // The security regression test for the reader.
        //
        // A command can emit a large amount of output with no newline at all. A line
        // reader with no cap grows its buffer to hold the whole run, so the host's
        // resident memory tracks the command's output. A security audit drove that
        // case to 805 MB of resident memory and an out-of-memory kill. That is fatal
        // for a host built to run many sessions at once.
        //
        // The property under test is the fix, stated directly: no emitted piece is
        // larger than the cap. So memory stays bounded whatever the command does.
        //
        // Note what this test does not do. It does not measure resident memory. An
        // earlier attempt asserted on the size of the kept output, and it passed
        // against the unbounded reader, because the output cap bounded the kept text
        // while the read buffer still grew without limit. A test that passes against
        // the broken code is worse than no test.
        let size = 8 * MAX_LINE_BYTES + 7;
        let pieces = read_all(vec![b'A'; size]).await;

        assert!(
            pieces.len() > 1,
            "an over-long line must be split, got {} piece(s)",
            pieces.len()
        );
        for piece in &pieces {
            assert!(
                piece.len() <= MAX_LINE_BYTES,
                "a piece of {} bytes passed the {MAX_LINE_BYTES} byte cap",
                piece.len()
            );
        }
        // Splitting must lose nothing. An over-long line is cut, never dropped.
        assert_eq!(pieces.iter().map(String::len).sum::<usize>(), size);
    }

    #[tokio::test]
    async fn reader_keeps_ordinary_lines_whole() {
        let pieces = read_all(b"first\nsecond\nthird\n".to_vec()).await;
        assert_eq!(pieces, vec!["first", "second", "third"]);
    }

    #[tokio::test]
    async fn reader_emits_a_final_line_with_no_newline() {
        let pieces = read_all(b"first\nno trailing newline".to_vec()).await;
        assert_eq!(pieces, vec!["first", "no trailing newline"]);
    }

    #[tokio::test]
    async fn reader_strips_a_carriage_return() {
        let pieces = read_all(b"windows\r\nunix\n".to_vec()).await;
        assert_eq!(pieces, vec!["windows", "unix"]);
    }

    #[tokio::test]
    async fn reader_handles_an_empty_stream() {
        assert!(read_all(Vec::new()).await.is_empty());
    }

    #[tokio::test]
    async fn reader_handles_consecutive_newlines() {
        let pieces = read_all(b"a\n\nb\n".to_vec()).await;
        assert_eq!(pieces, vec!["a", "", "b"]);
    }
}
