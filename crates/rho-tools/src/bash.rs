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
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if tx.send(line).await.is_err() {
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
