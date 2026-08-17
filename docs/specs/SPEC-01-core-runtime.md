# SPEC-01 — Core runtime

Status: draft for sprint 1.
Owning crate: `rho-core`.
Consumers: `rho-tui`, `rho-acp`, `rho-cli`, every provider crate, `rho-tools`, `rho-plugin`.

`rho-core` is a pure library. It links no HTTP client and no terminal. It defines
the message model, the streaming event model, the provider and tool traits, the
hook chain, the cancellation type, and the agent loop. This spec is the keystone.
Every other spec builds on the types defined here.

Features covered: F-01 (agent loop), F-02 (event stream), F-03 (turn model),
F-04 (cancellation), F-60 (stable prefix), F-64 (short system prompt),
F-121 (library-first API). The event model is also constrained by the ACP
frontend; see `SPEC-06` and section 12 below.

Decision note (D-002): the event model must express every concept the Agent
Client Protocol reports back to a client. Section 12 states the alignment. The
agent-level stop reason in section 9 mirrors the ACP `StopReason` set exactly.

## 1. Design rules

- The conversation is append-only. A turn adds entries. A turn never edits an
  earlier entry. This keeps the provider prompt prefix stable.
- The core owns no network code. A `Provider` is injected as a trait object.
- Every public error type uses `thiserror`. Transport errors and permanent
  client errors are separate variants.
- Cancellation drops in-flight work. A cancelled turn leaks no task.

## 2. Content block model

A message holds an ordered list of content blocks. One enum covers every block
type. The `type` tag drives serialisation.

```rust
use serde::{Deserialize, Serialize};

/// A base64-encoded image and its MIME type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageSource {
    /// Base64 payload with no data-URI prefix.
    pub data: String,
    /// MIME type, for example `image/png`.
    pub mime_type: String,
}

/// One typed unit of message content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain assistant or user text.
    Text { text: String },
    /// Model reasoning. `signature` carries a provider replay token when present.
    Thinking {
        thinking: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    /// A model request to call a tool. `arguments` is the parsed JSON object.
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    /// The result of a tool call. `content` holds only `Text` or `Image` blocks.
    ToolResult {
        tool_call_id: String,
        content: Vec<ContentBlock>,
        #[serde(default)]
        is_error: bool,
    },
    /// An image, in a user message or a tool result.
    Image { source: ImageSource },
}
```

Serialisation rule: each block serialises to a JSON object with a `type` field in
`snake_case`. A `Text` block serialises to `{"type":"text","text":"..."}`. A
`Thinking` block omits `signature` when it is `None`. This shape is stable and is
the on-disk session format.

## 3. Messages and roles

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    Tool,
}

/// One conversation entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}
```

A `User` message holds `Text` or `Image` blocks. An `Assistant` message holds
`Text`, `Thinking`, or `ToolCall` blocks. A `Tool` message holds one
`ToolResult` block per tool call.

## 4. Usage and stop reasons

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

/// Why the model stopped a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The model finished its answer.
    EndTurn,
    /// The model asked to call one or more tools.
    ToolUse,
    /// The output hit the token limit.
    MaxTokens,
    /// The output hit a stop sequence.
    StopSequence,
    /// A content filter stopped the output.
    ContentFiltered,
    /// The turn was cancelled by the caller.
    Canceled,
}
```

## 5. The normalised streaming event

Every provider emits this one event type. Every frontend consumes it. See
`ADR-003` for why. The provider maps its own wire format onto these variants.

```rust
/// A normalised streaming event, provider-agnostic.
///
/// `index` groups events for one content block within the current message.
/// A provider emits a `*Start`, then zero or more `*Delta`, then a `*End` for
/// each block, in `index` order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamEvent {
    /// The assistant message begins.
    MessageStart { role: Role },
    TextStart { index: u32 },
    TextDelta { index: u32, delta: String },
    TextEnd { index: u32 },
    ThinkingStart { index: u32 },
    ThinkingDelta { index: u32, delta: String },
    ThinkingEnd { index: u32, signature: Option<String> },
    /// A tool call begins. The name is known at the start.
    ToolCallStart { index: u32, id: String, name: String },
    /// A raw JSON fragment of the tool-call arguments.
    ToolCallDelta { index: u32, delta: String },
    /// The tool call is complete. `arguments` is the parsed JSON object.
    ToolCallEnd { index: u32, arguments: serde_json::Value },
    /// Cumulative token usage, reported one or more times.
    Usage(Usage),
    /// The turn is done. This is the last event of a successful turn.
    Done { stop_reason: StopReason },
}
```

Rules the provider crate must follow:
- The first event of a turn is `MessageStart`.
- Tool-call argument fragments arrive as `ToolCallDelta`. The provider buffers
  them and emits the parsed object in `ToolCallEnd`.
- The last event of a successful turn is `Done`.
- A stream error is not an event. The provider yields `Err(ProviderError)` on the
  stream and stops. See section 8.

## 6. Cancellation

`CancelToken` is a lightweight token. It uses an atomic flag and a `Notify`. It
adds no dependency. A clone shares the same state.

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

/// A shared, cloneable cancellation signal.
#[derive(Clone, Default)]
pub struct CancelToken {
    inner: Arc<CancelInner>,
}

#[derive(Default)]
struct CancelInner {
    flag: AtomicBool,
    notify: Notify,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation. Idempotent.
    pub fn cancel(&self) {
        self.inner.flag.store(true, Ordering::SeqCst);
        self.inner.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.flag.load(Ordering::SeqCst)
    }

    /// Resolve when the token is cancelled. Resolve at once if already cancelled.
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        self.inner.notify.notified().await;
    }
}
```

Cancellation contract:
- A `Provider::stream` future must select against `cancel.cancelled()`. It must
  stop the HTTP request when the token fires.
- The agent loop selects against the token between and during turns.
- Dropping the returned event stream also cancels the run. See section 9.

## 7. Append-only context discipline

This is a requirement, not a nicety. It keeps the provider KV cache warm.

The prompt prefix is the ordered sequence of: the system prompt, the tool list,
then the conversation messages. The provider sends this prefix on every turn. A
warm cache needs a byte-stable prefix.

Forbidden from the prefix:
- The current wall-clock time or any per-turn timestamp.
- A random request id, a nonce, or a trace id inside the prompt body.
- A token count, a cost figure, or any counter that changes each turn.
- Tool definitions that appear or reorder after turn one. The full tool list is
  sent in the first request and never changes shape mid-session. Plugin tools are
  advertised from an on-disk cache so a late plugin connection does not change the
  prefix. See `SPEC-04`.
- Any hook-injected text placed before existing messages. A hook may append a new
  message. A hook must not edit or reorder an earlier message.

The `Context` is append-only by construction. It exposes append and read. It
exposes no edit and no remove.

```rust
/// The append-only conversation log for one session.
#[derive(Clone, Debug, Default)]
pub struct Context {
    system: Option<String>,
    tools: Vec<ToolSpec>,
    messages: Vec<Message>,
}

impl Context {
    pub fn new(system: Option<String>, tools: Vec<ToolSpec>) -> Self {
        Self { system, tools, messages: Vec::new() }
    }

    /// Append one message. This is the only way to add to the log.
    pub fn append(&mut self, message: Message) {
        self.messages.push(message);
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn system(&self) -> Option<&str> {
        self.system.as_deref()
    }

    pub fn tools(&self) -> &[ToolSpec] {
        &self.tools
    }
}
```

## 8. Error taxonomy

Two error types. `ProviderError` is raised by a provider crate. `Error` is the
top-level agent error. Retryable transport errors are separate from permanent
client errors. See `SPEC-02` for the retry policy that reads `is_retryable`.

```rust
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// A network fault. Retryable.
    #[error("transport error: {0}")]
    Transport(String),
    /// HTTP 429. Retryable. `retry_after_ms` mirrors the `Retry-After` header.
    #[error("rate limited")]
    RateLimited { retry_after_ms: Option<u64> },
    /// HTTP 5xx. Retryable.
    #[error("server error: status {status}")]
    Server { status: u16 },
    /// HTTP 4xx other than 429. Permanent. Never retried.
    #[error("client error: status {status}: {message}")]
    Client { status: u16, message: String },
    /// The response body could not be decoded. Permanent.
    #[error("stream decode error: {0}")]
    Decode(String),
    /// Credential resolution or signing failed. Permanent.
    #[error("authentication failed: {0}")]
    Auth(String),
    /// The caller cancelled the request.
    #[error("canceled")]
    Canceled,
}

impl ProviderError {
    /// True for faults a retry may fix. False for permanent client faults.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ProviderError::Transport(_)
                | ProviderError::RateLimited { .. }
                | ProviderError::Server { .. }
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error("canceled")]
    Canceled,
}
```

`ToolError` is defined in `SPEC-03`.

## 9. The agent loop

The loop drives one full run. A run may span several provider turns because a
turn can call tools. The loop appends every message to the `Context`. It emits an
`AgentEvent` stream. A frontend renders the stream.

```rust
/// Why a full agent run stopped. This mirrors the ACP `StopReason` set exactly,
/// so `rho-acp` maps it one-to-one onto a `session/prompt` response. See SPEC-06.
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
    /// The caller cancelled the run.
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
```

`ToolOutput` and `ToolKind` are defined in `SPEC-03`.

The `Session` owns the provider, the tool registry, the hook chain, and the
context, all behind an `Arc`. `Session::prompt` starts a run. It spawns one task.
The task drives the loop and sends events on a channel. The returned `AgentEvents`
wraps the receiver and the task handle.

```rust
use futures::Stream;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

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
    inner: Arc<SessionInner>,
}

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
    pub fn prompt(&self, input: Vec<ContentBlock>, cancel: CancelToken) -> AgentEvents;
}
```

`ToolRegistry` is defined in `SPEC-03`. `HookChain` and `Hook` are defined in
`SPEC-04`. `Provider`, `ProviderStream`, `CompletionRequest`, and `ToolSpec` are
defined in `SPEC-02`.

Turn state machine, one turn:
1. Emit `TurnStart`.
2. Build a `CompletionRequest` from the context.
3. Call `provider.stream(request, cancel)`.
4. Forward each `StreamEvent` as `AgentEvent::Stream`.
5. Collect the assistant message from the events. Append it to the context.
6. On `Done { stop_reason: ToolUse }`: run each tool call, then loop to step 1.
7. On any other `Done`: emit `TurnEnd`, map the provider `StopReason` to an
   `AgentStopReason`, emit `AgentEnd`, then stop.

Provider-to-agent stop reason mapping at step 7:
- `EndTurn` and `StopSequence` map to `AgentStopReason::EndTurn`.
- `MaxTokens` maps to `AgentStopReason::MaxTokens`.
- `ContentFiltered` maps to `AgentStopReason::Refusal`.
- `Canceled` maps to `AgentStopReason::Canceled`.

Turn cap: the loop runs at most `AgentConfig::max_turns` provider turns per run.
The default is 32. When the loop hits the cap it stops with
`AgentEnd { stop_reason: MaxTurnRequests }`. This bounds a tool-call loop and
maps to the ACP `max_turn_requests` stop reason.

Tool dispatch, for one tool call:
1. Run the hook chain `before_tool_call` in registration order. The first
   `Block` stops the call. A blocked call produces a `ToolResult` with
   `is_error: true`.
2. Consult the `ApprovalPolicy`. A denial produces an error `ToolResult`. See
   `SPEC-03`. In the ACP frontend the policy issues `session/request_permission`.
3. Emit `ToolStart`.
4. Look up the tool in the registry. Run `execute`. Forward `ToolUpdate` for each
   streamed line.
5. Run the hook chain `after_tool_result` in registration order.
6. Emit `ToolEnd`. Append a `Tool` message with the `ToolResult` block.

Cancellation during a turn:
- The loop selects the event stream against `cancel.cancelled()`.
- When the token fires, the loop stops reading, drops the provider stream, and
  aborts any running tool future.
- The loop emits `TurnEnd { stop_reason: Canceled }`, then
  `AgentEnd { stop_reason: Canceled }`.

## 10. ACP alignment

The event model must carry every concept the Agent Client Protocol reports. This
table states the mapping. `SPEC-06` gives the full detail. Where a concept has no
core carrier, the row names the gap.

| ACP concept | Core carrier |
| --- | --- |
| `agent_message_chunk` | `StreamEvent::Text*` |
| `agent_thought_chunk` | `StreamEvent::Thinking*` |
| `tool_call` (pending) | `StreamEvent::ToolCallEnd` plus the tool `kind` |
| `tool_call_update` in_progress | `AgentEvent::ToolStart` |
| `tool_call_update` completed or failed | `AgentEvent::ToolEnd` with `is_error` |
| tool `kind` | `ToolKind` on the tool and the `ToolSpec` (`SPEC-03`) |
| `session/request_permission` | `ApprovalPolicy` (`SPEC-03`), which is async |
| `session/prompt` stop reason | `AgentStopReason` on `AgentEnd`, mapped one-to-one |
| `UsageUpdate` tokens | `StreamEvent::Usage` token counts |

Gaps recorded for sprint 1, handled in `rho-acp`, not in the core:
- `tool_call.title` is synthesised by `rho-acp` from the tool name and arguments.
- `tool_call.locations` for follow-along is not tracked. `rho-acp` omits it.
- A structured `diff` tool-call content is not carried. `rho-acp` reports an
  `edit` result as a text content block. Structured diff is planned.
- `plan` updates need a todo or plan feature (F-30, planned). Sprint 1 sends no
  plan updates.
- `UsageUpdate.size` and `cost` are not in the core `Usage`. `rho-acp` fills
  `size` from the model context window and omits `cost` in sprint 1.

## 11. Session format (planned, F-50)

Decision D-001: rho defines its own append-only JSONL session format. It is not
pi-compatible. The first record is a header with a `version` field. A later
crate, `rho-session-import-pi`, provides a one-way import from pi. Persistence is
out of scope for sprint 1. The append-only `Context` in section 7 is the
in-memory shape that the format will serialise.

## 12. Test cases

- `content_block_text_roundtrips_json` — a `Text` block serialises to
  `{"type":"text","text":...}` and parses back to an equal value.
- `content_block_thinking_omits_absent_signature` — a `Thinking` block with no
  signature has no `signature` key in its JSON.
- `content_block_tool_call_roundtrips_json` — a `ToolCall` block round-trips with
  its `id`, `name`, and `arguments`.
- `content_block_tool_result_roundtrips_json` — a `ToolResult` block round-trips
  with nested `Text` content and its `is_error` flag.
- `stream_event_tags_are_snake_case` — every `StreamEvent` variant serialises
  with a `snake_case` `kind` tag.
- `stop_reason_serialises_snake_case` — each `StopReason` value serialises to its
  `snake_case` name.
- `cancel_token_starts_uncancelled` — a fresh token reports `is_cancelled` false.
- `cancel_token_cancel_sets_flag` — after `cancel`, `is_cancelled` is true.
- `cancel_token_cancelled_resolves_after_cancel` — `cancelled().await` resolves
  once `cancel` is called from another task.
- `cancel_token_cancelled_returns_immediately_when_already_cancelled` —
  `cancelled().await` resolves at once for an already-cancelled token.
- `context_append_only_preserves_order` — appended messages read back in order.
- `provider_error_transport_is_retryable` — `Transport` reports retryable true.
- `provider_error_client_is_not_retryable` — `Client` reports retryable false.
- `provider_error_rate_limited_is_retryable` — `RateLimited` reports true.
- `agent_loop_emits_turn_start_then_stream_then_turn_end` — a scripted fake
  provider with no tool call yields `TurnStart`, stream events, `TurnEnd`,
  `AgentEnd` in order.
- `agent_loop_runs_tool_then_continues` — a scripted provider that stops with
  `ToolUse` triggers `ToolStart`, `ToolEnd`, then a second `TurnStart`.
- `agent_loop_appends_assistant_and_tool_messages` — after a tool turn the
  context holds the assistant message and the tool-result message in order.
- `agent_loop_end_turn_maps_to_agent_stop_reason_end_turn` — a provider
  `EndTurn` yields `AgentEnd { stop_reason: EndTurn }`.
- `agent_loop_turn_cap_stops_with_max_turn_requests` — a provider that always
  asks for a tool stops at `max_turns` with `AgentStopReason::MaxTurnRequests`.
- `agent_loop_cancel_ends_with_canceled_stop_reason` — cancelling mid-turn yields
  `TurnEnd { stop_reason: Canceled }` then `AgentEnd { stop_reason: Canceled }`.
- `agent_stop_reason_serialises_snake_case` — each `AgentStopReason` value
  serialises to its ACP `snake_case` name, for example `max_turn_requests`.
- `agent_events_drop_aborts_driver_task` — dropping `AgentEvents` before the run
  ends aborts the task; a `Drop` flag on the fake provider confirms the in-flight
  stream was dropped.
- `hook_block_produces_error_tool_result` — a hook that blocks a call yields a
  `ToolResult` with `is_error: true` and the tool never runs.

## 13. Out of scope for sprint 1

- Compaction and branch summarisation. The context grows without a cut point.
- Session persistence to disk. The context lives in memory only.
- Auto-continue on incomplete todos.
- Parallel tool execution. Tools run one at a time, in call order.
- Prompt caching hints in the request body. The design keeps the prefix stable,
  but no cache-control markers are emitted yet.
- Multi-model handoff inside one session.
