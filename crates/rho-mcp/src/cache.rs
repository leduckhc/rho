//! The on-disk schema cache.
//!
//! An MCP server only reports its tools after `initialize` and `tools/list`,
//! which is a round trip against a process rho just spawned. So rho advertises
//! the tools from a cache at turn one, and connects on a background task. The
//! first provider request already carries the tools, so a late connection never
//! rewrites the stable prefix and never throws away the prompt cache. See
//! `SPEC-09` section 4.
//!
//! Two correctness guards, and they are not optional:
//! - The cache entry is keyed by the config fingerprint, so a reconfigured
//!   server never advertises a stale tool.
//! - The cache is a hint, never truth. A live list always replaces the entry.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::{McpServerConfig, McpToolDef};

/// The cache format version. It sits in the first field, so a future change can
/// reject an old file rather than misread it.
const CACHE_VERSION: u32 = 1;

/// One cache entry: the tools a server reported last time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct CacheEntry {
    /// The server name at the time of caching. For a status line only.
    server: String,
    /// The tools the server reported.
    tools: Vec<McpToolDef>,
}

/// The on-disk cache of tool schemas, keyed by config fingerprint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct McpSchemaCache {
    version: u32,
    entries: HashMap<String, CacheEntry>,
}

impl Default for McpSchemaCache {
    fn default() -> Self {
        Self::new()
    }
}

impl McpSchemaCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self {
            version: CACHE_VERSION,
            entries: HashMap::new(),
        }
    }

    /// Load a cache from a JSON file.
    ///
    /// A missing file is not an error; it yields an empty cache, because a first
    /// run has no cache yet. A file with a different version yields an empty
    /// cache, because the shape may have changed.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<Self>(&bytes) {
                Ok(cache) if cache.version == CACHE_VERSION => Ok(cache),
                _ => Ok(Self::new()),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(error) => Err(error),
        }
    }

    /// Save the cache to a JSON file. The parent directory must exist.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self)?;
        std::fs::write(path, bytes)
    }

    /// The cached tools for a config, or an empty slice when the fingerprint has
    /// no entry.
    ///
    /// A config change alters the fingerprint, so a stale entry is never
    /// returned. That is the first correctness guard.
    pub fn tools_for(&self, config: &McpServerConfig) -> &[McpToolDef] {
        match self.entries.get(&config.fingerprint()) {
            Some(entry) => &entry.tools,
            None => &[],
        }
    }

    /// Replace the entry for a config with a live tool list.
    ///
    /// A live connection always wins over the cache. That is the second
    /// correctness guard.
    pub fn update(&mut self, config: &McpServerConfig, tools: Vec<McpToolDef>) {
        self.entries.insert(
            config.fingerprint(),
            CacheEntry {
                server: config.name.clone(),
                tools,
            },
        );
    }
}
