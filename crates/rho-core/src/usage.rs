//! Usage counters and stop reasons for one provider turn.

use serde::{Deserialize, Serialize};

/// Cumulative token usage for one turn.
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
