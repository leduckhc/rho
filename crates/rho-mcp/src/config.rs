//! The server config, the transport descriptor, and the tool definition.
//!
//! The config is the public shape of one server. Its fingerprint keys the shared
//! pool and the schema cache. Change any field and the fingerprint changes, so a
//! stale cache entry is ignored. See `SPEC-mcp` sections 4 and 7.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The serde default for `shared`. A server is shared unless it says otherwise.
fn default_true() -> bool {
    true
}

/// How to reach one server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// The user's name for the server. It is trusted, and it prefixes every tool.
    pub name: String,
    pub transport: McpTransport,
    /// Extra environment for a stdio server. Added after the credential scrub, so a
    /// server that needs a token names it here.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// False keeps this server private to one session. Default true.
    #[serde(default = "default_true")]
    pub shared: bool,
    #[serde(default)]
    pub call_timeout_ms: Option<u64>,
}

/// The transport for one server. The tag `type` selects the variant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}

/// A tool as a server described it. Its `name` and `input_schema` are untrusted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct McpToolDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

impl McpServerConfig {
    /// A stable fingerprint of the config.
    ///
    /// The fingerprint keys the pool and the cache. It covers the command, the
    /// arguments, the environment, the transport, the URL, and the shared flag.
    /// A `BTreeMap` iterates in sorted order, so the same config always produces
    /// the same text. The text is the key, so the match is exact and stable
    /// across runs of the same binary.
    pub fn fingerprint(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.push(format!("shared={}", self.shared));
        for (key, value) in &self.env {
            parts.push(format!("env:{key}={value}"));
        }
        match &self.transport {
            McpTransport::Stdio { command, args } => {
                parts.push(format!("stdio:command={command}"));
                for (index, arg) in args.iter().enumerate() {
                    parts.push(format!("stdio:arg{index}={arg}"));
                }
            }
            McpTransport::Http { url, headers } => {
                parts.push(format!("http:url={url}"));
                for (key, value) in headers {
                    parts.push(format!("http:header:{key}={value}"));
                }
            }
        }
        parts.join("\u{1f}")
    }
}
