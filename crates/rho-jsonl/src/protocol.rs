//! The wire types of the JSONL frontend.
//!
//! See `docs/specs/20260819-102749-SPEC-jsonl-frontend.md` section 3.
//!
//! Two rules govern this file, and they point in opposite directions on purpose.
//! `Command` denies an unknown field, because a command is an instruction and
//! obeying half of one is worse than refusing all of it. `Event` and `Reply` allow
//! an unknown field, because a report with a missing field is a display gap. See
//! decision D-a-command-is-strict-and-an-event-is-loose.

use serde::{Deserialize, Serialize};

use rho_core::{AgentStopReason, StopReason, ToolKind};

// ---------------------------------------------------------------- success flags

/// A type with one value. It serialises to the JSON literal `true`.
///
/// It exists so that `ReplyOk` cannot say `success: false`. A plain `bool` field
/// would let a careless writer build a reply that claims success and carries an
/// error. See `ReplyOk`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct True;

impl Serialize for True {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(true)
    }
}

impl<'de> Deserialize<'de> for True {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if bool::deserialize(deserializer)? {
            Ok(True)
        } else {
            Err(serde::de::Error::custom("expected the literal true"))
        }
    }
}

/// A type with one value. It serialises to the JSON literal `false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct False;

impl Serialize for False {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(false)
    }
}

impl<'de> Deserialize<'de> for False {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if bool::deserialize(deserializer)? {
            Err(serde::de::Error::custom("expected the literal false"))
        } else {
            Ok(False)
        }
    }
}

// ---------------------------------------------------------------- command

/// A command sent to rho-jsonl on stdin, one per line.
///
/// It denies an unknown field on purpose. See
/// decision D-a-command-is-strict-and-an-event-is-loose.
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
    Abort { req_id: Option<String> },
    /// Report the provider, the model, and whether a run is going.
    GetState { req_id: Option<String> },
    /// Replace the session with one on a new provider and model.
    SetModel {
        req_id: Option<String>,
        provider: String,
        model_id: String,
    },
    /// Replace the session with an empty one, on the same provider and model.
    NewSession { req_id: Option<String> },
    /// Report the conversation so far.
    GetMessages { req_id: Option<String> },
    /// Report the command names this build accepts.
    GetCommands { req_id: Option<String> },
    /// Answer a dialog. `id` matches the `DialogRequest`, and it is not a `req_id`.
    DialogResponse {
        req_id: Option<String>,
        id: String,
        answer: DialogAnswer,
    },
}

impl Command {
    /// Every command name this build accepts, in the order of the enum.
    ///
    /// `get_commands` reports this, so a client discovers the command set instead
    /// of guessing it from a version number. A test asserts that the list covers
    /// every variant, so a new variant with no name here fails the suite.
    pub const NAMES: &'static [&'static str] = &[
        "prompt",
        "steer",
        "abort",
        "get_state",
        "set_model",
        "new_session",
        "get_messages",
        "get_commands",
        "dialog_response",
    ];

    /// The wire name of this command. A reply echoes it in its `command` field.
    pub fn name(&self) -> &'static str {
        match self {
            Command::Prompt { .. } => "prompt",
            Command::Steer { .. } => "steer",
            Command::Abort { .. } => "abort",
            Command::GetState { .. } => "get_state",
            Command::SetModel { .. } => "set_model",
            Command::NewSession { .. } => "new_session",
            Command::GetMessages { .. } => "get_messages",
            Command::GetCommands { .. } => "get_commands",
            Command::DialogResponse { .. } => "dialog_response",
        }
    }

    /// The correlation id the caller sent, when it sent one.
    pub fn req_id(&self) -> Option<&str> {
        match self {
            Command::Prompt { req_id, .. }
            | Command::Steer { req_id, .. }
            | Command::Abort { req_id }
            | Command::GetState { req_id }
            | Command::SetModel { req_id, .. }
            | Command::NewSession { req_id }
            | Command::GetMessages { req_id }
            | Command::GetCommands { req_id }
            | Command::DialogResponse { req_id, .. } => req_id.as_deref(),
        }
    }
}

// ---------------------------------------------------------------- reply

/// A reply to one command. One JSON line on stdout.
///
/// It is an enum, not a struct with a `success` flag beside an optional error. A
/// struct makes `success: true` with an error present representable, and a wrong
/// state a type permits is a defect waiting for a careless writer. See decision
/// D-child-confined-by-composition.
///
/// `untagged` is safe here, and only here. The two arms differ by a one-value
/// type, so `success: true` cannot read as `ReplyErr`, and `success: false`
/// cannot read as `ReplyOk`. Contrast `DialogAnswer`, where the arms overlapped
/// and untagged had to go. See D-a-dialog-answer-holds-exactly-one-value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Reply {
    Ok(ReplyOk),
    Err(ReplyErr),
}

impl Reply {
    /// A success reply for this command, with no data payload.
    pub fn ok(command: &str, req_id: Option<String>) -> Self {
        Reply::Ok(ReplyOk {
            req_id,
            command: command.to_string(),
            success: True,
            data: None,
        })
    }

    /// A success reply for this command, with a data payload.
    pub fn ok_with(command: &str, req_id: Option<String>, data: serde_json::Value) -> Self {
        Reply::Ok(ReplyOk {
            req_id,
            command: command.to_string(),
            success: True,
            data: Some(data),
        })
    }

    /// A failure reply. `error` is the case a client matches on, and `message` is
    /// prose a client must never match on.
    pub fn err(
        command: &str,
        req_id: Option<String>,
        error: ReplyError,
        message: impl Into<String>,
    ) -> Self {
        Reply::Err(ReplyErr {
            req_id,
            command: command.to_string(),
            success: False,
            error,
            message: message.into(),
        })
    }
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

/// Named failure cases. A client maps each one to its own display string.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplyError {
    /// The `type` value is not a command this build knows.
    UnknownCommand,
    /// The line is not valid JSON, a field is missing, or a field is unknown.
    ParseError,
    /// The line passed the byte cap before its newline arrived.
    LineTooLong,
    /// A `Prompt`, `SetModel`, or `NewSession` arrived while a run is going.
    AlreadyStreaming,
    /// The steering queue is full. Every earlier message is still queued.
    QueueFull,
    /// A required argument is invalid. A malformed dialog answer lands here.
    InvalidArgument,
    /// The provider name is not one this build has.
    UnknownProvider,
    /// The provider has no credential in the environment.
    MissingCredential,
    /// An internal error stopped the command. The session stays open.
    Internal,
}

// ---------------------------------------------------------------- event

/// An event emitted during agent operation. One JSON line on stdout.
///
/// It maps from `rho_core::AgentEvent`. It carries no `deny_unknown_fields`, so an
/// older Rust client reads a newer event's known fields and ignores the rest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// One provider turn begins.
    TurnStart,
    /// One provider turn ends. It carries the reason, because rho-core carries one.
    TurnEnd { stop_reason: StopReason },
    /// A text delta of the current assistant message.
    TextDelta { index: u32, delta: String },
    /// A tool call begins. The approval gate has passed.
    ///
    /// `kind` is `rho_core::ToolKind` itself, not a copy of it. A copy drifted from
    /// the original before either side existed. See
    /// D-the-wire-reuses-the-core-stop-reason.
    ToolStart {
        id: String,
        name: String,
        kind: ToolKind,
    },
    /// A streamed line of tool output.
    ToolUpdate { id: String, output: String },
    /// A tool call finished. `ok` is the inverse of `ToolOutput::is_error`.
    ///
    /// The field is `ok` and not `success`. A client routes a line by looking for
    /// `success`, which only a reply carries, so an event named its own field
    /// `success` would be read as a reply. An invariant test pins the rule. See
    /// D-no-event-carries-the-success-key.
    ToolEnd { id: String, ok: bool },
    /// A steered message was queued. `position` counts from one.
    MessageQueued { position: usize },
    /// Queued messages reached the model at a turn boundary. This is how a client
    /// sees a steered message land.
    MessageDelivered { count: usize },
    /// A dialog needs a client answer. The agent side owns the timeout.
    Dialog(DialogRequest),
    /// A non-fatal error after acceptance.
    Fault { kind: FaultKind, message: String },
    /// The run is settled. It is the only end signal, and exactly one arrives per
    /// accepted prompt. See D-settled-is-the-only-end-signal.
    Settled { stop_reason: SettleReason },
}

/// One line from the event stream: an event this build knows, or one it does not.
///
/// **Read events through this, not through [`Event`].** The spec tells a client to ignore
/// an unknown event type, and `Event` alone cannot obey that rule: a tagged enum refuses a
/// tag it does not know, so a Rust client would stop reading the moment rho gained an
/// event. A client in any other language just skips the line. That made this crate the
/// worst-served reader of its own protocol.
///
/// It costs the wire nothing. It changes only what a reader accepts.
///
/// ```
/// use rho_jsonl::{Event, MaybeEvent};
///
/// // A type this build knows.
/// let known: MaybeEvent = serde_json::from_str(r#"{"type":"turn_start"}"#).unwrap();
/// assert!(matches!(known, MaybeEvent::Known(Event::TurnStart)));
///
/// // A type from a newer rho. The client skips it and keeps reading.
/// let newer: MaybeEvent = serde_json::from_str(r#"{"type":"future_event"}"#).unwrap();
/// assert!(matches!(newer, MaybeEvent::Unknown(_)));
/// ```
///
/// See D-a-command-is-strict-and-an-event-is-loose, which sets the rule for both
/// directions.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum MaybeEvent {
    /// An event this build knows. Handle it.
    Known(Event),
    /// An event type this build does not know. Skip it, and keep reading the stream.
    ///
    /// It carries the raw line, so a client that wants to log or forward it still can.
    Unknown(serde_json::Value),
}

/// Why a run settled, on the wire.
///
/// It mirrors `rho_core::AgentStopReason` value for value, and it adds one case
/// core has no variant for. A run that fails at the provider emits no `AgentEnd`,
/// so this crate settles it with `faulted`. See D-the-frontend-settles-every-prompt.
///
/// The `cancelled` spelling matches ACP. Rust spells the variant `Canceled` with
/// one `l`, and the rename corrects the wire value. See D-acp-cancelled-spelling.
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
    /// The stream ended on an error and rho-core emitted no end event. A `Fault`
    /// event came first, and it says why.
    Faulted,
}

impl From<AgentStopReason> for SettleReason {
    /// An exhaustive map with no wildcard arm.
    ///
    /// Do not add a wildcard arm. A wildcard turns a new `AgentStopReason` variant
    /// into `faulted` in silence, and a wrong end reason tells a client the answer
    /// failed when it did not. With no wildcard, a new core variant is a compile
    /// error here. That is the same rule `ToolKind::is_read_only` follows.
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

/// Kind of a post-acceptance fault. One variant per `rho_core::Error` case, plus
/// one for a stream that stopped with no reason at all.
///
/// There is no `BudgetExceeded`. Nothing emits it today, and an event with no
/// producer is dead surface. See D-dead-surface-is-a-defect-class.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind {
    /// From `rho_core::Error::Provider`.
    Provider,
    /// From `rho_core::Error::Tool`.
    Tool,
    /// From `rho_core::Error::Canceled`.
    Canceled,
    /// The stream ended with no end event, and no error explained it.
    Incomplete,
}

impl From<&rho_core::Error> for FaultKind {
    /// An exhaustive map with no wildcard arm, for the same reason as
    /// `SettleReason::from`.
    fn from(error: &rho_core::Error) -> Self {
        match error {
            rho_core::Error::Provider(_) => Self::Provider,
            rho_core::Error::Tool(_) => Self::Tool,
            rho_core::Error::Canceled => Self::Canceled,
        }
    }
}

// ---------------------------------------------------------------- dialog

/// A request for a human decision, emitted inside `Event::Dialog`.
///
/// One line then carries two tags: `{"type":"dialog","method":"confirm",...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum DialogRequest {
    /// Choose one option. It blocks the agent until an answer or the timeout.
    Select {
        id: String,
        title: String,
        options: Vec<String>,
        /// The agent-side timeout in milliseconds.
        ///
        /// It is not optional. The agent side owns every timeout, so a dialog always
        /// carries one. An optional field let a host opt out of the rule
        /// `D-a-dialog-timeout-cancels` states, and a dialog with no timeout outlived an
        /// abort: the abort cancelled the run token while this wait kept going, so the
        /// prompt could not settle until the client answered.
        timeout_ms: u64,
    },
    /// Yes or no. It blocks the agent until an answer or the timeout.
    Confirm {
        id: String,
        title: String,
        message: String,
        /// The agent-side timeout in milliseconds.
        ///
        /// It is not optional. The agent side owns every timeout, so a dialog always
        /// carries one. An optional field let a host opt out of the rule
        /// `D-a-dialog-timeout-cancels` states, and a dialog with no timeout outlived an
        /// abort: the abort cancelled the run token while this wait kept going, so the
        /// prompt could not settle until the client answered.
        timeout_ms: u64,
    },
    /// Free text input. It blocks the agent until an answer or the timeout.
    Input {
        id: String,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
        /// The agent-side timeout in milliseconds.
        ///
        /// It is not optional. The agent side owns every timeout, so a dialog always
        /// carries one. An optional field let a host opt out of the rule
        /// `D-a-dialog-timeout-cancels` states, and a dialog with no timeout outlived an
        /// abort: the abort cancelled the run token while this wait kept going, so the
        /// prompt could not settle until the client answered.
        timeout_ms: u64,
    },
    /// Display a message. Fire-and-forget: the client must not answer.
    ///
    /// Only [`Asker::notify`] builds one, and nothing in `crates/*/src` calls that. Both are
    /// host-only extension surface, and the doc comment on `notify` says why they stay.
    Notify { id: String, message: String },
}

impl DialogRequest {
    /// The dialog id a `DialogResponse` must carry to answer this request.
    pub fn id(&self) -> &str {
        match self {
            DialogRequest::Select { id, .. }
            | DialogRequest::Confirm { id, .. }
            | DialogRequest::Input { id, .. }
            | DialogRequest::Notify { id, .. } => id,
        }
    }

    /// True when the agent waits for an answer. `Notify` blocks nothing.
    pub fn blocks(&self) -> bool {
        !matches!(self, DialogRequest::Notify { .. })
    }

    /// The timeout the agent applies, when this method blocks.
    ///
    /// `Notify` blocks nothing, so it has none. Every other method has one, because the
    /// field is not optional.
    pub fn timeout_ms(&self) -> Option<u64> {
        match self {
            DialogRequest::Select { timeout_ms, .. }
            | DialogRequest::Confirm { timeout_ms, .. }
            | DialogRequest::Input { timeout_ms, .. } => Some(*timeout_ms),
            DialogRequest::Notify { .. } => None,
        }
    }
}

/// The answer in a `Command::DialogResponse`.
///
/// The wire shape is one of `{"value":"a"}`, `{"confirmed":true}`, or
/// `{"cancelled":true}`. Exactly one key. Zero keys is refused, and two or more
/// keys are refused. An untagged reader would take the first match in silence, and
/// a dialog answer decides whether a tool runs. See
/// D-a-dialog-answer-holds-exactly-one-value.
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

/// The wire shape of a dialog answer. It is private, because it is a shape and not
/// a type a caller should hold. `DialogAnswer` is the type a caller holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireAnswer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    confirmed: Option<bool>,
    /// It is `Option<True>` and not `Option<bool>`.
    ///
    /// `{"cancelled":false}` reads as "I did not cancel", and an `Option<bool>` would
    /// have read it as a cancellation, which denies a tool call. The one-value type
    /// refuses the literal instead, so the client hears about its own mistake.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cancelled: Option<True>,
}

impl From<DialogAnswer> for WireAnswer {
    fn from(answer: DialogAnswer) -> Self {
        let (value, confirmed, cancelled) = match answer {
            DialogAnswer::Value(value) => (Some(value), None, None),
            DialogAnswer::Confirmed(confirmed) => (None, Some(confirmed), None),
            DialogAnswer::Cancelled => (None, None, Some(True)),
        };
        Self {
            value,
            confirmed,
            cancelled,
        }
    }
}

/// Why a dialog answer is not readable. It is one case, because there is one rule.
#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
#[error(
    "a dialog answer holds exactly one of value, confirmed, or cancelled, and this one holds \
     {found}"
)]
pub struct AnswerError {
    /// How many of the three keys the object carried.
    pub found: usize,
}

impl TryFrom<WireAnswer> for DialogAnswer {
    type Error = AnswerError;

    fn try_from(wire: WireAnswer) -> Result<Self, Self::Error> {
        let found = usize::from(wire.value.is_some())
            + usize::from(wire.confirmed.is_some())
            + usize::from(wire.cancelled.is_some());
        if found != 1 {
            return Err(AnswerError { found });
        }
        if let Some(value) = wire.value {
            return Ok(DialogAnswer::Value(value));
        }
        if let Some(confirmed) = wire.confirmed {
            return Ok(DialogAnswer::Confirmed(confirmed));
        }
        Ok(DialogAnswer::Cancelled)
    }
}
