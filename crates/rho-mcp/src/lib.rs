//! `rho-mcp` is a Model Context Protocol client for rho.
//!
//! MCP is how the rest of the world ships agent tools. This crate connects to an
//! MCP server over stdio, or over an HTTP transport a caller supplies, runs the
//! handshake, lists the tools, and serves each call. It advertises tools from an
//! on-disk schema cache at turn one, so a late connection never rewrites the
//! stable prompt prefix.
//!
//! A server is shared between sessions by default, keyed by the config
//! fingerprint and reference counted, because a server is a whole process and
//! rho runs many sessions at once. See `SPEC-mcp`.

mod cache;
mod client;
mod config;
mod error;
mod limits;
mod line;
mod naming;
mod pool;
mod sanitize;
mod tool;
mod transport;

pub use cache::McpSchemaCache;
pub use client::McpClient;
pub use config::{McpServerConfig, McpToolDef, McpTransport};
pub use error::McpError;
pub use limits::McpLimits;
pub use line::LineOutcome;
pub use naming::{dispatch_name, validate_tool_name};
pub use pool::{McpHandle, McpPool};
pub use sanitize::sanitize_output;
pub use tool::{McpTool, tools_for};
pub use transport::{
    DefaultTransportFactory, TransportFactory, TransportPair, TransportReader, TransportWriter,
};
