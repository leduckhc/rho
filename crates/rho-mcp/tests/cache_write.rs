//! The schema cache is written, hashed, and merged. See `SPEC-wire-the-dead-switches`.
//!
//! `McpSchemaCache::save` had no caller anywhere, so the cache was never written, so
//! `tools_for` always advertised nothing, so no MCP tool ever reached the model. Every run
//! said the tools would arrive next session. See `docs/verification/mcp-live-probe.md`.

use rho_mcp::{McpSchemaCache, McpServerConfig, McpToolDef, McpTransport, record_tools};

fn server(name: &str, token: &str) -> McpServerConfig {
    McpServerConfig {
        name: name.to_string(),
        transport: McpTransport::Stdio {
            command: "true".to_string(),
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
    // The fingerprint embeds every `env` entry, including a token. As a persisted key it
    // would write that token to disk in clear text.
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
fn a_changed_config_does_not_serve_a_stale_list() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("cache.json");
    let before = server("search", "t1");
    record_tools(&path, &before, vec![tool("find")]).expect("write");

    // A different token is a different connection, so its tools are not this config's.
    let after = server("search", "t2");
    let cache = McpSchemaCache::load(&path).expect("loads");
    assert!(
        cache.tools_for(&after).is_empty(),
        "a changed config gets no stale entry"
    );
}
