//! The `bash` tool. It runs a shell command in the session root.
//!
//! The tool streams output line by line. It enforces a timeout. On timeout or on
//! cancel it kills the whole process group, not only the direct child, so an
//! orphaned grandchild cannot keep running.

use crate::args::parse_args;
use crate::progress::{ProgressScan, scan_line};
use crate::sandbox;
use async_trait::async_trait;
use rho_core::{
    BackgroundReason, DEFAULT_FOREGROUND_LIMIT_MS, RunMode, SandboxMode, TaskError, TaskHandle,
    TaskRegistry, TaskState, Tool, ToolContext, ToolError, ToolKind, ToolOutput, decide_run_mode,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
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
    /// Run the command in the background. When omitted, rho decides with a
    /// heuristic. An explicit value overrides the heuristic either way.
    #[serde(default)]
    run_in_background: Option<bool>,
}

/// Runs a shell command. Mutating, so it needs approval.
///
/// The tool may run a command in the background. A background command returns at
/// once with a task id, so the conversation is not blocked. rho decides between
/// foreground and background with a heuristic, and the model may override it. A
/// background run needs a task registry, so a `BashTool` built with `new` runs
/// only in the foreground.
#[derive(Default)]
pub struct BashTool {
    tasks: Option<Arc<TaskRegistry>>,
    /// The OS confinement mode for the command. The default is `Off`, which runs
    /// the command unconfined, as before. See `SPEC-bash-sandbox`.
    sandbox: SandboxMode,
}

impl BashTool {
    /// Build a foreground-only `bash` tool. It has no task registry, so it never
    /// backgrounds a command.
    pub fn new() -> Self {
        Self {
            tasks: None,
            sandbox: SandboxMode::Off,
        }
    }

    /// Build a `bash` tool that can run a command in the background. It shares
    /// the session task registry with the `task` and `task_cancel` tools.
    pub fn with_tasks(tasks: Arc<TaskRegistry>) -> Self {
        Self {
            tasks: Some(tasks),
            sandbox: SandboxMode::Off,
        }
    }

    /// Set the OS confinement mode. The default is `SandboxMode::Off`. See
    /// `SPEC-bash-sandbox`.
    pub fn sandbox(mut self, mode: SandboxMode) -> Self {
        self.sandbox = mode;
        self
    }
}

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
                "timeout_ms": { "type": "integer", "minimum": 1, "maximum": MAX_TIMEOUT_MS, "description": "Timeout in milliseconds. Default 120000, maximum 600000." },
                "run_in_background": { "type": "boolean", "description": "Run in the background and return at once with a task id. When omitted, rho decides." }
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

        // Decide foreground or background. The decision reads the requested
        // timeout, not the resolved default, so a plain command with no explicit
        // timeout is never backgrounded by rule 2. A registry must exist to run a
        // background task; without one, every command runs in the foreground.
        let requested_timeout = args.timeout_ms.unwrap_or(0);
        let mode = decide_run_mode(
            &args.command,
            args.run_in_background,
            requested_timeout,
            DEFAULT_FOREGROUND_LIMIT_MS,
        );
        if let (RunMode::Background(reason), Some(registry)) = (&mode, &self.tasks) {
            return start_background(
                registry,
                &args.command,
                *reason,
                &ctx.session_root,
                background_timeout_ms(registry, args.timeout_ms),
                self.sandbox,
            );
        }

        let mut command = build_command(&args.command, &ctx.session_root, false, self.sandbox)?;

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
                    // The command outran its foreground timeout. Adopt it into the
                    // background rather than kill it, so its work is not thrown
                    // away. Adoption needs a registry; without one, kill it.
                    if let Some(registry) = &self.tasks {
                        return adopt_on_timeout(
                            registry,
                            &args.command,
                            child,
                            line_rx,
                            &combined,
                            background_timeout_ms(registry, args.timeout_ms),
                        );
                    }
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

/// Build the command with the shared hardening: no stdin, piped output, kill on
/// drop, a scrubbed environment, and its own process group. A background command
/// also gets `RHO_PROGRESS_FD=1`, so a script can detect a progress consumer and
/// opt in with one `echo`.
///
/// Under a confinement `sandbox`, the program is the OS sandbox wrapper, not `sh`
/// directly. When the mode needs confinement and no backend is available, this
/// fails closed with an error, so the command never runs unconfined. See
/// `SPEC-bash-sandbox`.
/// Build a command for a gate check, under the parent's sandbox.
///
/// It reuses `build_command`, so an acceptance check is confined exactly like a
/// `bash` call. A separate path would drift, and the drift would be a hole.
pub(crate) fn build_gate_command(
    command_str: &str,
    session_root: &Path,
    sandbox: SandboxMode,
) -> Result<tokio::process::Command, ToolError> {
    build_command(command_str, session_root, false, sandbox)
}

fn build_command(
    command_str: &str,
    session_root: &Path,
    background: bool,
    sandbox: SandboxMode,
) -> Result<tokio::process::Command, ToolError> {
    // The scratch directory is a write target under confinement, so the child can
    // still use its temporary space. Build the plan first; it fails closed when a
    // confinement mode is asked for and no OS backend is available.
    let scratch = scratch_dir();
    let run = sandbox::plan(sandbox, session_root, scratch.as_deref(), command_str)
        .map_err(|error| ToolError::Io(error.to_string()))?;
    let mut command = tokio::process::Command::new(&run.program);
    command
        .args(&run.args)
        .current_dir(session_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    scrub_environment(&mut command);
    configure_scratch_dir(&mut command);
    if background {
        command.env("RHO_PROGRESS_FD", "1");
    }
    // Put the child in its own process group. So a later kill on the negated pid
    // reaches every grandchild, not only the direct child.
    #[cfg(unix)]
    command.process_group(0);
    Ok(command)
}

/// The effective timeout for a background task, in milliseconds. Use the model's
/// requested timeout when it gave one, else the registry default. Cap it at the
/// registry maximum, because a background task is unsupervised.
fn background_timeout_ms(registry: &TaskRegistry, requested: Option<u64>) -> u64 {
    let limits = registry.limits();
    requested
        .unwrap_or(limits.default_timeout_ms)
        .min(limits.max_timeout_ms)
}

/// Start a command in the background. Return at once with the task id and the
/// reason, so the model can carry on.
fn start_background(
    registry: &Arc<TaskRegistry>,
    command_str: &str,
    reason: BackgroundReason,
    session_root: &Path,
    timeout_ms: u64,
    sandbox: SandboxMode,
) -> Result<ToolOutput, ToolError> {
    let handle = registry
        .start(command_str, reason)
        .map_err(map_task_error)?;
    let mut command = match build_command(command_str, session_root, true, sandbox) {
        Ok(command) => command,
        Err(error) => {
            // The plan failed, for example the sandbox is unavailable. The task
            // must not linger in the registry as a running task.
            handle.finish(TaskState::Exited { code: -1 });
            return Err(error);
        }
    };
    let mut child = command.spawn().map_err(|error| {
        // The task never started, so it must not linger in the registry as a
        // running task. Mark it finished with a non-zero code.
        handle.finish(TaskState::Exited { code: -1 });
        ToolError::Io(format!(
            "cannot start the command: {error}. Check the command."
        ))
    })?;
    let pid = child.id();
    handle.set_killer(move || kill_group(pid));
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let (line_tx, line_rx) = mpsc::channel::<String>(64);
    spawn_reader(stdout, line_tx.clone());
    spawn_reader(stderr, line_tx);
    let id = handle.id();
    tokio::spawn(supervise(child, line_rx, handle, timeout_ms));
    Ok(started_output(&id, reason, timeout_ms))
}

/// Adopt a foreground command that outran its timeout. Seed the task with the
/// output from before adoption, then supervise it to the end.
fn adopt_on_timeout(
    registry: &Arc<TaskRegistry>,
    command_str: &str,
    child: tokio::process::Child,
    line_rx: mpsc::Receiver<String>,
    output_so_far: &str,
    timeout_ms: u64,
) -> Result<ToolOutput, ToolError> {
    let handle = registry
        .start(command_str, BackgroundReason::AdoptedOnTimeout)
        .map_err(map_task_error)?;
    let pid = child.id();
    handle.set_killer(move || kill_group(pid));
    // Keep the output from before adoption, so no work is thrown away.
    for line in output_so_far.lines() {
        handle.push_output(line);
    }
    let id = handle.id();
    tokio::spawn(supervise(child, line_rx, handle, timeout_ms));
    Ok(started_output(
        &id,
        BackgroundReason::AdoptedOnTimeout,
        timeout_ms,
    ))
}

/// Supervise one background child to its final state.
///
/// This task awaits `child.wait()`, and that await is the completion
/// notification. The task is already awaiting before the child can exit, so the
/// exit cannot be missed. It reads the exit code, not a bare signal, and it works
/// the same on Windows. See `SPEC-background-tasks` section 3. So there is no signal handler
/// and no poll loop.
async fn supervise(
    mut child: tokio::process::Child,
    mut line_rx: mpsc::Receiver<String>,
    handle: TaskHandle,
    timeout_ms: u64,
) {
    let pid = child.id();
    let deadline = tokio::time::sleep(Duration::from_millis(timeout_ms));
    tokio::pin!(deadline);
    let mut saw_explicit = false;
    let mut timed_out = false;
    loop {
        tokio::select! {
            biased;
            _ = &mut deadline => {
                kill_group(pid);
                timed_out = true;
                break;
            }
            maybe_line = line_rx.recv() => match maybe_line {
                Some(line) => process_progress_line(&handle, &line, &mut saw_explicit),
                // The readers reached the end of both streams, so the child
                // closed its output. Await the exit next.
                None => break,
            }
        }
    }
    let status = child.wait().await;
    let state = if handle.cancel_requested() {
        TaskState::Canceled
    } else if timed_out {
        TaskState::TimedOut
    } else {
        state_from_status(status)
    };
    // Always report, whatever the task wrote. A finished task is never silent.
    handle.finish(state);
}

/// Fold one output line into the task. An explicit progress line is consumed. An
/// ordinary line is kept, and may carry inferred progress. Inference never
/// overrides an explicit progress line.
fn process_progress_line(handle: &TaskHandle, line: &str, saw_explicit: &mut bool) {
    match scan_line(line) {
        ProgressScan::Explicit(progress) => {
            *saw_explicit = true;
            handle.report_progress(progress);
        }
        ProgressScan::Output { inferred } => {
            handle.push_output(line);
            if let Some(progress) = inferred
                && !*saw_explicit
            {
                handle.report_progress(progress);
            }
        }
    }
}

/// Map a process exit status onto a task state.
fn state_from_status(status: std::io::Result<std::process::ExitStatus>) -> TaskState {
    match status {
        Ok(status) => match status.code() {
            Some(code) => TaskState::Exited { code },
            None => TaskState::Signaled {
                signal: signal_name(&status),
            },
        },
        // The wait itself failed. Report a signal with no name, so the task is
        // still final and never silent.
        Err(_) => TaskState::Signaled { signal: None },
    }
}

/// The signal name, where the platform reports one.
#[cfg(unix)]
fn signal_name(status: &std::process::ExitStatus) -> Option<String> {
    use std::os::unix::process::ExitStatusExt;
    status.signal().map(|signal| signal.to_string())
}

/// A non-Unix host reports no signal.
#[cfg(not(unix))]
fn signal_name(_status: &std::process::ExitStatus) -> Option<String> {
    None
}

/// Map a task-registry error onto a tool error. The message keeps the advice.
fn map_task_error(error: TaskError) -> ToolError {
    ToolError::Io(error.to_string())
}

/// The message a background start returns to the model. It names the task id and
/// the reason, so the choice to background is never silent.
fn started_output(id: &rho_core::TaskId, reason: BackgroundReason, timeout_ms: u64) -> ToolOutput {
    ToolOutput::text(background_message(id, reason, timeout_ms))
}

/// The message for any background start.
///
/// A live run showed why the unit hint belongs here rather than on the adoption path
/// alone. A `sleep` command matches a long-running shape, so rho backgrounds it before
/// the timeout ever applies. The model then saw only "matches a long-running shape" and
/// never learned that its `timeout_ms` of 1000 meant one second.
///
/// So the hint follows the **value**, not the reason. Any background start warns when the
/// requested timeout looks like a seconds-for-milliseconds mistake, and stays quiet
/// otherwise, because a hint that fires every time is a hint nobody reads.
fn background_message(id: &rho_core::TaskId, reason: BackgroundReason, timeout_ms: u64) -> String {
    let mut message = format!(
        "Started background task {id}. Reason: {}. Probe it with the task tool.",
        reason_text(reason)
    );
    if timeout_ms <= SUSPICIOUS_TIMEOUT_MS {
        let seconds = timeout_ms as f64 / 1000.0;
        message.push_str(&format!(
            " Note the unit: timeout_ms is in milliseconds, so {timeout_ms} is \
             {seconds:.1} seconds. Pass a larger value if you meant longer."
        ));
    }
    message
}

/// A short reason phrase for the user.
fn reason_text(reason: BackgroundReason) -> &'static str {
    match reason {
        BackgroundReason::ModelRequested => "you asked for a background run",
        BackgroundReason::KnownLongRunning => "the command matches a long-running shape",
        BackgroundReason::LongTimeoutRequested => "the command asked for a long timeout",
        BackgroundReason::AdoptedOnTimeout => "the command outran its foreground timeout",
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
        if rho_redact::looks_like_a_secret(&text) {
            command.env_remove(&name);
        }
    }
}

/// Point the child's temporary directory at disk-backed storage.
///
/// Adopted from jcode, and it matters more here than there. On most Linux systems `/tmp`
/// is a tmpfs, so it lives in RAM. A build, a git worktree, or a virtual environment
/// placed there consumes the memory this project exists to save. One careless `cargo
/// build --target-dir /tmp/x` can cost more than a hundred sessions.
///
/// So the child gets `TMPDIR` pointing at `~/.rho/scratch`, and `RHO_SCRATCH_DIR` naming
/// the same place for a script that wants it explicitly.
///
/// A failure here is not fatal. If the directory cannot be created, the child keeps the
/// caller's `TMPDIR`, which is the behaviour before this change.
fn configure_scratch_dir(command: &mut tokio::process::Command) {
    if let Some(dir) = scratch_dir() {
        command.env("TMPDIR", &dir).env("RHO_SCRATCH_DIR", &dir);
    }
}

/// The scratch directory, created if needed.
///
/// `RHO_SCRATCH_DIR` in the parent environment wins, so a caller can place scratch space
/// on a chosen volume.
fn scratch_dir() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("RHO_SCRATCH_DIR")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
            let home = std::path::PathBuf::from(home);
            if home.as_os_str().is_empty() {
                return None;
            }
            Some(home.join(".rho").join("scratch"))
        })?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// A foreground timeout at or below this looks like a seconds-for-milliseconds mistake.
const SUSPICIOUS_TIMEOUT_MS: u64 = 5_000;

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

    #[test]
    fn background_message_names_the_unit_for_a_tiny_timeout() {
        let id = rho_core::TaskId("task-1".to_string());
        let text = background_message(&id, BackgroundReason::AdoptedOnTimeout, 1_000);
        assert!(text.contains("task-1"));
        assert!(text.to_lowercase().contains("millisecond"), "{text}");
        assert!(text.contains("1.0 seconds"), "{text}");
    }

    #[test]
    fn background_message_does_not_lecture_on_a_deliberate_timeout() {
        // A hint that fires every time is a hint nobody reads.
        let id = rho_core::TaskId("task-1".to_string());
        let text = background_message(&id, BackgroundReason::KnownLongRunning, 120_000);
        assert!(!text.to_lowercase().contains("millisecond"), "{text}");
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
    async fn a_line_longer_than_the_cap_is_split() {
        // The section 8 limit, named as in `SPEC-background-tasks`. A line longer than the cap
        // is split into pieces, and no piece passes the cap. This shares the
        // property that `reader_splits_a_line_that_never_ends` proves, under the
        // spec's own name.
        let size = 3 * MAX_LINE_BYTES + 11;
        let pieces = read_all(vec![b'Z'; size]).await;
        assert!(pieces.len() > 1, "an over-long line must be split");
        for piece in &pieces {
            assert!(piece.len() <= MAX_LINE_BYTES, "a piece passed the cap");
        }
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
