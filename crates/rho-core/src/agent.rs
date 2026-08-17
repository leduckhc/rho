//! The agent loop.
//!
//! The loop drives one full run. A run may span several provider turns because a
//! turn can call tools. The loop appends every message to the `Context`. It
//! emits an `AgentEvent` stream. A frontend renders the stream.

use crate::{
    Context, Error, HookChain, Provider, StopReason, StreamEvent, ToolKind, ToolOutput,
    ToolRegistry,
};
use crate::{CancelToken, ContentBlock};
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

/// Why a full agent run stopped. The wire names match the ACP `StopReason` set,
/// so `rho-acp` maps the values one-to-one onto a `session/prompt` response.
/// See SPEC-06.
///
/// One name needs an explicit rename. ACP spells the cancelled reason with two
/// letters `l`, as `cancelled`. Rust names the variant `Canceled` with one `l`,
/// which `rename_all = "snake_case"` would turn into `canceled`. That value is
/// not valid in ACP. The `serde(rename)` attribute below corrects it. Do not
/// remove the attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStopReason {
    /// The model finished and asked for no more tools.
    EndTurn,
    /// A turn hit the token limit.
    MaxTokens,
    /// The loop hit its per-run turn cap. See section 9.
    MaxTurnRequests,
    /// The model refused, or a content filter stopped the output.
    Refusal,
    /// The caller cancelled the run. The wire value is `cancelled`.
    #[serde(rename = "cancelled")]
    Canceled,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvent {
    /// One provider turn begins.
    TurnStart,
    /// A normalised provider event.
    Stream(StreamEvent),
    /// A tool begins execution, after hooks and the approval policy pass.
    ToolStart { id: String, name: String, kind: ToolKind },
    /// A streamed line of tool output.
    ToolUpdate { id: String, output: String },
    /// A tool finished. The output feeds the next turn.
    ToolEnd { id: String, output: ToolOutput },
    /// One provider turn ended.
    TurnEnd { stop_reason: StopReason },
    /// The run is fully settled. No further turn will run.
    AgentEnd { stop_reason: AgentStopReason },
}

/// Configuration for one agent run.
#[derive(Clone, Copy, Debug)]
pub struct AgentConfig {
    /// The per-run turn cap. The loop stops with `MaxTurnRequests` at the cap.
    pub max_turns: u32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self { max_turns: 32 }
    }
}

/// The event stream of one agent run.
///
/// Drop this value to cancel the run. `Drop` aborts the driver task, which drops
/// the provider stream and any running tool future. No task leaks.
pub struct AgentEvents {
    rx: tokio::sync::mpsc::Receiver<Result<AgentEvent, Error>>,
    handle: tokio::task::JoinHandle<()>,
}

impl Stream for AgentEvents {
    type Item = Result<AgentEvent, Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

impl Drop for AgentEvents {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

pub struct Session {
    // S4 reads `inner` when it fills in `prompt`. The body is `todo!()` now, so
    // the field is write-only in S3. S4 removes this allow.
    #[allow(dead_code)]
    inner: Arc<SessionInner>,
}

// S4 reads these fields when it fills in the agent loop. The loop body is
// `todo!()` now, so the fields are write-only in S3. S4 removes this allow.
#[allow(dead_code)]
struct SessionInner {
    provider: Arc<dyn Provider>,
    tools: Arc<ToolRegistry>,
    hooks: Arc<HookChain>,
    context: tokio::sync::Mutex<Context>,
}

impl Session {
    pub fn new(
        provider: Arc<dyn Provider>,
        tools: Arc<ToolRegistry>,
        hooks: Arc<HookChain>,
        context: Context,
    ) -> Self {
        Self {
            inner: Arc::new(SessionInner {
                provider,
                tools,
                hooks,
                context: tokio::sync::Mutex::new(context),
            }),
        }
    }

    /// Start one agent run. Append `input` to the context, then drive the loop.
    /// `cancel` stops the run. Dropping the returned value also stops the run.
    pub fn prompt(&self, _input: Vec<ContentBlock>, _cancel: CancelToken) -> AgentEvents {
        todo!()
    }
}
