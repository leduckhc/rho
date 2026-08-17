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
