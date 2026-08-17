//! The error type for the MCP client.
//!
//! Every message tells the user what to do next, and it names the server, so a
//! failure points at the server that caused it. See `SPEC-09` section 6.

/// An error from the MCP client, the pool, or one call.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("failed to start the MCP server {server}: {reason}. Check the command path.")]
    Launch { server: String, reason: String },

    #[error(
        "the handshake with the MCP server {server} failed: {reason}. \
         Check the server speaks MCP over this transport."
    )]
    Handshake { server: String, reason: String },

    #[error(
        "the MCP server {server} did not connect in time. \
         Check the server starts and answers `initialize`."
    )]
    ConnectTimeout { server: String },

    #[error(
        "the MCP server {server} did not answer the call in time. Try again, or raise call_timeout_ms."
    )]
    CallTimeout { server: String },

    #[error("the MCP server {server} is not available. It may have stopped. Start it again.")]
    Unavailable { server: String },

    #[error("the MCP server {server} sent an invalid message: {reason}.")]
    Protocol { server: String, reason: String },

    #[error(
        "the MCP server {server} reported the protocol version {version}, which rho does not \
         support. Update the server, or update rho."
    )]
    UnknownProtocolVersion { server: String, version: String },

    #[error(
        "the MCP server {server} sent a schema larger than the {limit} byte cap for the tool \
         {tool}. Refuse it. A vast schema is a denial of service against the context window."
    )]
    SchemaTooLarge {
        server: String,
        tool: String,
        limit: usize,
    },

    #[error(
        "the MCP server {server} sent the invalid tool name {name}: {reason}. \
         Refuse it. A tool name must be a safe registry key."
    )]
    InvalidToolName {
        server: String,
        name: String,
        reason: String,
    },

    #[error(
        "the tool name {name} is used by both the MCP server {first} and the MCP server \
         {second}. Rename one server so the final tool names differ."
    )]
    DuplicateToolName {
        name: String,
        first: String,
        second: String,
    },

    #[error(
        "the MCP server {server} uses the HTTP transport, but rho-mcp links no HTTP client. \
         Supply an HTTP transport with a TransportFactory."
    )]
    HttpTransportNotWired { server: String },

    #[error(
        "the cached tool {tool} on the MCP server {server} no longer exists. The server list changed. Reconnect."
    )]
    CachedToolGone { server: String, tool: String },

    #[error("an input or output error with the MCP server {server}: {reason}.")]
    Io { server: String, reason: String },
}
