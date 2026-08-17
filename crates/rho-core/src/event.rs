//! The normalised streaming event.
//!
//! Every provider emits this one event type. Every frontend consumes it. The
//! provider maps its own wire format onto these variants.

use crate::{Role, Usage};
use serde::{Deserialize, Serialize};

use crate::StopReason;

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
    ThinkingEnd {
        index: u32,
        signature: Option<String>,
    },
    /// A tool call begins. The name is known at the start.
    ToolCallStart { index: u32, id: String, name: String },
    /// A raw JSON fragment of the tool-call arguments.
    ToolCallDelta { index: u32, delta: String },
    /// The tool call is complete. `arguments` is the parsed JSON object.
    ToolCallEnd {
        index: u32,
        arguments: serde_json::Value,
    },
    /// Cumulative token usage, reported one or more times.
    Usage(Usage),
    /// The turn is done. This is the last event of a successful turn.
    Done { stop_reason: StopReason },
}
