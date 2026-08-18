//! The limits that bound one server and one call.
//!
//! Every limit is per call, not per session. A server is shared between
//! sessions, so a limit that was per session would let one session set the bound
//! for another. See decision D-mcp-shared-by-default.

/// The safety and timeout bounds for the MCP client.
#[derive(Clone, Copy, Debug)]
pub struct McpLimits {
    /// The largest time one `tools/call` may take, in milliseconds.
    pub call_timeout_ms: u64,
    /// The largest time the handshake may take, in milliseconds.
    pub connect_timeout_ms: u64,
    /// The largest single line the client reads from a server, in bytes.
    pub max_line_bytes: usize,
    /// The largest tool output the client keeps, in bytes.
    pub max_output_bytes: usize,
    /// The largest tool input schema the client accepts, in bytes.
    pub max_schema_bytes: usize,
    /// The largest number of tools one server may advertise.
    pub max_tools_per_server: usize,
}

impl Default for McpLimits {
    fn default() -> Self {
        Self {
            call_timeout_ms: 30_000,
            connect_timeout_ms: 10_000,
            max_line_bytes: 1_000_000,
            max_output_bytes: 1_000_000,
            max_schema_bytes: 100_000,
            max_tools_per_server: 256,
        }
    }
}
