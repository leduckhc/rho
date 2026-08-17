//! Usage counters and stop reasons for one provider turn.

use serde::{Deserialize, Serialize};

/// Cumulative token usage for one turn.
// `Eq` is absent on purpose. `cost_usd` is a float, and a float has no total equality.
// `PartialEq` is enough for a test, and nothing here needs a hash or a sort.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// The cost of the call in dollars, when the provider reports it.
    ///
    /// **Measured, never estimated.** A harness that multiplies tokens by a price table
    /// goes wrong whenever a price changes, a request falls back to another model, or a
    /// cached token is billed at a discount. OpenRouter returns the charged amount, so rho
    /// passes it through and leaves the field empty where a provider does not report one.
    /// See decision D-032.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

impl Usage {
    /// The share of input tokens that were served from the provider's cache.
    ///
    /// Returns `None` when there were no input tokens, so a caller cannot divide by zero
    /// and cannot mistake "no data" for "no cache hits".
    ///
    /// This number is the point of the append-only context rule in `SPEC-01` section 1.
    /// A competitor claims that discipline keeps the cache warm and publishes no
    /// hit rate. rho reports one.
    pub fn cache_hit_ratio(&self) -> Option<f64> {
        let total = self.input_tokens + self.cache_read_tokens;
        if total == 0 {
            return None;
        }
        Some(self.cache_read_tokens as f64 / total as f64)
    }

    /// Add another usage report to this one. Costs add, and a missing cost stays missing.
    ///
    /// A session sums usage across turns, so the addition must be explicit about the
    /// absent case: adding a report with no cost must not silently make the total look
    /// like zero.
    pub fn add(&mut self, other: &Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_write_tokens += other.cache_write_tokens;
        self.cost_usd = match (self.cost_usd, other.cost_usd) {
            (Some(a), Some(b)) => Some(a + b),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
    }
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
