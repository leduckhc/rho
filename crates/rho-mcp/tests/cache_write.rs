//! The schema cache is written, hashed, and merged. See `SPEC-wire-the-dead-switches`.
//!
//! `McpSchemaCache::save` had no caller anywhere, so the cache was never written, so
//! `tools_for` always advertised nothing, so no MCP tool ever reached the model. Every run
//! said the tools would arrive next session. See `docs/verification/mcp-live-probe.md`.

mod common;

use rho_mcp::{McpSchemaCache, McpServerConfig, McpToolDef, McpTransport, record_tools};

fn server(name: &str, token: &str) -> McpServerConfig {
    McpServerConfig {
        name: name.to_string(),
        transport: McpTransport::Stdio {
            // The command is the server's non-secret identity. Two different servers run
            // different commands, so a distinct command gives each its own cache key. The
            // server name is a label and is not part of the cache identity. See A4.
            command: name.to_string(),
            args: Vec::new(),
        },
        env: [("SECRET_TOKEN".to_string(), token.to_string())]
            .into_iter()
            .collect(),
        shared: true,
        call_timeout_ms: None,
    }
}

fn tool(name: &str) -> McpToolDef {
    McpToolDef {
        name: name.to_string(),
        description: "a tool".to_string(),
        input_schema: serde_json::json!({"type": "object"}),
    }
}

#[test]
fn a_recorded_tool_list_is_read_back() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("cache.json");
    let config = server("search", "t1");

    record_tools(&path, &config, vec![tool("find")]).expect("the write succeeds");

    let cache = McpSchemaCache::load(&path).expect("the file loads");
    let tools = cache.tools_for(&config);
    assert_eq!(tools.len(), 1, "the entry is read back");
    assert_eq!(tools[0].name, "find");
}

#[test]
fn a_second_server_does_not_clobber_the_first() {
    // Two rho sessions, or two servers in one process, write the same file. A whole-file
    // overwrite from an in-memory snapshot loses the other entry.
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("cache.json");
    let first = server("first", "t1");
    let second = server("second", "t2");

    record_tools(&path, &first, vec![tool("one")]).expect("write one");
    record_tools(&path, &second, vec![tool("two")]).expect("write two");

    let cache = McpSchemaCache::load(&path).expect("loads");
    assert_eq!(cache.tools_for(&first).len(), 1, "the first entry survives");
    assert_eq!(cache.tools_for(&second).len(), 1, "the second is there too");
}

#[test]
fn the_cache_file_holds_no_server_secret() {
    // The cache key is a hash of the config fingerprint, so no `env` value and no header
    // value ever reaches the file in clear. The token is fed to the hash, but a hash is not
    // reversible, and A3 makes the file owner-only, so the digest stays confidential. This
    // pins that no cleartext secret, and not even the variable name, lands in the file.
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("cache.json");
    let config = server("search", "super-secret-token");

    record_tools(&path, &config, vec![tool("find")]).expect("write");

    let text = std::fs::read_to_string(&path).expect("read");
    assert!(
        !text.contains("super-secret-token"),
        "a persisted key must be hashed: {text}"
    );
    assert!(
        !text.contains("SECRET_TOKEN"),
        "nor may it name the variable: {text}"
    );
}

#[test]
fn a_changed_command_does_not_serve_a_stale_list() {
    // The cache key covers non-secret identity. A changed command is a different server, so
    // its entry does not match, and no stale tool is served.
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("cache.json");
    let before = server("search", "t1");
    record_tools(&path, &before, vec![tool("find")]).expect("write");

    let after = server("search-v2", "t1");
    let cache = McpSchemaCache::load(&path).expect("loads");
    assert!(
        cache.tools_for(&after).is_empty(),
        "a changed command gets no stale entry"
    );
}

#[test]
fn an_env_value_change_does_not_serve_a_stale_list() {
    // The restored invariant. An `env` value reaches the server as a process environment
    // variable (see rho-mcp `transport.rs`), so it can change the tool surface, not only a
    // credential. `MODE=readonly` and `MODE=write` are two different servers with two
    // different tool lists. The cache key must depend on the whole config, values included,
    // or turn one advertises the wrong server's tools to the model. That is the same "wrong
    // tools reach the model" defect this branch exists to fix.
    //
    // The key cannot tell a semantic change (`MODE`) from a credential rotation (a token),
    // because both are only `env` values. So it treats both as a new server. The cost is
    // one cold cache at turn one after a token rotation, which the live connection corrects
    // at once. That cost is acceptable; serving a stale tool list is not. The token stays
    // confidential because the key is a hash and the file is owner-only; see
    // `the_cache_file_holds_no_server_secret` and `the_cache_file_is_owner_only_on_unix`.
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("cache.json");

    let mut before = server("search", "t1");
    before
        .env
        .insert("MODE".to_string(), "readonly".to_string());
    record_tools(&path, &before, vec![tool("find")]).expect("write");

    let mut after = server("search", "t1");
    after.env.insert("MODE".to_string(), "write".to_string());
    let cache = McpSchemaCache::load(&path).expect("loads");
    assert!(
        cache.tools_for(&after).is_empty(),
        "a changed env value is a different tool surface, so it gets no stale entry"
    );
}

// ---- The production path, not the helper. ---------------------------------

#[tokio::test]
async fn a_handshake_through_the_pool_writes_the_cache() {
    // Every test above calls `record_tools` directly, so deleting the call in
    // `spawn_connect`'s `Ok` arm, or the `set_cache_path` call in the CLI, would restore the
    // defect this sprint exists to fix and leave the suite green. A test review and an
    // external review both said so. This drives the pool.
    use rho_mcp::{McpLimits, McpPool, tools_for};
    use std::sync::Arc;

    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("cache.json");
    let pool = McpPool::with_factory(McpLimits::default(), Arc::new(common::OneToolFactory));
    pool.set_cache_path(path.clone());

    let server = common::stub_config("probe", "ok");
    let cache = McpSchemaCache::new();
    tools_for(&pool, std::slice::from_ref(&server), &cache)
        .await
        .expect("tools_for returns at once");

    // The handshake runs on a background task, so wait for the file rather than assume it.
    for _ in 0..50 {
        if path.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        path.exists(),
        "a successful handshake through the pool must write the cache"
    );
    let written = McpSchemaCache::load(&path).expect("the file loads");
    assert_eq!(
        written.tools_for(&server).len(),
        1,
        "the writer's key must equal the reader's key, and the one listed tool is read back"
    );
    assert_eq!(
        written.tools_for(&server)[0].name,
        "probe_tool",
        "the read-back entry is the tool the server listed"
    );
    let text = std::fs::read_to_string(&path).expect("read");
    assert!(text.contains("probe"), "the entry names the server: {text}");
}

#[test]
fn eight_concurrent_writers_keep_every_entry() {
    // A review ran this and one entry of eight survived, every run. The read-modify-write
    // held no lock, so each writer overwrote the others. A bounded "one extra handshake" was
    // the wrong description: a multi-server user's cache never converged.
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("cache.json");
    let servers: Vec<McpServerConfig> = (0..8).map(|i| server(&format!("srv{i}"), "t")).collect();

    // **This test does not prove the lock is necessary, and it must not be read as if it
    // did.** Removing `LockGuard::acquire` from `record_tools` leaves it green on this
    // machine, with a barrier and eight rounds: the read, the write, and the rename finish
    // inside one scheduling quantum, so the writers serialise by luck.
    //
    // A concurrency review demonstrated the loss, eight servers on eight threads with one
    // entry surviving on every run, so the lock stays. Its necessity rests on that
    // demonstration and on the shape of a read-modify-write, not on this test. What this
    // test does prove is that concurrent writers still produce a readable file with every
    // entry present, which is worth having and is less than it looks.
    for round in 0..8 {
        let _ = std::fs::remove_file(&path);
        let start = std::sync::Arc::new(std::sync::Barrier::new(servers.len()));
        std::thread::scope(|scope| {
            for config in &servers {
                let path = path.clone();
                let start = std::sync::Arc::clone(&start);
                scope.spawn(move || {
                    start.wait();
                    record_tools(&path, config, vec![tool("find")]).expect("the write succeeds");
                });
            }
        });

        let cache = McpSchemaCache::load(&path).expect("the file loads");
        for config in &servers {
            assert_eq!(
                cache.tools_for(config).len(),
                1,
                "round {round}: every entry must survive, and {} is missing",
                config.name
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn the_cache_file_is_owner_only_on_unix() {
    // A3: the cache names every configured server, so it is owner-only, like every other
    // file rho writes. rho-core `transcript.rs` sets 0o600 on the file and 0o700 on the
    // directory, and pins it with a test; this matches it.
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("a temp dir");
    let cache_dir = dir.path().join("rho");
    let path = cache_dir.join("cache.json");
    let config = server("search", "t1");

    record_tools(&path, &config, vec![tool("find")]).expect("write");

    let file_mode = std::fs::metadata(&path)
        .expect("stat the file")
        .permissions()
        .mode();
    assert_eq!(file_mode & 0o777, 0o600, "the cache file is owner-only");
    let dir_mode = std::fs::metadata(&cache_dir)
        .expect("stat the dir")
        .permissions()
        .mode();
    assert_eq!(dir_mode & 0o777, 0o700, "the cache directory is owner-only");
}

#[tokio::test]
async fn a_failed_cache_write_surfaces_a_notice() {
    // A6: a cache write failure once reached only `tracing::debug!`, then the connection
    // went ready, so the user was told the tools arrive next session, forever. The pool now
    // records a notice the caller can show. A file where the parent directory cannot be
    // created makes the write fail.
    use rho_mcp::{McpLimits, McpPool, tools_for};
    use std::sync::Arc;

    let dir = tempfile::tempdir().expect("a temp dir");
    // A regular file blocks a directory of the same name, so `create_dir_all` fails.
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"not a directory").expect("write the blocker");
    let path = blocker.join("cache.json");

    let pool = McpPool::with_factory(McpLimits::default(), Arc::new(common::OneToolFactory));
    pool.set_cache_path(path);

    let server = common::stub_config("probe", "ok");
    let cache = McpSchemaCache::new();
    tools_for(&pool, std::slice::from_ref(&server), &cache)
        .await
        .expect("tools_for returns at once");

    // Draining the connect tasks makes the background write finish, so the notice is ready.
    pool.drain_connects(std::time::Duration::from_secs(5)).await;

    let notices = pool.take_cache_notices();
    assert_eq!(notices.len(), 1, "a failed cache write surfaces one notice");
    assert!(
        notices[0].contains("probe"),
        "the notice names the server: {}",
        notices[0]
    );
}
