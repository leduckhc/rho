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

### How to read the code blocks in this spec

Every `rust` block states the public surface verbatim. Copy the signatures
exactly. Do not rename a type, a method, or a field.

A function shown without a body is a signature, not a compile error. Give it a
`todo!()` body when you first paste it. The S3 stage leaves the bodies as
`todo!()` on purpose, so the tests fail for the right reason. The S4 stage fills
them in.

This rule covers `Session::prompt`, `RetryPolicy::backoff`, `confine`, every
`PluginHost` method, `TuiState::apply`, `TuiState::submit_input`, `render`, and
every `App` method.

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
```

`ToolOutput` and `ToolKind` are defined in `SPEC-03`.

The `Session` owns the provider, the tool registry, the hook chain, and the
context, all behind an `Arc`. It also owns a `SessionConfig`. `Session::prompt`
starts a run. It spawns one task. The task drives the loop and sends events on a
channel. The returned `AgentEvents` wraps the receiver and the task handle.

`SessionConfig` carries the values a session needs before it can run: the model
id, the confinement root, the approval policy, and the per-run turn cap. It holds
no default for the session root. A caller states it. A tool cannot confine a path
against a root that nobody chose. See decision D-011. A caller that wants the
current directory calls `SessionConfig::for_current_dir`, so that choice is
visible in the calling code.

```rust
use std::path::PathBuf;

/// The configuration one `Session` needs before it can run.
#[derive(Clone)]
pub struct SessionConfig {
    /// The model id sent in every `CompletionRequest`.
    pub model: String,
    /// The path confinement root. It has no default.
    pub session_root: PathBuf,
    /// The approval policy. Tool dispatch consults it before it runs a tool.
    pub approval: Arc<dyn ApprovalPolicy>,
    /// The per-run turn cap. The loop stops with `MaxTurnRequests` at the cap.
    pub max_turns: u32,
}

impl SessionConfig {
    /// Build a config with an explicit model, root, and policy.
    pub fn new(
        model: impl Into<String>,
        session_root: impl Into<PathBuf>,
        approval: Arc<dyn ApprovalPolicy>,
    ) -> Self;
    /// Build a config that confines paths to the current directory.
    pub fn for_current_dir(
        model: impl Into<String>,
        approval: Arc<dyn ApprovalPolicy>,
    ) -> std::io::Result<Self>;
    /// Override the per-run turn cap.
    pub fn with_max_turns(self, max_turns: u32) -> Self;
}
```

`ApprovalPolicy` is defined in `SPEC-03`.

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
    config: SessionConfig,
}

impl Session {
    /// Build a session with an explicit `SessionConfig`. This is the primary
    /// constructor. See decision D-011.
    /// The only constructor. Decision D-013 deleted a four-argument `new`, because
    /// it silently supplied a fake model id, the current directory as the session
    /// root, and a policy that approved every tool call. A caller states all three.
    pub fn with_config(
        config: SessionConfig,
        provider: Arc<dyn Provider>,
        tools: Arc<ToolRegistry>,
        hooks: Arc<HookChain>,
        context: Context,
    ) -> Self;

    /// Start one agent run. Append `input` to the context, then drive the loop.
    /// `cancel` stops the run. Dropping the returned value also stops the run.
    pub fn prompt(&self, input: Vec<ContentBlock>, cancel: CancelToken) -> AgentEvents;

    /// Read the conversation so far. The context is append-only, so this grants
    /// no mutation. A lock guards the context, so this returns a cheap snapshot
    /// of the messages, not a borrow. See decision D-008.
    pub async fn messages(&self) -> Vec<Message>;
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
2. Consult the `ApprovalPolicy` from `SessionConfig`. It reads the tool `kind`,
   so the decision is typed, not a name match. A denial produces an error
   `ToolResult` and the tool never runs. See `SPEC-03`. In the ACP frontend the
   policy issues `session/request_permission`.
3. Emit `ToolStart`.
4. Look up the tool in the registry. Build a `ToolContext` with the
   `session_root` from `SessionConfig`. Run `execute`. Forward `ToolUpdate` for
   each streamed line.
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
  serialises to its ACP wire name, for example `max_turn_requests`.
- `agent_stop_reason_canceled_serialises_as_cancelled` — `Canceled` serialises to
  `"cancelled"` with two letters `l`, which is the ACP spelling. This test
  guards the `serde(rename)` attribute in section 9.
- `agent_events_drop_aborts_driver_task` — dropping `AgentEvents` before the run
  ends aborts the task; a `Drop` flag on the fake provider confirms the in-flight
  stream was dropped.
- `hook_block_produces_error_tool_result` — a hook that blocks a call yields a
  `ToolResult` with `is_error: true` and the tool never runs.
- `approval_denied_mutating_call_never_runs_the_tool` — a `ReadOnlyPolicy` in
  `SessionConfig` denies a mutating tool, the tool never runs, and the result is
  an error result. See `crates/rho-core/tests/approval.rs`.
- `approval_allowed_reading_call_runs_the_tool` — a `ReadOnlyPolicy` allows a
  reading tool and the tool runs. See `crates/rho-core/tests/approval.rs`.

## 12a. Credentials and retry, from decision D-014

`rho-core` owns two types that `SPEC-02` describes in detail.

```rust
/// A credential that never prints itself. No `Display`. `Debug` prints a mask.
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self;
    /// Read the value. Never log the result.
    pub fn expose(&self) -> &str;
    /// True when the credential is empty. An empty key fails with a message that
    /// blames the service, so check it early.
    pub fn is_empty(&self) -> bool;
}

/// How a provider retries. Retries `Transport`, `Server`, and `RateLimited`.
/// Never retries `Client`, `Decode`, or `Auth`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl RetryPolicy {
    /// A policy that never retries.
    pub fn none() -> Self;
    /// True when the error may be retried at `attempt`, which is one-based.
    pub fn should_retry(&self, error: &ProviderError, attempt: u32) -> bool;
    /// The delay before `attempt`. Exponential growth with full jitter. A server
    /// hint wins and skips the jitter, but the ceiling still caps it. `None` means
    /// do not retry.
    pub fn backoff(&self, attempt: u32, retry_after_ms: Option<u64>) -> Option<Duration>;
}
```

Both live in `rho-core` so there is one definition and one test suite. See
decision D-014 for the drift that forced the move.

### Test cases

- `secret_debug_prints_a_fixed_mask` — `Debug` prints `Secret(***)`.
- `secret_debug_never_contains_the_value` — the value never appears.
- `secret_inside_a_derived_debug_struct_stays_masked` — the real risk. A config
  struct derives `Debug`, somebody logs it, and the mask must survive that path.
- `secret_exposes_the_value_on_purpose` — `expose` returns the value.
- `secret_reports_an_empty_value` — `is_empty` is true for an empty credential.
- `retry_policy_never_retries_an_unauthorised_request` — a 401 is never retried.
- `retry_policy_never_retries_any_client_error` — 400, 401, 403, 404, and 422 are
  never retried.
- `retry_policy_never_retries_a_decode_or_auth_error` — both are permanent.
- `retry_policy_retries_a_rate_limit`, `..._a_server_error`, `..._a_transport_error`
  — each retryable class retries.
- `retry_policy_stops_at_the_attempt_cap` — the cap holds.
- `retry_policy_none_never_retries_a_retryable_error` — the no-retry policy holds.
- `backoff_returns_none_past_the_cap` — past the cap means do not retry.
- `backoff_stays_inside_the_window_and_grows` — full jitter picks a value in
  `[0, window)`. The test asserts the bound, never one fixed value, because a
  fixed assertion on a jittered value would be flaky by design.
- `backoff_never_passes_the_ceiling` — `max_delay_ms` holds at every attempt.
- `backoff_uses_a_server_hint_without_jitter` — a hint wins.
- `backoff_caps_a_hostile_server_hint` — a mistaken `Retry-After` cannot stall a
  session for an hour.
- `backoff_spreads_across_calls` — the jitter really spreads. A single value would
  rebuild the spike that the jitter exists to avoid.

### Coverage tests, in `crates/rho-core/tests/coverage.rs`

A reviewer listed every public item with no test. That list closed the gap that hid
the three worst defects in this crate. These tests cover it.

- `session_config_for_current_dir_uses_the_working_directory`
- `session_config_with_max_turns_overrides_the_default`
- `agent_loop_honours_a_max_turns_override` — the setter reaches the loop. The older
  cap test relied on the default of 32, so the setter itself was unproven.
- `context_tools_returns_the_registered_specs`
- `provider_error_server_is_retryable` — a 5xx retries. Only the other variants were
  covered.
- `provider_error_decode_and_auth_are_not_retryable`
- `agent_loop_maps_content_filtered_to_refusal`, `agent_loop_maps_max_tokens_through_the_loop`,
  `agent_loop_maps_stop_sequence_to_end_turn` — the mapping runs through the real
  loop, not only through serde.
- `agent_loop_reports_an_unregistered_tool_and_continues` — a model may invent a
  tool name. The run must continue and the model must read why.
- `agent_loop_forwards_streamed_tool_output_in_order`
- `dropping_events_drops_a_tool_future_in_flight` — see the note below.

**A note on the drop test, because the first version was worthless.** It dropped the
event stream and asserted the tool had not completed. But the tool was waiting on a
signal that never fired, so it could not complete either way. That test passed
against a deliberately emptied `Drop` impl. The current version releases the tool
after the drop, so a live future would wake and set its flag, while a dropped future
has no waiter. The controller confirmed it fails against the emptied `Drop`.

## 13. Out of scope for sprint 1

- Compaction and branch summarisation. The context grows without a cut point.
- Session persistence to disk. The context lives in memory only.
- Auto-continue on incomplete todos.
- Parallel tool execution. Tools run one at a time, in call order.
- Prompt caching hints in the request body. The design keeps the prefix stable,
  but no cache-control markers are emitted yet.
- Multi-model handoff inside one session.
