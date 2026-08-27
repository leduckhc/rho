# SPEC-jsonl-frontend — JSONL headless frontend

Status: delivered. Owning crate: `rho-jsonl`, with the host seam in `rho-cli`. Depends on `rho-core` only.
Links no provider crate. `rho-core` keeps no terminal and no HTTP dependency.

Features covered: F-jsonl-frontend, F-jsonl-prompt, F-jsonl-steer,
F-jsonl-abort, F-jsonl-session-commands, F-jsonl-dialog-sub-protocol.

## 0. What changed from the draft

This spec was a draft, written before the crate existed. Executing it found nine
places where the contract disagreed with `rho-core`. Each one is now a decision
file, and the contract below is the corrected one.

| What the draft said | What the code proved | Decision |
|---|---|---|
| `AgentStopReasonWire` with five variants | `AgentStopReason` has six. `ToolKindWire` had four names, and `ToolKind` has ten, none of them matching | `D-the-wire-reuses-the-core-stop-reason` |
| `RunEnd` with a `will_retry` field | Retry happens inside a provider, below this layer, so `will_retry` can never be true | `D-settled-is-the-only-end-signal` |
| The agent is done only after `Settled` | A provider failure returns early and emits no `AgentEnd` at all | `D-the-frontend-settles-every-prompt` |
| `ReplyError::NotStreaming` for an early steer | `Session::steer` keeps an early message and delivers it next run | `D-a-steer-is-never-rejected-for-being-early` |
| `DialogValue` as an untagged enum | An untagged reader takes the first match, so two answers in one object read as one | `D-a-dialog-answer-holds-exactly-one-value` |
| A timeout "resolves with a default" | The default was never named, and three of the four candidates fail open | `D-a-dialog-timeout-cancels` |
| Framing said how to split a line | It never bounded one. This is the third unbounded reader in the project | `D-a-command-line-is-capped` |
| Clients ignore unknown fields | True for an event. Wrong for a command, where it means half-obeying | `D-a-command-is-strict-and-an-event-is-loose` |
| No answer for who builds a session | `set_model` needs a provider, and this crate must not link one | `D-rho-jsonl-asks-a-factory-for-a-session` |
| Route a line on the `success` field | One event carried a `success` field of its own, so a client read it as a reply | `D-no-event-carries-the-success-key` |
| Settle when the event stream ends | A failed run's stream never ends, because rho-core leaks the task that holds its sender | `D-an-error-on-the-event-stream-ends-the-run` |

Two events the draft omitted are now in the contract. `rho_core::AgentEvent` has
`MessageQueued` and `MessageDelivered`. Without them a client cannot see a steered
message arrive, so `F-jsonl-steer` had no observable outcome.

## 1. Why not ACP first

ACP stays the interop target. Decision D-acp-is-real-acp still holds. Only the
order changes.

`rho-acp` currently has zero lines of implementation. The ACP surface is large.
ACP uses JSON-RPC 2.0, with `jsonrpc`, `method`, and `params` fields. pi's
headless protocol is smaller. It has 35 command variants. It has no `jsonrpc`
field, no `method`, and no `params`. The wire is plain tagged JSON on stdin and
stdout.

A bridge maps ACP onto a smaller protocol. The owner's app `makit` already
does this. See `makit/server/src/adapters/acp.ts` line 11. That file confirms
pi runs through a bridge. rho can do the same. `SPEC-acp` stays valid as a
later bridge target.

`rho-jsonl` is not a private dialect. It is a public, documented contract. A
third party can implement a client without forking.

## 2. What this protocol is not

This protocol is not JSON-RPC. It has no `jsonrpc` field. Do not call it RPC.
pi calls its version "RPC mode." The wire has no `jsonrpc` field. Decision
D-acp-is-real-acp exists to keep protocol names honest. rho must not repeat
that confusion.

## 3. The contract

### 3.0 The sides

| Side | Owner | What it must keep |
|---|---|---|
| The client | any process, any language | Write one command per line. Ignore an unknown event type. |
| The frontend | `rho-jsonl` | Reply to every command once. Settle every accepted prompt once. |
| The runtime | `rho-core` | The event stream and the steering queue. |
| The host | `rho-cli`, or an embedder | Implement `SessionFactory`. Own credentials and tools. |

Contract kinds this change touches: the public API, the data model, the error
taxonomy, the wire format, the extension surface, and the behaviour rules for
ordering, cancellation, and timeouts. It touches no persisted format and no
config key.

### 3.1 Command

A command is a JSON object on stdin, one per line. `req_id` is optional. Use
it to correlate a reply to the command that sent it. An absent `req_id` key
reads as `None`.

`deny_unknown_fields` is deliberate. See `D-a-command-is-strict-and-an-event-is-loose`.

```rust
use serde::{Deserialize, Serialize};

/// A command sent to rho-jsonl on stdin, one per line.
///
/// It denies an unknown field on purpose. A command is an instruction, and an
/// unknown field may be the part that limits it. So rho refuses the whole line
/// rather than obey half of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    /// Start a run. It fails when a run is already going.
    Prompt {
        req_id: Option<String>,
        message: String,
    },
    /// Queue a message for the next turn boundary. It never interrupts a request.
    Steer {
        req_id: Option<String>,
        message: String,
    },
    /// Cancel the current run. It is not an error when no run is going.
    Abort {
        req_id: Option<String>,
    },
    /// Report the provider, the model, and whether a run is going.
    GetState {
        req_id: Option<String>,
    },
    /// Replace the session with one on a new provider and model.
    SetModel {
        req_id: Option<String>,
        provider: String,
        model_id: String,
    },
    /// Replace the session with an empty one on the same provider and model.
    NewSession {
        req_id: Option<String>,
    },
    /// Report the conversation so far.
    GetMessages {
        req_id: Option<String>,
    },
    /// Report the command names this build accepts.
    GetCommands {
        req_id: Option<String>,
    },
    /// Answer a dialog. `id` matches the DialogRequest, and it is not a req_id.
    DialogResponse {
        req_id: Option<String>,
        id: String,
        answer: DialogAnswer,
    },
}
```

This spec deliberately omits 27 of pi's 35 commands. rho starts with nine.
The omitted commands include thinking levels, queue modes, compaction controls,
retry controls, bash execution, session branching, session export, and tree
navigation. Add them only when a caller needs them. Every command is a promise
every side must keep.

### 3.2 Reply

A reply is a JSON object on stdout. `req_id` echoes the value from the command.
`command` names the command type. Replies and events share the same stream.

A client routes each line by these two rules, and either one is enough:

- A reply carries `command` and `success`. It never carries `type`.
- An event carries `type`. It never carries `success`.

`success` is a reserved field name. Only a reply may use it. An invariant test builds
one of every event and fails if any of them carries it. See
D-no-event-carries-the-success-key.

```rust
/// A reply to one command. Emitted on stdout as one JSON line.
///
/// This is an enum, not a struct holding a `success` flag beside an optional
/// error. A struct makes `success: true` with an error present representable,
/// and a wrong state that a type permits is a defect waiting for a careless
/// writer. Decision D-child-confined-by-composition states the rule: make the
/// wrong state unrepresentable rather than merely tested.
///
/// `untagged` is safe here, and only here. The two arms differ by a one-value
/// type, so `success: true` cannot read as `ReplyErr` and `success: false`
/// cannot read as `ReplyOk`. Contrast `D-a-dialog-answer-holds-exactly-one-value`,
/// where the arms overlapped and untagged had to go.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Reply {
    Ok(ReplyOk),
    Err(ReplyErr),
}

/// A command that rho accepted. `success` always serialises as `true`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplyOk {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub req_id: Option<String>,
    pub command: String,
    /// Always `true` on the wire. A client routes on this field.
    pub success: True,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// A command that failed before acceptance. `success` always serialises as `false`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplyErr {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub req_id: Option<String>,
    pub command: String,
    /// Always `false` on the wire.
    pub success: False,
    /// The machine-readable case. A client matches on this.
    pub error: ReplyError,
    /// Prose for a human. A client must never match on this.
    pub message: String,
}

/// Named error variants. The client maps each variant to a display string.
/// The client must not match on the `message` field of the reply.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplyError {
    /// The `type` value is not a command this build knows.
    UnknownCommand,
    /// The line is not valid JSON, a required field is missing, or a field is unknown.
    ParseError,
    /// The line passed the byte cap before its newline arrived.
    LineTooLong,
    /// A Prompt, SetModel, or NewSession arrived while a run is going.
    AlreadyStreaming,
    /// The steering queue is full. The earlier messages are still queued.
    QueueFull,
    /// A required argument is invalid. A malformed dialog answer lands here.
    InvalidArgument,
    /// The provider name is not one this build has.
    UnknownProvider,
    /// The provider has no credential in the environment.
    MissingCredential,
    /// An internal error stopped the command. The session is still open.
    Internal,
}
```

`rho-jsonl` owns and handles `UnknownCommand`, `ParseError`, and `LineTooLong`.
The rest may surface to the user.

`True` and `False` are one-value types that serialise to the matching JSON
literal. They keep the wire shape a pi-style client expects, and they stop the
two cases from drifting apart. A client still routes on `success`.

```rust
/// A type with one value. It serialises to the JSON literal `true`, and it
/// refuses to deserialise from `false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct True;

/// A type with one value. It serialises to the JSON literal `false`, and it
/// refuses to deserialise from `true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct False;
```

### 3.3 Event

An event is a JSON object on stdout. Every event has a `"type"` field. A reply
has a `"success"` field. A client uses those two fields to route each line.

An event type carries no `deny_unknown_fields`, so an older Rust client reads a
newer event's known fields and ignores the rest. See
`D-a-command-is-strict-and-an-event-is-loose`.

```rust
use rho_core::{AgentStopReason, StopReason, ToolKind};

/// An event emitted by rho-jsonl during agent operation. One JSON line on stdout.
/// Maps from rho_core::AgentEvent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// One provider turn begins. Maps from AgentEvent::TurnStart.
    TurnStart,
    /// One provider turn ends. Maps from AgentEvent::TurnEnd.
    ///
    /// It carries the reason, because rho-core carries one. The draft dropped it,
    /// and a dropped field is a silent loss.
    TurnEnd { stop_reason: StopReason },
    /// A text delta from the current assistant message.
    /// Maps from AgentEvent::Stream(StreamEvent::TextDelta).
    TextDelta { index: u32, delta: String },
    /// A tool call begins. The approval gate has passed.
    /// Maps from AgentEvent::ToolStart. `kind` is rho-core's own enum, not a copy.
    ToolStart {
        id: String,
        name: String,
        kind: ToolKind,
    },
    /// A streamed line of tool output. Maps from AgentEvent::ToolUpdate.
    ToolUpdate { id: String, output: String },
    /// A tool call finished. `success` is the inverse of ToolOutput::is_error.
    /// Maps from AgentEvent::ToolEnd.
    ///
    /// The field is `ok`, and not `success`. Only a reply may carry `success`, because
    /// that is how a client tells a reply from an event. See
    /// D-no-event-carries-the-success-key.
    ToolEnd { id: String, ok: bool },
    /// A steered message was queued. `position` counts from one.
    /// Maps from AgentEvent::MessageQueued.
    MessageQueued { position: usize },
    /// Queued messages reached the model at a turn boundary.
    /// Maps from AgentEvent::MessageDelivered. This is how a client sees a steer land.
    MessageDelivered { count: usize },
    /// A dialog needs a client answer. See section 4.
    /// The agent side owns any timeout. The client does not track timeouts.
    Dialog(DialogRequest),
    /// A non-fatal error after acceptance. See section 6.
    Fault { kind: FaultKind, message: String },
    /// The run is settled. No automatic continuation remains, and no further
    /// event arrives for that prompt. This is the only end signal.
    /// See D-settled-is-the-only-end-signal.
    Settled { stop_reason: SettleReason },
}

/// Why a run settled, on the wire.
///
/// It mirrors `rho_core::AgentStopReason` value for value and adds one case that
/// core has no variant for. A run that fails at the provider emits no `AgentEnd`,
/// so `rho-jsonl` settles it itself with `faulted`. See
/// D-the-frontend-settles-every-prompt.
///
/// The `cancelled` spelling matches ACP. Rust spells the variant Canceled with
/// one l, and the serde rename corrects the wire value. See D-acp-cancelled-spelling.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SettleReason {
    EndTurn,
    MaxTokens,
    MaxToolCalls,
    MaxTurnRequests,
    Refusal,
    #[serde(rename = "cancelled")]
    Canceled,
    /// The stream ended on an error and rho-core emitted no end event. A Fault
    /// event came first, and it says why.
    Faulted,
}

impl From<AgentStopReason> for SettleReason {
    /// An exhaustive map with no wildcard arm. A new core variant is a compile
    /// error here, and never a silent `faulted`.
    fn from(reason: AgentStopReason) -> Self {
        match reason {
            AgentStopReason::EndTurn => Self::EndTurn,
            AgentStopReason::MaxTokens => Self::MaxTokens,
            AgentStopReason::MaxToolCalls => Self::MaxToolCalls,
            AgentStopReason::MaxTurnRequests => Self::MaxTurnRequests,
            AgentStopReason::Refusal => Self::Refusal,
            AgentStopReason::Canceled => Self::Canceled,
        }
    }
}

/// Kind of a post-acceptance fault. One variant per `rho_core::Error` case.
///
/// There is no `BudgetExceeded`. Nothing emits it today, and an event with no
/// producer is dead surface. See D-dead-surface-is-a-defect-class.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind {
    /// From Error::Provider.
    Provider,
    /// From Error::Tool.
    Tool,
    /// From Error::Canceled.
    Canceled,
    /// The stream ended with no end event, and no error explained it.
    Incomplete,
}
```

The event mapping from `rho_core::AgentEvent`:

| `AgentEvent` | Wire event |
|---|---|
| `TurnStart` | `TurnStart` |
| `TurnEnd { stop_reason }` | `TurnEnd { stop_reason }` |
| `Stream(TextDelta { index, delta })` | `TextDelta { index, delta }` |
| `ToolStart { id, name, kind }` | `ToolStart { id, name, kind }` |
| `ToolUpdate { id, output }` | `ToolUpdate { id, output }` |
| `ToolEnd { id, output }` | `ToolEnd { id, ok: !output.is_error }` |
| `MessageQueued { position }` | `MessageQueued { position }` |
| `MessageDelivered { count }` | `MessageDelivered { count }` |
| `AgentEnd { stop_reason }` | `Settled { stop_reason: reason.into() }` |
| `Err(Error)` from the stream | `Fault { kind, message }`, then `Settled { faulted }` |

**Two spellings of one word travel on this wire.** `turn_end` carries `canceled`, from
`rho_core::StopReason`. `settled` carries `cancelled`, from `rho_core::AgentStopReason`,
which has a serde rename so its value matches ACP. See `D-acp-cancelled-spelling`. A
client must accept both, one per event. Changing either would change a persisted format
that another lane owns, so this spec records the wart instead of hiding it. Two tests pin
the two spellings, so neither can drift.

**An error ends the run.** When an error arrives, the frontend writes the `Fault`, writes
`Settled { stop_reason: faulted }`, and stops reading. It does not wait for the stream to
close. See `D-an-error-on-the-event-stream-ends-the-run`, which records the rho-core leak
that makes waiting a hang.

These `AgentEvent` variants are out of scope, and `rho-jsonl` drops each one:
every other `Stream` variant, `TaskStart`, `TaskProgressed`, `TaskEnd`,
`AgentSpawned`, `AgentProgressed`, and `AgentFinished`. See section 7.

### 3.4 Dialog sub-protocol

Extensions and the approval gate need a human reply. The dialog sub-protocol
layers on the same stream. It does not need a separate channel.

`F-extension-ui-sub-protocol` in `docs/features.md` already describes this
pattern for ACP. It uses the name `extension_ui_request`, which comes from pi's
RPC mode and is not an ACP method. Real ACP uses `session/request_permission`.
`rho-jsonl` adopts the same shape under the honest name `DialogRequest`.

A `dialog` event arrives on stdout. The client sends a `Command::DialogResponse`
on stdin. The `id` field links them. The agent side owns the timeout, so the two
sides can never disagree about whether a dialog is open.

```rust
/// Emitted by rho-jsonl on stdout when the agent needs a human decision.
///
/// It nests inside `Event::Dialog`, so a line carries both tags:
/// `{"type":"dialog","method":"confirm",...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum DialogRequest {
    /// Choose one option. It blocks the agent until an answer or the timeout.
    Select {
        id: String,
        title: String,
        options: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    /// Yes or no. It blocks the agent until an answer or the timeout.
    Confirm {
        id: String,
        title: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    /// Free text input. It blocks the agent until an answer or the timeout.
    Input {
        id: String,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    /// Display a message. Fire-and-forget: the client must not reply.
    Notify { id: String, message: String },
}

/// The answer in a Command::DialogResponse.
///
/// The wire shape is one of `{"value":"a"}`, `{"confirmed":true}`, or
/// `{"cancelled":true}`. Exactly one key. Zero keys is refused, and two keys are
/// refused. An untagged reader would take the first match in silence, and a
/// dialog answer decides whether a tool runs. See
/// D-a-dialog-answer-holds-exactly-one-value.
///
/// `cancelled` takes only the literal `true`. `{"cancelled":false}` means "I did not
/// cancel", and reading that as a cancellation would deny a tool the user never
/// refused. So the reader refuses the literal instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "WireAnswer", into = "WireAnswer")]
pub enum DialogAnswer {
    /// A chosen option, or typed input text.
    Value(String),
    /// A yes or no answer.
    Confirmed(bool),
    /// The user dismissed the dialog. A timeout resolves to this.
    Cancelled,
}
```

`Notify` is fire-and-forget. Do not answer a `Notify`. Every other method blocks
the agent until an answer arrives or the timeout expires.

**A timeout resolves as `Cancelled`.** One rule for all three blocking methods.
`Cancelled` reads as a denial. See `D-a-dialog-timeout-cancels`, which says why
no other default is safe. The client receives no second event for that dialog id,
and a late answer for a resolved id is dropped.

`rho-jsonl` ships one producer of dialogs, so the sub-protocol is not dead
surface. `DialogApproval` implements `rho_core::ApprovalPolicy`. It asks the
client with a `Confirm` dialog, and it maps the answer like this:

| Answer | Decision |
|---|---|
| `Confirmed(true)` | `Allow` |
| `Confirmed(false)` | `Deny` |
| `Cancelled`, including a timeout | `Deny` |
| `Value(_)` | `Deny` |

Every case that is not an explicit yes denies the call. That is the fail-closed
rule of `D-ask-policy-fails-closed`.

## 4. Framing

`rho-jsonl` uses strict JSONL semantics. LF (`\n`, U+000A) is the only record
delimiter.

A reader must split on `\n` only. A reader must strip a trailing `\r` before
parsing. A reader must not split on U+2028 (LINE SEPARATOR) or U+2029
(PARAGRAPH SEPARATOR). Those code points are valid inside JSON strings. A
reader that splits on them corrupts real data. Node's `readline` is not
protocol-compliant for this reason.

A writer must end every record with exactly one `\n`. A writer must not add
`\r`.

**One line is capped.** `MAX_COMMAND_LINE_BYTES` is 1 MiB. The reader counts
bytes read, not bytes kept. It refuses an over-long line with `LineTooLong`,
discards bytes to the next `\n`, and keeps the session open. A peer that never
writes a newline cannot grow the reader without limit. See
`D-a-command-line-is-capped`.

## 5. Ordering and one-writer discipline

The frontend runs as `rho jsonl`, behind the `jsonl` cargo feature. The feature is on in
the default build and in the `minimal` build, because a headless build with no headless
frontend can answer only one prompt.


One task writes stdout. Every reply and every event goes through it, so two
lines never interleave. A reader on the far side may assume every line is whole.

Ordering rules a client may trust:

- A reply to a command arrives before any event that command causes.
- Exactly one reply per command line, including a line that fails to parse.
- Exactly one `Settled` per accepted `Prompt`.
- No event for a prompt arrives after that prompt's `Settled`.

## 6. Acceptance versus completion

A successful reply to `Prompt` means the prompt was accepted. It does not mean
the agent has finished. A failure before acceptance arrives as a reply with
`success: false`. A failure after acceptance arrives as a `Fault` event. No
second reply arrives for the same `req_id`.

The agent is done only after `Settled`. `Settled` always arrives, even for a run
that faults, because `rho-jsonl` owns that pairing rather than `rho-core`. See
`D-the-frontend-settles-every-prompt`.

## 7. Out of scope

These items are not part of this spec:

- Thinking-block events (`ThinkingDelta`, `ThinkingEnd`) and usage events.
- Background task events (`TaskStart`, `TaskProgressed`, `TaskEnd`).
- Subagent events (`AgentSpawned`, `AgentProgressed`, `AgentFinished`).
- Session branching, forking, resuming, and tree navigation.
- Session recording to disk. Another lane owns the session store wiring.
- Context compaction commands and events.
- Bash execution commands, and session export.
- Thinking-level and queue-mode controls.
- Image content in a prompt. `Prompt` carries text.
- The ACP bridge. See `SPEC-acp`. The TUI. See `SPEC-tui`.

Add any of these as a **new** `type` value on `Command` or `Event`. Never as a
new field on an existing variant. See `D-a-command-is-strict-and-an-event-is-loose`.

## 8. The public API

```rust
/// Serve the protocol over one pair of streams until stdin closes.
///
/// It returns when the input reaches end of file, or when the output closes.
pub async fn serve<R, W>(
    factory: std::sync::Arc<dyn SessionFactory>,
    start: SessionRequest,
    input: R,
    output: W,
) -> std::io::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send + Sync + 'static;

/// Map one core event onto its wire event. `None` means the variant is out of scope.
pub fn map_event(event: &rho_core::AgentEvent) -> Option<Event>;

/// Drive one run's events onto the writer, and settle it exactly once.
///
/// It takes any stream of run events, not only `rho_core::AgentEvents`. That is what
/// makes the pairing testable with no provider and no network.
pub async fn pump_run<S, W>(events: S, out: &Writer<W>) -> std::io::Result<RunOutcome>
where
    S: futures::Stream<Item = Result<rho_core::AgentEvent, rho_core::Error>> + Unpin,
    W: tokio::io::AsyncWrite + Unpin;
```

## 9. The extension point

`SessionFactory` is the extension point, and it is the reason `rho-jsonl` needs
no provider crate. See `D-rho-jsonl-asks-a-factory-for-a-session`.

```rust
/// What a host must provide so the frontend can build a session.
///
/// `rho-cli` implements this. So does any embedder that wants the protocol over
/// its own streams, with its own providers, tools, and approval policy.
#[async_trait::async_trait]
pub trait SessionFactory: Send + Sync {
    /// Build a session for this provider and model. Called for the first session,
    /// and again for every `set_model` and `new_session`.
    ///
    /// `asker` reaches the client over the protocol. A host that wants a human in the
    /// approval loop wraps it in `DialogApproval` and puts that in the `SessionConfig`.
    /// Passing it here is what makes the dialog sub-protocol reachable, rather than a
    /// shape with no producer.
    async fn build(
        &self,
        request: &SessionRequest,
        asker: std::sync::Arc<dyn Asker>,
    ) -> Result<rho_core::Session, FactoryError>;
}

/// What a host uses to ask the client a question.
#[async_trait::async_trait]
pub trait Asker: Send + Sync {
    /// Ask a blocking question. A timeout resolves as `Cancelled`.
    async fn ask(&self, request: DialogRequest) -> DialogAnswer;
    /// Tell the client something. It expects no answer.
    async fn notify(&self, message: String);
    /// Mint a fresh dialog id.
    fn next_dialog_id(&self) -> String;
}

/// Which provider and model a session runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRequest {
    pub provider: String,
    pub model_id: String,
}

/// Why a session could not be built. Each case maps to a named ReplyError.
#[derive(Debug, thiserror::Error)]
pub enum FactoryError {
    /// Maps to ReplyError::UnknownProvider.
    #[error("no provider is called {name}")]
    UnknownProvider { name: String },
    /// Maps to ReplyError::MissingCredential. It never carries the credential.
    #[error("{provider} has no credential: set {variable}")]
    MissingCredential { provider: String, variable: String },
    /// Maps to ReplyError::InvalidArgument.
    #[error("{provider} refused the model {model_id}: {reason}")]
    RefusedModel {
        provider: String,
        model_id: String,
        reason: String,
    },
    /// Maps to ReplyError::Internal.
    #[error("cannot build the session: {0}")]
    Internal(String),
}
```

A third party adds no enum variant and edits no shared file. It writes one impl of
`SessionFactory` and calls `serve`. A new command variant is upstream work, and a client
discovers what a build accepts with `get_commands`.

The trait has exactly one method. An earlier draft also had `providers`, so `get_state`
could list the provider names a build has. It came out again, because `rho-cli` cannot
answer it without a second copy of its provider list, and a copied list is the drift this
spec already refused once. A client discovers a provider by trying `set_model` and reading
the named error, which is how it discovers a command too.

`rho-cli` holds the only implementation in the tree, in `crates/rho-cli/src/jsonl.rs`. It
reuses the same config merge, provider set, tool registry, and skill loader that `rho run`
uses, so a config file reaches a JSONL session exactly as it reaches a headless run.

## 10. Test cases

Every test uses a scripted fake provider in `tests/support/mod.rs`. No test uses the
network. No test uses `sleep`. The timeout tests use `tokio::time` pause and advance. Each
session root is a `tempfile::TempDir`, so no test reads the real `~/.rho`.

The live transcripts are in `docs/verification/jsonl-frontend.md`.

### Framing, in `tests/frame.rs`

| Test name | Assertion |
|---|---|
| `crlf_line_parses` | A line ending with CRLF is one record. The CR is stripped before parsing. |
| `unicode_line_separator_inside_string` | A line holding U+2028 inside a JSON string is one record. |
| `a_last_line_with_no_newline_is_a_record` | The final line needs no newline. |
| `an_empty_input_yields_no_record` | An empty input yields nothing, and no error. |
| `a_blank_line_is_an_empty_record` | A blank line is a record, so the reply count matches the line count. |
| `invalid_utf8_reaches_the_caller_as_bytes` | Invalid UTF-8 is not a reader error. The caller replies with a parse error. |
| `an_over_long_line_is_refused_and_bounded` | A line past the cap yields `TooLong`. It asserts the bytes read, never the bytes kept. |
| `the_reader_resumes_after_an_over_long_line` | The command after a refused line parses normally. |
| `an_unterminated_line_cannot_grow_without_limit` | A peer that sends no newline is stopped at the cap, and each call returns. |
| `a_dropped_next_line_loses_no_bytes` | The read future is cancel safe. A dropped read loses no byte of a command. |

### The wire format, in `tests/protocol.rs`

| Test name | Assertion |
|---|---|
| `a_success_reply_carries_no_error_field` | A `ReplyOk` serialises with `success: true` and no `error` key. |
| `an_error_reply_always_says_success_false` | A `ReplyErr` carries `success: false`, a named error, and a message. |
| `a_reply_routes_on_the_success_field` | `Reply` round trips, and `success` alone picks the arm. |
| `reply_carries_req_id` | A reply echoes the `req_id`. An absent one serialises to no key at all. |
| `an_event_has_a_type_and_no_success_field` | Every event variant carries `type` and never `success`. It pins the routing rule. |
| `every_settle_reason_has_a_wire_value` | Every core stop reason maps to a distinct wire value, and `cancelled` keeps the ACP spelling. |
| `every_turn_end_stop_reason_has_a_wire_value` | Every `StopReason` has a pinned wire value on `turn_end`. |
| `every_tool_kind_reaches_the_wire` | Every one of the ten `ToolKind` variants has a pinned wire value. |
| `unknown_command_is_reply_error` | An unknown `type` is refused, and the reader names the variant. |
| `parse_error_is_reply_error` | Malformed JSON, and a missing required field, are both refused. |
| `an_unknown_field_on_a_command_is_refused` | A command with an extra field is refused whole, and the message names the field. |
| `a_new_field_on_an_event_is_ignored_by_an_old_reader` | An event with an extra key still parses. It pins the loose-reader half. |
| `an_absent_req_id_reads_as_none` | `req_id` is optional with no `serde(default)`. |
| `get_commands_lists_every_command` | `Command::NAMES` holds one entry per variant, and the reader accepts each name. |
| `a_dialog_answer_holds_exactly_one_value` | Each answer shape round trips to its one wire key. |
| `a_dialog_answer_with_two_keys_is_invalid_argument` | Two answers in one object are refused, and the message says the rule. |
| `a_dialog_answer_with_no_key_is_invalid_argument` | An empty answer is refused. |
| `a_dialog_request_round_trips_under_two_tags` | A dialog survives nesting under both `type` and `method`. |
| `a_line_holding_a_newline_in_a_string_is_escaped` | A prompt holding a newline stays one record. |
| `every_public_reply_type_is_constructed_and_pinned` | Each public reply type is built by hand and its wire shape pinned. `True` and `False` each refuse the wrong literal. |
| `every_reply_error_case_has_a_distinct_wire_value` | All nine error cases have distinct wire values, so no two collapse into one. |
| `every_fault_kind_has_a_distinct_wire_value` | All four fault kinds have distinct wire values. |

### The event pump, in `tests/pump.rs`

| Test name | Assertion |
|---|---|
| `settled_is_the_last_event_of_a_run` | `Settled` is last, and it arrives once. |
| `no_event_arrives_after_settled` | The pump stops at `Settled`, even when the stream keeps talking. |
| `a_faulting_run_still_settles_once` | A run whose stream errors settles once, as `faulted`, after one `Fault`. |
| `a_silent_stream_end_is_an_incomplete_fault` | A stream that ends with no result settles, and says nothing explained it. |
| `every_accepted_prompt_settles_exactly_once` | Over three run shapes, the `Settled` count equals the run count. It pins the pairing. |
| `every_error_variant_has_a_named_fault_kind` | Every `rho_core::Error` case maps to its own fault kind. |
| `a_tool_failure_is_reported_as_not_ok` | `ok` is the inverse of `is_error`, in both directions. |
| `the_event_map_drops_only_what_the_spec_lists` | Every mapped variant maps, and every out-of-scope variant drops. |

### The dialog sub-protocol, in `tests/dialog.rs`

| Test name | Assertion |
|---|---|
| `dialog_timeout_auto_resolves_cancelled` | A timeout resolves as `Cancelled`, emits no second request, and frees the slot. |
| `dialog_response_id_matches_request` | The right id resolves a dialog. A wrong id is dropped and the dialog stays open. |
| `a_late_dialog_answer_is_dropped` | An answer for a resolved id changes nothing and emits no event. |
| `dialog_notify_expects_no_reply` | A notify opens no slot and blocks nothing. |
| `asking_a_notify_does_not_block` | Passing a notify to `ask` returns instead of hanging the run. |
| `dialog_ids_are_unique` | No id repeats, so one answer cannot resolve two dialogs. |
| `dialog_approval_allows_only_an_explicit_yes` | Only `Confirmed(true)` allows. Every other answer denies. |
| `dialog_approval_denies_on_a_timeout` | A silent client never gets a tool approved. |
| `the_approval_dialog_carries_no_tool_arguments` | A secret in a tool argument never reaches the dialog. |
| `the_approval_dialog_always_carries_a_timeout` | The approval gate cannot hang, because its dialog always has a timeout. |
| `every_tool_kind_gets_a_plain_word` | Every tool kind reads as a word a human can understand. |

### The serve loop, in `tests/serve.rs`

| Test name | Assertion |
|---|---|
| `a_prompt_replies_then_streams_then_settles` | The reply precedes every event of that run, and one reply is sent. |
| `unknown_command_is_reply_error` | An unknown command is named, and the session stays open. |
| `parse_error_is_reply_error_and_the_session_stays_open` | A malformed line is refused, and the next command works. |
| `an_unknown_field_on_a_command_is_refused` | The whole command is refused, and no run starts. |
| `a_crlf_command_line_is_accepted` | A CRLF command line works end to end. |
| `steer_before_a_run_is_accepted` | An early steer succeeds and reports its position. |
| `steer_lands_at_a_turn_boundary` | Delivery sits between a turn end and the next turn start. |
| `a_full_queue_is_queue_full` | A full queue is named, and every earlier message stays queued. |
| `abort_with_no_run_is_accepted` | An abort with no run succeeds and emits no event. |
| `a_prompt_after_a_settled_run_is_accepted` | A prompt sent after `settled` starts a second run. |
| `a_second_prompt_while_running_is_refused` | A prompt during a run is refused, and the run is untouched. |
| `abort_during_stream_settles_cancelled` | An abort settles the run as `cancelled`. |
| `two_aborts_settle_once` | Two aborts settle one run exactly once. |
| `set_model_while_running_is_refused` | A model swap during a run is refused, and the model is kept. |
| `stdin_eof_mid_run_still_settles` | A run drains to `settled` when the client closes stdin. |
| `new_session_clears_the_messages` | A fresh session starts with an empty conversation. |
| `a_faulting_provider_settles_and_the_session_survives` | Two provider failures settle once each, and the session lives. |
| `a_stream_that_ends_with_no_done_still_settles` | A truncated provider stream still settles. |
| `set_model_round_trip` | A swap replies with the new model, and `get_state` then reports it. |
| `every_factory_failure_has_its_own_named_case` | Each of the four factory errors maps to its own wire case. |
| `a_missing_credential_reply_names_the_variable_and_no_secret` | The reply names the variable to set, and no value. |
| `get_commands_lists_every_command` | `get_commands` reports one name per command. |
| `an_over_long_line_is_refused_once_and_the_next_command_works` | One long line is one reply, not one per buffer. |
| `a_dialog_answer_with_a_wrong_shape_is_invalid_argument` | A malformed answer is refused, and the session stays open. |
| `a_dialog_answer_for_no_open_dialog_is_dropped` | The reply says the answer was not delivered. |
| `the_first_session_failure_is_reported_and_serve_stops` | A build failure at start is reported, never a silent exit. |
| `no_line_is_ever_half_written` | Every line is whole JSON, with replies and events racing. |
| `a_dialog_reaches_the_client_and_its_answer_runs_the_tool` | The whole dialog round trip over the wire, and the tool then runs. |
| `a_denied_dialog_stops_the_tool` | An explicit no leaves the tool unrun. |
| `a_client_that_answers_no_dialog_denies_the_tool_and_the_run_continues` | A client with no dialog support cannot hang rho, and gets no tool approved. |
