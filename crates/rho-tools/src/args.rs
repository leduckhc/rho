//! Shared argument parsing for the built-in tools.

use rho_core::ToolError;
use serde::de::DeserializeOwned;

/// Parse tool arguments into a typed struct. A parse failure returns
/// `InvalidArguments` with a message that names the fault.
pub(crate) fn parse_args<T: DeserializeOwned>(args: serde_json::Value) -> Result<T, ToolError> {
    serde_json::from_value(args).map_err(|error| {
        ToolError::InvalidArguments(format!(
            "the arguments are not valid: {error}. Check the tool schema."
        ))
    })
}
