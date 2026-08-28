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
    /// Epoch milliseconds, as a decimal string, of the last write of this entry.
    ///
    /// Pruning an old entry needs a timestamp, and adding it now keeps the file
    /// at version 1. An old file has no such field, so it defaults to an empty
    /// string and still loads. No eviction bound exists yet; this is the field a
    /// bound will read. See A9 and decision D-one-timestamp-format.
    #[serde(default)]
    last_used: String,
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
    /// This hashes the config's full fingerprint, which includes each `env` value
    /// and each HTTP header value. An `env` value reaches the server as a process
    /// environment variable, so it can change the tool surface, not only a
    /// credential. `MODE=readonly` and `MODE=write` are two servers with two tool
    /// lists. The pool already spawns one process per fingerprint, so the cache
    /// keys on the same identity, or turn one advertises the wrong server's tools.
    ///
    /// The key is a hash, so no fingerprint text, and so no `env` value, ever
    /// reaches the file. A server token stays confidential in two ways at once: it
    /// is never written in clear, and A3 makes the file owner-only (`0o600` in a
    /// `0o700` directory). Residual: on a platform without unix permissions, a
    /// reader of the file could brute-force a low-entropy value against this
    /// digest, because the fingerprint format is public. rho accepts the same
    /// residual for its transcripts, which hold cleartext under the same `0o600`
    /// protection, so a hash of a token is strictly less exposure. FNV-1a keeps
    /// the digest stable across builds, which a cache needs. See A4 and A3.
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
                last_used: now_unix_ms().to_string(),
            },
        );
    }
}

/// The current unix time in milliseconds. A clock error yields zero, which is
/// harmless for a cache timestamp. Matches every other timestamp rho writes; see
/// decision D-one-timestamp-format.
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|delta| delta.as_millis() as u64)
        .unwrap_or(0)
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
        // The cache names every configured server. A world-readable directory
        // leaks which servers a user runs, so the directory is owner-only. rho
        // sets 0o700 on its own directories; see rho-core `transcript.rs`. See A3.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
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
    // Set owner-only on the temporary before the rename, so the cache file is
    // never briefly world-readable. The rename keeps the mode. See A3.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) =
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))
        {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
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
///
/// Each guard writes an unguessable nonce into the lock file. A steal breaks only the exact
/// stale lock it watched, and `Drop` removes the file only when it still holds this guard's
/// nonce. Without the nonce, a steal deleted a successor's fresh lock and `Drop` deleted
/// whatever file was there, so two overlapping read-modify-writes lost an entry. See A2.
struct LockGuard {
    path: std::path::PathBuf,
    nonce: String,
}

/// How long a held lock may sit unchanged before a waiter breaks it. A killed process must
/// not wedge the cache forever, and a live write finishes in milliseconds.
const LOCK_STEAL_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(2_000);

impl LockGuard {
    fn acquire(path: &Path) -> std::io::Result<Self> {
        Self::acquire_within(path, LOCK_STEAL_TIMEOUT)
    }

    /// Acquire the lock, breaking a lock that sits unchanged for `steal_after`.
    ///
    /// The timeout is a parameter so a test can drive the steal path without a real wait.
    fn acquire_within(path: &Path, steal_after: std::time::Duration) -> std::io::Result<Self> {
        use std::io::Write;
        let nonce = Self::new_nonce();
        // The nonce we have watched holding the lock, and when we first saw it. We steal
        // only when the same nonce persists for the whole timeout, so a successor that
        // replaces the lock resets the clock and is never deleted from under its owner.
        let mut watched: Option<String> = None;
        let mut watched_since = std::time::Instant::now();
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(mut file) => {
                    file.write_all(nonce.as_bytes())?;
                    file.flush()?;
                    return Ok(Self {
                        path: path.to_path_buf(),
                        nonce,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let current = std::fs::read_to_string(path).ok();
                    if current != watched {
                        // A different holder took the lock. Watch it afresh, so a lock that
                        // keeps changing is never stolen.
                        watched = current;
                        watched_since = std::time::Instant::now();
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }
                    if watched_since.elapsed() >= steal_after {
                        // The same lock has sat unchanged past the timeout. Break it, but
                        // only if it still holds the exact bytes we watched, so we never
                        // delete a successor's fresh lock.
                        if let Some(seen) = watched.as_deref() {
                            Self::remove_if_matches(path, seen);
                        }
                        watched = None;
                        watched_since = std::time::Instant::now();
                        continue;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// An unguessable lock nonce. Two writes never collide, and a killed process
    /// leaves a nonce no successor can hold.
    fn new_nonce() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        // The process id, a per-process counter, and the wall clock in nanoseconds. No two
        // live writers, in one process or across processes, produce the same nonce.
        format!("{}-{}-{}", std::process::id(), count, nanos)
    }

    /// Remove the file only when its contents still equal `expected`.
    ///
    /// There is no atomic compare-and-delete in `std`, so this reads then removes. The
    /// window is small, and the check stops the cascade where one writer deletes another's
    /// lock. See A2.
    fn remove_if_matches(path: &Path, expected: &str) {
        if std::fs::read_to_string(path).ok().as_deref() == Some(expected) {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // Remove the lock only when it still holds our nonce. A successor that stole the
        // lock wrote its own nonce, so we must not delete its lock. See A2.
        Self::remove_if_matches(&self.path, &self.nonce);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // ---- A2 and A8: the lock nonce and the steal path. --------------------

    #[test]
    fn a_stale_lock_is_stolen_after_the_timeout() {
        // A killed process left a lock behind. A waiter must break it, or the cache never
        // records a schema again. The stolen lock ends up holding our nonce, not the dead
        // one, so we truly own it.
        let dir = tempfile::tempdir().expect("a temp dir");
        let lock = dir.path().join("cache.lock");
        std::fs::write(&lock, "dead-process-nonce").expect("plant a stale lock");

        let guard = LockGuard::acquire_within(&lock, Duration::from_millis(20))
            .expect("the stale lock is stolen");
        let held = std::fs::read_to_string(&lock).expect("the lock is held");
        assert_ne!(held, "dead-process-nonce", "we replaced the stale nonce");
        assert_eq!(held, guard.nonce, "the lock holds our nonce");

        drop(guard);
        assert!(!lock.exists(), "our own lock is released on drop");
    }

    #[test]
    fn drop_does_not_remove_a_successors_lock() {
        // The original defect: `Drop` removed whatever file was there. If a successor stole
        // the lock and wrote its own nonce, the first guard's `Drop` deleted the
        // successor's lock, so two writers ran at once and one entry was lost.
        let dir = tempfile::tempdir().expect("a temp dir");
        let lock = dir.path().join("cache.lock");
        let guard = LockGuard::acquire_within(&lock, LOCK_STEAL_TIMEOUT).expect("acquire");

        // A successor stole the lock and wrote its own nonce.
        std::fs::write(&lock, "successor-nonce").expect("successor takes over");

        drop(guard);
        assert!(lock.exists(), "drop must not remove a successor's lock");
        assert_eq!(
            std::fs::read_to_string(&lock).expect("read"),
            "successor-nonce",
            "the successor's lock is untouched"
        );
    }

    #[test]
    fn a_lock_that_keeps_changing_is_not_stolen() {
        // A steal breaks only a lock that sits unchanged for the timeout. A lock whose
        // holder keeps rewriting it is alive, so the waiter must keep waiting, not steal.
        let dir = tempfile::tempdir().expect("a temp dir");
        let lock = dir.path().join("cache.lock");
        std::fs::write(&lock, "nonce-0").expect("plant a lock");

        let path = lock.clone();
        let live = std::thread::spawn(move || {
            // Rewrite the lock every 5 ms for 150 ms, so it never sits still.
            for round in 1..=30 {
                std::fs::write(&path, format!("nonce-{round}")).expect("rewrite");
                std::thread::sleep(Duration::from_millis(5));
            }
        });

        // Steal-after 40 ms. The lock changes faster than that, so no steal happens while
        // the holder is alive. Acquire must not succeed until the holder stops.
        let started = std::time::Instant::now();
        let guard = LockGuard::acquire_within(&lock, Duration::from_millis(40)).expect("acquire");
        assert!(
            started.elapsed() >= Duration::from_millis(140),
            "acquire waited for the live holder to stop, not stole it early"
        );
        live.join().expect("the holder thread finished");
        drop(guard);
    }

    // ---- A8: the record_tools rollback arm. -------------------------------

    #[test]
    fn a_rename_failure_cleans_up_the_temporary_and_errors() {
        // Make the destination a directory, so the rename cannot succeed. The rollback arm
        // must remove the temporary file and return the error, leaving no stray temp behind.
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("cache.json");
        std::fs::create_dir(&path).expect("the destination is a directory");

        let config = McpServerConfig {
            name: "srv".to_string(),
            transport: crate::config::McpTransport::Stdio {
                command: "true".to_string(),
                args: Vec::new(),
            },
            env: Default::default(),
            shared: true,
            call_timeout_ms: None,
        };
        let tool = McpToolDef {
            name: "find".to_string(),
            description: String::new(),
            input_schema: serde_json::json!({ "type": "object" }),
        };

        let result = record_tools(&path, &config, vec![tool]);
        assert!(result.is_err(), "a rename onto a directory is an error");

        let leftover_temp = std::fs::read_dir(dir.path())
            .expect("read the dir")
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains("tmp"));
        assert!(!leftover_temp, "the temporary file is removed on failure");
    }

    // ---- A9: the entry timestamp. -----------------------------------------

    #[test]
    fn an_updated_entry_records_a_write_timestamp() {
        let config = McpServerConfig {
            name: "srv".to_string(),
            transport: crate::config::McpTransport::Stdio {
                command: "true".to_string(),
                args: Vec::new(),
            },
            env: Default::default(),
            shared: true,
            call_timeout_ms: None,
        };
        let mut cache = McpSchemaCache::new();
        cache.update(&config, Vec::new());

        let key = McpSchemaCache::persisted_key(&config);
        let entry = cache.entries.get(&key).expect("the entry exists");
        let stamp: u64 = entry
            .last_used
            .parse()
            .expect("last_used is a decimal millisecond stamp");
        assert!(stamp > 0, "the entry records a real write time");
    }

    #[test]
    fn an_old_file_without_a_timestamp_still_loads() {
        // The field is `#[serde(default)]`, so a version 1 file written before A9 still
        // loads rather than being discarded as unreadable.
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("cache.json");
        let old = r#"{"version":1,"entries":{"abc":{"server":"srv","tools":[]}}}"#;
        std::fs::write(&path, old).expect("write an old file");

        let cache = McpSchemaCache::load(&path).expect("an old file still loads");
        assert_eq!(cache.entries.len(), 1, "the old entry survives the load");
    }
}
