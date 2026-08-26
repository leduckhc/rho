//! The on-disk schema cache.
//!
//! An MCP server only reports its tools after `initialize` and `tools/list`,
//! which is a round trip against a process rho just spawned. So rho advertises
//! the tools from a cache at turn one, and connects on a background task. The
//! first provider request already carries the tools, so a late connection never
//! rewrites the stable prefix and never throws away the prompt cache. See
//! `SPEC-mcp` section 4.
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
        match self.entries.get(&Self::persisted_key(config)) {
            Some(entry) => &entry.tools,
            None => &[],
        }
    }

    /// The persisted key for a config.
    ///
    /// A fingerprint embeds every `env` entry, including a server token, so it is an
    /// in-memory key and never a file one. This hashes it, so the file names no secret and
    /// no variable. FNV-1a keeps the digest stable across builds, which a cache needs, and
    /// it carries no security claim beyond hiding the input.
    fn persisted_key(config: &McpServerConfig) -> String {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in config.fingerprint().as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{hash:016x}")
    }

    /// Replace the entry for a config with a live tool list.
    ///
    /// A live connection always wins over the cache. That is the second
    /// correctness guard.
    pub fn update(&mut self, config: &McpServerConfig, tools: Vec<McpToolDef>) {
        self.entries.insert(
            Self::persisted_key(config),
            CacheEntry {
                server: config.name.clone(),
                tools,
            },
        );
    }
}

/// Record one server's tools in the cache file, keeping every other entry.
///
/// The write is serialised by a lock file beside the cache, so the read, the update, and the
/// rename are one step. Without it the read-modify-write lost entries: a review ran eight
/// servers on eight threads and **one entry of eight survived, on every run**. That is the
/// same "no MCP tool reaches the model" class this whole change exists to fix, so a bounded
/// cost was the wrong answer.
///
/// A lock that cannot be taken is not an error worth failing a session over. rho retries
/// briefly and then gives up, because a cache is a hint and a live connection always wins.
pub fn record_tools(
    path: &Path,
    config: &McpServerConfig,
    tools: Vec<McpToolDef>,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock = path.with_extension("lock");
    let _guard = LockGuard::acquire(&lock)?;

    let mut cache = McpSchemaCache::load(path).unwrap_or_else(|_| McpSchemaCache::new());
    cache.update(config, tools);

    // A name keyed by the process id alone collided when two servers in one process finished
    // together, so the counter makes every write its own path. The lock above is what keeps
    // the entries, and this keeps the temporary files apart.
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let ticket = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temporary = path.with_extension(format!("tmp{}.{ticket}", std::process::id()));
    if let Err(error) = cache.save(&temporary) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

/// An advisory lock held by the existence of a file, released on drop.
///
/// `create_new` is atomic, so exactly one writer wins the race. A stale lock from a killed
/// process is broken after the timeout rather than blocking forever, because a cache must
/// never wedge a session.
struct LockGuard {
    path: std::path::PathBuf,
}

impl LockGuard {
    fn acquire(path: &Path) -> std::io::Result<Self> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2_000);
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(_) => {
                    return Ok(Self {
                        path: path.to_path_buf(),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if std::time::Instant::now() >= deadline {
                        // A lock nobody released, so take it. The alternative is a session
                        // that never records a schema again.
                        let _ = std::fs::remove_file(path);
                        continue;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
