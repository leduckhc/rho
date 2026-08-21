//! Shared test support for the `rho-core` agent-loop tests.
//!
//! It provides one scripted fake `Provider`, one recording fake `Tool`, and a
//! few fake `Hook` types. Every agent-loop test uses the same fakes. A scripted
//! fake is better than many ad-hoc fakes.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use futures::stream;
use rho_core::{
    CancelToken, CompletionRequest, ContentBlock, Hook, HookOutcome, Provider, ProviderError,
    ProviderStream, StreamEvent, Tool, ToolCallView, ToolContext, ToolError, ToolKind, ToolOutput,
};
use tokio::sync::Notify;

/// A provider that replays a scripted list of turns.
///
/// Each call to `stream` pops the next turn and yields its events. When the
/// `cycle` flag is set the provider replays the last turn for every later call.
/// This drives a turn-cap test without a long script.
pub struct ScriptedProvider {
    turns: Mutex<VecDeque<Vec<StreamEvent>>>,
    cycle: Option<Vec<StreamEvent>>,
    /// Set to true when a returned stream drops. The drop test reads this.
    pub stream_dropped: Arc<AtomicBool>,
    /// Notified when a returned stream drops. The drop test awaits this. It uses
    /// `notify_one`, so it stores a permit and never races the awaiter.
    pub dropped: Arc<Notify>,
}

impl ScriptedProvider {
    /// Build a provider from a list of scripted turns.
    pub fn new(turns: Vec<Vec<StreamEvent>>) -> Self {
        Self {
            turns: Mutex::new(turns.into()),
            cycle: None,
            stream_dropped: Arc::new(AtomicBool::new(false)),
            dropped: Arc::new(Notify::new()),
        }
    }

    /// Build a provider that replays `turn` for every call. This never ends, so
    /// the loop must stop it with the turn cap.
    pub fn cycling(turn: Vec<StreamEvent>) -> Self {
        Self {
            turns: Mutex::new(VecDeque::new()),
            cycle: Some(turn),
            stream_dropped: Arc::new(AtomicBool::new(false)),
            dropped: Arc::new(Notify::new()),
        }
    }
}

/// A guard that sets a flag when the provider stream drops.
struct DropGuard {
    flag: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl Drop for DropGuard {
    fn drop(&mut self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_one();
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn id(&self) -> &str {
        "scripted"
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
        _cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError> {
        let events = {
            let mut turns = self.turns.lock().unwrap();
            turns
                .pop_front()
                .or_else(|| self.cycle.clone())
                .unwrap_or_default()
        };
        let guard = DropGuard {
            flag: Arc::clone(&self.stream_dropped),
            notify: Arc::clone(&self.dropped),
        };
        // The guard moves into the stream state. The flag flips when the stream
        // drops, which proves the driver task dropped the in-flight stream.
        let s = stream::unfold((events.into_iter(), guard), |(mut it, guard)| async move {
            it.next().map(|ev| (Ok(ev), (it, guard)))
        });
        Ok(Box::pin(s))
    }
}

/// A tool that records whether it ran. The hook-block test asserts it stayed
/// false. The tool returns a fixed text result.
pub struct RecordingTool {
    name: String,
    kind: ToolKind,
    /// Set to true the moment `execute` runs.
    pub ran: Arc<AtomicBool>,
    /// The last arguments `execute` saw. An arg-edit hook test reads this.
    pub last_args: Arc<Mutex<Option<serde_json::Value>>>,
}

impl RecordingTool {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: ToolKind::Other,
            ran: Arc::new(AtomicBool::new(false)),
            last_args: Arc::new(Mutex::new(None)),
        }
    }
}

#[async_trait]
impl Tool for RecordingTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "a recording test tool"
    }
    fn kind(&self) -> ToolKind {
        self.kind
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        self.ran.store(true, Ordering::SeqCst);
        *self.last_args.lock().unwrap() = Some(args);
        Ok(ToolOutput::text("tool ran"))
    }
}

/// A hook that records its name in a shared log when a point fires.
pub struct OrderRecordingHook {
    name: String,
    pub log: Arc<Mutex<Vec<String>>>,
}

impl OrderRecordingHook {
    pub fn new(name: impl Into<String>, log: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            name: name.into(),
            log,
        }
    }
}

#[async_trait]
impl Hook for OrderRecordingHook {
    fn name(&self) -> &str {
        &self.name
    }
    async fn before_tool_call(&self, _call: &mut ToolCallView<'_>) -> HookOutcome {
        self.log.lock().unwrap().push(self.name.clone());
        HookOutcome::Continue
    }
}

/// A hook that always blocks a call with a fixed reason.
pub struct BlockingHook {
    name: String,
    pub log: Arc<Mutex<Vec<String>>>,
}

impl BlockingHook {
    pub fn new(name: impl Into<String>, log: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            name: name.into(),
            log,
        }
    }
}

#[async_trait]
impl Hook for BlockingHook {
    fn name(&self) -> &str {
        &self.name
    }
    async fn before_tool_call(&self, _call: &mut ToolCallView<'_>) -> HookOutcome {
        self.log.lock().unwrap().push(self.name.clone());
        HookOutcome::Block {
            reason: "blocked by test hook".to_string(),
        }
    }
}

/// A hook that edits a tool argument in place before the tool runs.
pub struct ArgEditHook {
    name: String,
    pub key: String,
    pub value: serde_json::Value,
}

impl ArgEditHook {
    pub fn new(name: impl Into<String>, key: impl Into<String>, value: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            key: key.into(),
            value,
        }
    }
}

#[async_trait]
impl Hook for ArgEditHook {
    fn name(&self) -> &str {
        &self.name
    }
    async fn before_tool_call(&self, call: &mut ToolCallView<'_>) -> HookOutcome {
        if let Some(map) = call.arguments.as_object_mut() {
            map.insert(self.key.clone(), self.value.clone());
        }
        HookOutcome::Continue
    }
}

/// A hook that appends a marker to the tool output text.
pub struct OutputEditHook {
    name: String,
    pub marker: String,
}

impl OutputEditHook {
    pub fn new(name: impl Into<String>, marker: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            marker: marker.into(),
        }
    }
}

#[async_trait]
impl Hook for OutputEditHook {
    fn name(&self) -> &str {
        &self.name
    }
    async fn after_tool_result(&self, _name: &str, output: &mut ToolOutput) {
        output.content.push(ContentBlock::Text {
            text: self.marker.clone(),
        });
    }
}

/// A scripted turn that ends with `EndTurn` after one text block.
pub fn text_turn(text: &str) -> Vec<StreamEvent> {
    use rho_core::{Role, StopReason};
    vec![
        StreamEvent::MessageStart {
            role: Role::Assistant,
        },
        StreamEvent::TextStart { index: 0 },
        StreamEvent::TextDelta {
            index: 0,
            delta: text.to_string(),
        },
        StreamEvent::TextEnd { index: 0 },
        StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        },
    ]
}

/// A scripted turn that asks to call `tool_name`, ending with `ToolUse`.
pub fn tool_call_turn(id: &str, tool_name: &str, arguments: serde_json::Value) -> Vec<StreamEvent> {
    use rho_core::{Role, StopReason};
    vec![
        StreamEvent::MessageStart {
            role: Role::Assistant,
        },
        StreamEvent::ToolCallStart {
            index: 0,
            id: id.to_string(),
            name: tool_name.to_string(),
        },
        StreamEvent::ToolCallEnd {
            index: 0,
            arguments,
        },
        StreamEvent::Done {
            stop_reason: StopReason::ToolUse,
        },
    ]
}

/// A session config for tests. It states every permissive choice out loud.
///
/// Production code must not copy this. It approves every tool call, and it roots
/// path confinement at the current directory. `rho-core` ships no such default,
/// because a permissive default in a constructor becomes a security accident.
pub fn test_config() -> rho_core::SessionConfig {
    rho_core::SessionConfig::new(
        "test-model",
        std::env::current_dir().expect("a test needs a working directory"),
        std::sync::Arc::new(rho_core::AllowAllPolicy),
    )
}

/// An approval policy that approves every tool call.
pub fn allow_all() -> impl rho_core::ApprovalPolicy {
    rho_core::AllowAllPolicy
}

/// the model as an error result, rather than killing the run.
pub struct FailingTool {
    name: String,
    error_text: String,
}

impl FailingTool {
    pub fn new(name: impl Into<String>, error_text: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            error_text: error_text.into(),
        }
    }
}

#[async_trait]
impl Tool for FailingTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "A tool that always fails."
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        Err(ToolError::Io(self.error_text.clone()))
    }
}

/// A tool that streams output lines before it returns. Used to cover the
/// `AgentEvent::ToolUpdate` branch of tool dispatch.
pub struct UpdatingTool {
    lines: Vec<String>,
}

impl UpdatingTool {
    pub fn new(lines: &[&str]) -> Self {
        Self {
            lines: lines.iter().map(|line| line.to_string()).collect(),
        }
    }
}

#[async_trait]
impl Tool for UpdatingTool {
    fn name(&self) -> &str {
        "streamer"
    }
    fn description(&self) -> &str {
        "A tool that streams lines."
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        for line in &self.lines {
            // A full channel must not lose a line, so await rather than try_send.
            let _ = ctx.updates.send(line.clone()).await;
        }
        Ok(ToolOutput {
            content: vec![ContentBlock::Text {
                text: "streamed".to_string(),
            }],
            is_error: false,
        })
    }
}

/// A tool that blocks until it is told to finish, and records whether its future
/// was dropped before completing. Used to prove that dropping `AgentEvents` drops
/// a tool future in flight.
pub struct BlockingTool {
    /// Set to true only if `execute` ran to completion.
    pub completed: Arc<AtomicBool>,
    /// Set to true when `execute` starts, so a test can wait without sleeping.
    pub started: Arc<tokio::sync::Notify>,
    /// Released to let `execute` finish.
    pub release: Arc<tokio::sync::Notify>,
}

impl BlockingTool {
    pub fn new() -> Self {
        Self {
            completed: Arc::new(AtomicBool::new(false)),
            started: Arc::new(tokio::sync::Notify::new()),
            release: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

#[async_trait]
impl Tool for BlockingTool {
    fn name(&self) -> &str {
        "blocker"
    }
    fn description(&self) -> &str {
        "A tool that waits."
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        self.started.notify_waiters();
        self.release.notified().await;
        // Reached only if the future was not dropped first.
        self.completed.store(true, Ordering::SeqCst);
        Ok(ToolOutput {
            content: vec![ContentBlock::Text {
                text: "finished".to_string(),
            }],
            is_error: false,
        })
    }
}

/// A scripted turn that ends with an arbitrary stop reason and no text.
///
/// Used to drive `map_stop_reason` through the real loop, rather than testing only
/// its serde form.
pub fn turn_ending_with(stop_reason: rho_core::StopReason) -> Vec<StreamEvent> {
    use rho_core::Role;
    vec![
        StreamEvent::MessageStart {
            role: Role::Assistant,
        },
        StreamEvent::Done { stop_reason },
    ]
}

// ---------------------------------------------------------------------------
// Tracing capture, so a test can assert a warning was emitted rather than
// silently degraded. See decisions D-write-failure-degrades and D-bash-line-cap.
// ---------------------------------------------------------------------------

/// A `tracing` writer that collects every emitted byte into a shared buffer.
#[derive(Clone)]
pub struct LogCapture(pub Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogCapture {
    type Writer = LogCapture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Run `f` under a subscriber that captures `WARN`-and-above output, and return the
/// captured text. A silent degrade leaves this empty, so a test can fail on it.
///
/// **Why this installs a global subscriber.** A thread-local subscriber alone is not
/// enough. With no global subscriber, `tracing` reports the current level filter as
/// `OFF`, so a `warn!` takes its fast path and never reaches a thread-local capture. The
/// result is a test that passes or fails by which test ran first. This capture was flaky
/// one run in twenty for exactly that reason.
///
/// So a global subscriber is installed once per test binary, and it writes into a
/// thread-local buffer. The global sets the level filter, and the thread-local buffer
/// keeps parallel tests apart.
pub fn capture_warnings(f: impl FnOnce()) -> String {
    install_capture();
    CAPTURE.with(|cell| cell.lock().unwrap().clear());
    f();
    let text = CAPTURE.with(|cell| String::from_utf8(cell.lock().unwrap().clone()).expect("utf8"));
    // A capture that captures nothing would make every log assertion vacuous, so prove
    // the pipe works before a caller trusts an empty result.
    tracing::warn!("capture-probe");
    let probe = CAPTURE.with(|cell| String::from_utf8(cell.lock().unwrap().clone()).expect("utf8"));
    assert!(
        probe.contains("capture-probe"),
        "the log capture is broken, so no log assertion in this test means anything"
    );
    CAPTURE.with(|cell| cell.lock().unwrap().clear());
    text
}

thread_local! {
    /// The captured log bytes for this thread. A global subscriber writes here.
    static CAPTURE: Mutex<Vec<u8>> = const { Mutex::new(Vec::new()) };
}

/// A writer that appends to the calling thread's capture buffer.
#[derive(Clone, Default)]
pub struct ThreadCapture;

impl std::io::Write for ThreadCapture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        CAPTURE.with(|cell| cell.lock().unwrap().extend_from_slice(buf));
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for ThreadCapture {
    type Writer = ThreadCapture;
    fn make_writer(&'a self) -> Self::Writer {
        ThreadCapture
    }
}

/// Install the global capture subscriber once per test binary.
fn install_capture() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_writer(ThreadCapture)
            .with_max_level(tracing::Level::WARN)
            .without_time()
            .finish();
        // A second install would fail, and the `OnceLock` makes that impossible.
        tracing::subscriber::set_global_default(subscriber)
            .expect("the capture subscriber installs once per test binary");
    });
}

/// A tool that counts how many times it ran.
///
/// A budget test needs this. Asserting only the run's outcome cannot show an
/// overrun, and a review proved that by mutating the cap check from `>=` to `>`
/// while the test stayed green.
pub struct CountingTool {
    name: String,
    calls: Arc<std::sync::atomic::AtomicU32>,
}

impl CountingTool {
    pub fn new(name: impl Into<String>, calls: Arc<std::sync::atomic::AtomicU32>) -> Self {
        Self {
            name: name.into(),
            calls,
        }
    }
}

#[async_trait]
impl rho_core::Tool for CountingTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "counts its own calls"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: rho_core::ToolContext,
    ) -> Result<rho_core::ToolOutput, rho_core::ToolError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(rho_core::ToolOutput::text("counted"))
    }
}

/// A tool that never returns until its run is cancelled.
///
/// A timeout test needs it: the child must still be working when the deadline
/// fires, so the transcript has lines written and not yet flushed.
pub struct HangingTool {
    name: String,
}

impl HangingTool {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl rho_core::Tool for HangingTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "never finishes"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
        ctx: rho_core::ToolContext,
    ) -> Result<rho_core::ToolOutput, rho_core::ToolError> {
        ctx.cancel.cancelled().await;
        Ok(rho_core::ToolOutput::text("cancelled"))
    }
}
