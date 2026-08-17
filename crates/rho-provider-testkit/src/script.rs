//! The abstract response script.
//!
//! A `Script` names one canonical response. A [`crate::ProviderHarness`] turns a
//! script into that provider's wire format. The contract checks know the exact
//! content each script produces, so they can assert on the normalised events.

use serde_json::{Value, json};

/// The canonical assistant text answer. A text script produces this string.
pub const SCRIPT_TEXT: &str = "Hello";

/// The canonical tool name. A tool-call script requests this tool.
pub const SCRIPT_TOOL_NAME: &str = "get_weather";

/// The canonical parsed tool arguments. A tool-call script assembles this
/// object from fragments split across several chunks.
pub fn script_tool_arguments() -> Value {
    json!({ "city": "Paris", "unit": "celsius" })
}

/// One canonical response, expressed without a wire format.
///
/// The harness maps each variant to the provider's own bytes or events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    /// The assistant answers with text `SCRIPT_TEXT`, then ends the turn.
    /// The harness must split the text into at least two deltas.
    Text,
    /// The assistant requests one tool call. The harness must split the call
    /// across at least three chunks. It must split the JSON arguments
    /// mid-token, so the arguments parse only after concatenation.
    ToolCall,
    /// A text answer that also reports token usage on the final chunk.
    Usage,
    /// A text answer whose transport sends the first chunk, then holds the rest
    /// back. This proves a provider streams and does not buffer the whole body.
    Gated,
}
