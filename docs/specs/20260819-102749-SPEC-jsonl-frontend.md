# SPEC-jsonl-frontend — JSONL headless frontend

Status: draft. Owning crate: `rho-jsonl` (new). Depends on `rho-core` only.
Links no provider crate. `rho-core` keeps no terminal and no HTTP dependency.

Features covered: F-jsonl-frontend, F-jsonl-prompt, F-jsonl-steer,
F-jsonl-abort, F-jsonl-session-commands, F-jsonl-dialog-sub-protocol.

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

### 3.1 Command

A command is a JSON object on stdin, one per line. `req_id` is optional. Use
it to correlate a reply to the command that sent it.

```rust
use serde::{Deserialize, Serialize};

/// A command sent to rho-jsonl on stdin, one per line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    Prompt {
        req_id: Option<String>,
        message: String,
    },
    Steer {
        req_id: Option<String>,
        message: String,
    },
    Abort {
        req_id: Option<String>,
    },
    GetState {
        req_id: Option<String>,
    },
    SetModel {
        req_id: Option<String>,
        provider: String,
        model_id: String,
    },
    NewSession {
        req_id: Option<String>,
    },
    GetMessages {
        req_id: Option<String>,
    },
    GetCommands {
        req_id: Option<String>,
    },
    /// Reply to a dialog from the agent. `id` matches the DialogRequest, not req_id.
    DialogResponse {
        id: String,
        #[serde(flatten)]
        value: DialogValue,
    },
}
```

This spec deliberately omits 27 of pi's 35 commands. rho starts with eight.
The omitted commands include thinking levels, queue modes, compaction controls,
retry controls, bash execution, session branching, session export, and tree
navigation. Add them only when a caller needs them. Every command is a promise
every side must keep.

### 3.2 Reply

A reply is a JSON object on stdout. `req_id` echoes the value from the command.
`command` names the command type. Replies and events share the same stream.
Distinguish them by the top-level `"success"` field: replies carry it, events
do not.

```rust
/// A reply to one command. Emitted on stdout as one JSON line.
///
/// This is an enum, not a struct holding a `success` flag beside an optional
/// error. A struct makes `success: true` with an error present representable,
/// and a wrong state that a type permits is a defect waiting for a careless
/// writer. Decision D-child-confined-by-composition states the rule: make the
/// wrong state unrepresentable rather than merely tested.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Reply {
    Ok(ReplyOk),
    Err(ReplyErr),
}

/// A command that rho accepted. `success` always serialises as `true`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ReplyOk {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub req_id: Option<String>,
    pub command: String,
    /// Always `true` on the wire. A client routes on this field.
    pub success: True,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// A command that failed before acceptance. `success` always serialises as `false`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ReplyErr {
    #[serde(skip_serializing_if = "Option::is_none")]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplyError {
    /// The command type is not in the Command enum.
    UnknownCommand,
    /// The line is not valid JSON, or a required field is missing.
    ParseError,
    /// A Prompt arrived while the agent is streaming. No streaming behaviour was set.
    AlreadyStreaming,
    /// A Steer arrived while the agent is not streaming.
    NotStreaming,
    /// A required argument is invalid.
    InvalidArgument,
    /// An internal error stopped the command. The session is still open.
    Internal,
}
```

`rho-jsonl` owns and handles `UnknownCommand` and `ParseError`. All other
variants may surface to the user.

`True` and `False` are one-value types that serialise to the matching JSON
literal. They keep the wire shape a pi-style client expects, and they stop the
two cases from drifting apart. A client still routes on `success`.

### 3.3 Event

An event is a JSON object on stdout. Every event has a `"type"` field. A reply
has a `"success"` field. A client uses those two fields to route each line.

```rust
use crate::{AgentStopReasonWire, DialogRequest, FaultKind, ToolKindWire};

/// An event emitted by rho-jsonl during agent operation. One JSON line on stdout.
/// Maps from rho_core::AgentEvent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// One provider turn begins. Maps from AgentEvent::TurnStart.
    TurnStart,
    /// One provider turn ends. Maps from AgentEvent::TurnEnd.
    TurnEnd,
    /// A text delta from the current assistant message.
    /// Maps from AgentEvent::Stream(StreamEvent::TextDelta).
    TextDelta {
        index: u32,
        delta: String,
    },
    /// A tool call begins. The approval gate has passed.
    /// Maps from AgentEvent::ToolStart.
    ToolStart {
        id: String,
        name: String,
        kind: ToolKindWire,
    },
    /// A streamed line of tool output.
    /// Maps from AgentEvent::ToolUpdate.
    ToolUpdate {
        id: String,
        output: String,
    },
    /// A tool call finished.
    /// Maps from AgentEvent::ToolEnd.
    ToolEnd {
        id: String,
        success: bool,
    },
    /// One low-level agent run is done. `will_retry` is true when a retry follows.
    /// When rho-core emits AgentEnd today, rho-jsonl fires RunEnd with will_retry: false
    /// immediately before Settled. Future versions will fire RunEnd with will_retry: true
    /// between retries.
    RunEnd {
        stop_reason: AgentStopReasonWire,
        will_retry: bool,
    },
    /// The session-level run is fully settled. No automatic continuation remains.
    /// Maps from AgentEvent::AgentEnd. This is the authoritative end signal.
    /// A client that waits on RunEnd may see an incomplete answer when will_retry is true.
    Settled {
        stop_reason: AgentStopReasonWire,
    },
    /// A dialog request needs a client response. See section 7.
    /// The agent side owns any timeout. The client does not track timeouts.
    DialogRequest(DialogRequest),
    /// A non-fatal error after acceptance. See section 5.
    Fault {
        message: String,
        kind: FaultKind,
    },
}

/// Wire form of rho_core::AgentStopReason.
/// Mirrors the ACP StopReason set so a later bridge maps one-to-one.
/// The `cancelled` spelling matches ACP. Rust spells the variant Canceled
/// with one l; a serde rename corrects the wire value. See SPEC-acp section 5.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentStopReasonWire {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    #[serde(rename = "cancelled")]
    Canceled,
}

/// Wire form of rho_core::ToolKind.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolKindWire {
    Read,
    Write,
    Execute,
    Ask,
}

/// Kind of a post-acceptance fault.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind {
    ToolError,
    ProviderError,
    BudgetExceeded,
}
```

The event mapping from `rho_core::AgentEvent`:

| `AgentEvent` | Wire event |
|---|---|
| `TurnStart` | `TurnStart` |
| `TurnEnd { .. }` | `TurnEnd` |
| `Stream(TextDelta { index, delta })` | `TextDelta { index, delta }` |
| `ToolStart { id, name, kind }` | `ToolStart { id, name, kind }` |
| `ToolUpdate { id, output }` | `ToolUpdate { id, output }` |
| `ToolEnd { id, output }` | `ToolEnd { id, success }` |
| `AgentEnd { stop_reason }` | `RunEnd { will_retry: false }` then `Settled { stop_reason }` |

These `AgentEvent` variants are out of scope for this spec: `Stream(ThinkingDelta)`,
`TaskStart`, `TaskProgressed`, `TaskEnd`, `AgentSpawned`, `AgentProgressed`,
`AgentFinished`. `rho-jsonl` drops them silently today. See section 6.

### 3.4 Dialog sub-protocol

Extensions and the approval gate need a human reply. The dialog sub-protocol
layers on the same stream. It does not need a separate channel.

`F-extension-ui-sub-protocol` in `docs/features.md` already describes this
pattern. That row sits in the `## Frontends — ACP` section and names `rho-acp`
as owner. It uses the name `extension_ui_request`. That name comes from pi's
RPC mode. It is not an ACP method. Real ACP uses `session/request_permission`.
The presence of the pi shape in the ACP section is evidence the owner already
reached for it before this decision was written. `rho-jsonl` adopts the same
shape. The wire type is called `DialogRequest` to avoid the naming confusion.

A `DialogRequest` event arrives on stdout. The client sends a `Command::DialogResponse`
on stdin. The `id` field links them. The agent side owns any timeout. The client
does not need to track timeouts. pi's documentation states the same rule.

```rust
/// Emitted by rho-jsonl on stdout when the agent needs a human decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum DialogRequest {
    /// Choose one option. Dialog method: blocks until a response arrives.
    Select {
        id: String,
        title: String,
        options: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    /// Yes or no. Dialog method: blocks until a response arrives.
    Confirm {
        id: String,
        title: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    /// Free text input. Dialog method: blocks until a response arrives.
    Input {
        id: String,
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    /// Display a message. Fire-and-forget: the client must not reply.
    Notify {
        id: String,
        message: String,
    },
}

/// The payload in a Command::DialogResponse.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DialogValue {
    /// Selected option string, or typed input text.
    Value { value: String },
    /// Yes or no answer.
    Confirmed { confirmed: bool },
    /// The user dismissed the dialog.
    Cancelled { cancelled: bool },
}
```

`Notify` is fire-and-forget. Do not send a reply for a `Notify` dialog. All
other dialog methods block the agent until a reply arrives or the timeout
expires. When the timeout expires, the agent resolves with a default and
continues. The client receives no second event for the same dialog id.

`SPEC-budget-governor` tracks nested tool usage through the `ToolResultMessage`
usage field. `rho-jsonl` reports usage events for dialog round-trips when a
nested LLM call occurs, so the budget governor can account for them. The
`ToolResultMessage.usage` design in pi's documentation matches the rho approach.

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

## 5. Acceptance versus completion

A successful reply to `Prompt` means the prompt was accepted, queued, or
handled. It does not mean the agent has finished. A failure before acceptance
arrives as a reply with `success: false`. A failure after acceptance arrives
as a `Fault` event. No second reply arrives for the same `req_id`.

The agent is done only after `Settled`. `RunEnd` signals one low-level run. If
`will_retry` is true, a new run will start. `Settled` means no run will follow.

A client that stops reading at `RunEnd` will miss subsequent runs. Always wait
for `Settled` before treating an answer as final.

## 6. Out of scope

These items are not part of this spec:

- Thinking-block events (`ThinkingDelta`, `ThinkingEnd`).
- Background task events (`TaskStart`, `TaskProgressed`, `TaskEnd`).
- Subagent events (`AgentSpawned`, `AgentProgressed`, `AgentFinished`).
- Session branching, forking, and tree navigation.
- Context compaction commands and events.
- Bash execution commands.
- Session export.
- Thinking-level and queue-mode controls.
- The ACP bridge. See `SPEC-acp`.
- The TUI. See `SPEC-tui`.

Add any of these by extending `Command` or `Event` with new variants. Existing
clients ignore unknown fields and unknown event types. See section 8.

## 7. Test cases

Each test uses a scripted fake provider. No test uses the network. No test
uses `sleep`. Synchronise async tests with a channel or `tokio::time` pause
and advance.

| Test name | Assertion |
|---|---|
| `reply_carries_req_id` | A reply carries the `req_id` from its command. |
| `unknown_command_is_reply_error` | An unknown command produces a `ReplyError::UnknownCommand` reply, not a process fault. |
| `parse_error_is_reply_error` | A malformed JSON line produces a `ReplyError::ParseError` reply. The session stays open. |
| `crlf_line_parses` | A line ending with `\r\n` parses as one record. The `\r` is stripped before deserialization. |
| `unicode_line_separator_inside_string` | A line containing U+2028 inside a JSON string value parses as one record. The framing reader does not split on it. |
| `fault_after_acceptance_is_event` | A provider error after a `Prompt` reply arrives as a `Fault` event. No second reply arrives for the same `req_id`. |
| `run_end_before_settled` | `RunEnd` arrives before `Settled`. Both carry the same `stop_reason`. |
| `settled_after_run_end` | `Settled` is the last event of a complete run. No event follows it for that prompt. |
| `dialog_timeout_auto_resolves` | A `Select` dialog with a timeout resolves on the agent side when the timeout expires. No second `DialogRequest` arrives for the same `id`. The run continues. |
| `dialog_notify_expects_no_reply` | A `Notify` dialog does not block the agent. The next event arrives without a `DialogResponse`. |
| `dialog_response_id_matches_request` | A `DialogResponse` with the correct `id` unblocks the matching dialog. A response with a wrong `id` is silently dropped. |
| `abort_during_stream_emits_settled` | An `Abort` command during a streaming run produces a `Settled` event with `stop_reason: cancelled`. |
| `set_model_round_trip` | A `SetModel` command produces a reply with `success: true` and a data payload naming the new model. |
| `a_success_reply_carries_no_error_field` | A `ReplyOk` serialises with `success: true` and no `error` key. The struct cannot hold one. |
| `an_error_reply_always_says_success_false` | A `ReplyErr` serialises with `success: false`, a named `error`, and a `message`. |

## 8. Extension point

`Command`, `Event`, `DialogRequest`, and `ReplyError` are all enums with
`#[non_exhaustive]` on the wire-receiving side. A third party adds a new
command variant by adding a new arm to `Command` and a handler in `rho-jsonl`.
No existing variant changes. No existing client breaks. A new event variant
arrives on a client that does not know it. The client must ignore unknown
event types. This is the only guarantee.

A third party must not add a field that changes the meaning of an existing
variant. That is a modification, not an extension.
