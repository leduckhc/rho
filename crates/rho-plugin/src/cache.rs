//! The plugin schema cache (F-plugin-schema-cache).
//!
//! A plugin's tool schemas come from an on-disk cache. So the host advertises the
//! tools in the very first provider request, and a late plugin connection never
//! invalidates the prompt cache. Sprint 1 defines the cache shape and reads it.

use rho_core::ToolKind;
use serde::{Deserialize, Serialize};

/// One tool a plugin exposes, as advertised in the handshake or the cache.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginToolSpec {
    /// The tool name the model calls.
    pub name: String,
    /// A short description for the model.
    pub description: String,
    /// The ACP tool category. It maps to `ToolKind`.
    pub kind: ToolKind,
    /// A JSON Schema object for the arguments. The wire name is `inputSchema`.
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// The on-disk cache file for one plugin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginCache {
    /// The cache format version.
    pub version: u32,
    /// The plugin name.
    pub plugin: String,
    /// The plugin tools.
    pub tools: Vec<PluginToolSpec>,
}

impl PluginCache {
    /// Parse a cache from its JSON bytes.
    pub fn from_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    /// Serialise the cache to JSON bytes.
    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }
}
