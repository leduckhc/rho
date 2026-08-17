//! Tool naming.
//!
//! An MCP tool is namespaced, because two servers may both expose `search`. The
//! dispatch name is `mcp__<server>__<tool>`. A hyphen becomes an underscore,
//! since some providers reject a hyphen in a tool name. A tool name from a
//! server is untrusted, so it is validated before it becomes a registry key. See
//! `SPEC-09` sections 5 and 6.

use crate::error::McpError;

/// The prefix that marks a tool as an MCP tool.
const MCP_PREFIX: &str = "mcp";
/// The separator between the prefix, the server, and the tool.
const SEPARATOR: &str = "__";

/// Validate a tool name that a server reported.
///
/// The name becomes a registry key, so it must be safe. A name with a path
/// separator, a control character, or a leading dot is refused. An empty name is
/// refused. The check runs before the name reaches the registry.
pub fn validate_tool_name(server: &str, name: &str) -> Result<(), McpError> {
    let refuse = |reason: &str| {
        Err(McpError::InvalidToolName {
            server: server.to_string(),
            name: name.to_string(),
            reason: reason.to_string(),
        })
    };
    if name.is_empty() {
        return refuse("the name is empty");
    }
    if name.starts_with('.') {
        return refuse("the name starts with a dot");
    }
    if name.contains('/') || name.contains('\\') {
        return refuse("the name holds a path separator");
    }
    if name.chars().any(|c| c.is_control()) {
        return refuse("the name holds a control character");
    }
    Ok(())
}

/// Build the dispatch name for one tool.
///
/// The server name and the tool name both replace a hyphen with an underscore,
/// so the final name never holds a hyphen.
pub fn dispatch_name(server: &str, tool: &str) -> String {
    format!(
        "{}{}{}{}{}",
        MCP_PREFIX,
        SEPARATOR,
        replace_hyphen(server),
        SEPARATOR,
        replace_hyphen(tool)
    )
}

/// Replace every hyphen in `input` with an underscore.
fn replace_hyphen(input: &str) -> String {
    input.replace('-', "_")
}
