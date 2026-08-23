//! Schema cache tests for `rho-mcp`.
//!
//! rho advertises tools from an on-disk cache at turn one, so a late connection
//! never rewrites the stable prompt prefix. The entry is keyed by the config
//! fingerprint, and a live list always wins. See `SPEC-mcp` section 4.

mod common;

use std::sync::Arc;

use common::{FakeFactory, stub_config, test_limits};
use rho_mcp::{
    McpClient, McpPool, McpSchemaCache, McpServerConfig, McpToolDef, McpTransport, tools_for,
};

fn config(name: &str, command: &str) -> McpServerConfig {
    McpServerConfig {
        name: name.to_string(),
        transport: McpTransport::Stdio {
            command: command.to_string(),
            args: vec![],
        },
        env: Default::default(),
        shared: true,
        call_timeout_ms: None,
    }
}

fn tool_def(name: &str) -> McpToolDef {
    McpToolDef {
        name: name.to_string(),
        description: "A tool.".to_string(),
        input_schema: serde_json::json!({ "type": "object" }),
    }
}

#[tokio::test]
async fn tools_are_advertised_from_the_cache_before_any_connection() {
    // The cache holds a tool. `tools_for` advertises it before any connection.
    let pool = McpPool::with_factory(test_limits(), Arc::new(FakeFactory));
    let server = config("srv", "srv");
    let mut cache = McpSchemaCache::new();
    cache.update(&server, vec![tool_def("search")]);

    let tools = tools_for(&pool, std::slice::from_ref(&server), &cache)
        .await
        .unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert_eq!(names, vec!["mcp__srv__search"], "advertised from the cache");
}

#[tokio::test]
async fn tools_for_returns_without_waiting_for_a_handshake() {
    // A blocking factory never finishes the handshake. `tools_for` must still
    // return at once. A bounded timeout fails a blocking implementation.
    use async_trait::async_trait;
    use rho_mcp::{McpError, McpLimits, TransportFactory, TransportPair};

    struct BlockingFactory;
    #[async_trait]
    impl TransportFactory for BlockingFactory {
        async fn open(
            &self,
            _config: &McpServerConfig,
            _limits: &McpLimits,
        ) -> Result<TransportPair, McpError> {
            std::future::pending::<()>().await;
            unreachable!()
        }
    }

    let pool = McpPool::with_factory(test_limits(), Arc::new(BlockingFactory));
    let server = config("srv", "srv");
    let mut cache = McpSchemaCache::new();
    cache.update(&server, vec![tool_def("search")]);

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tools_for(&pool, std::slice::from_ref(&server), &cache),
    )
    .await;
    assert!(result.is_ok(), "tools_for must not wait for a handshake");
    assert_eq!(result.unwrap().unwrap().len(), 1);
}

#[test]
fn a_config_change_invalidates_the_cached_entry() {
    // The entry is keyed by the fingerprint. A command change alters the
    // fingerprint, so the old entry no longer matches.
    let mut cache = McpSchemaCache::new();
    let before = config("srv", "old-command");
    cache.update(&before, vec![tool_def("old")]);
    assert_eq!(cache.tools_for(&before).len(), 1);

    let after = config("srv", "new-command");
    assert!(
        cache.tools_for(&after).is_empty(),
        "a reconfigured server does not advertise a stale tool"
    );
}

#[tokio::test]
async fn a_live_connection_replaces_the_cached_entry() {
    // The cache is a hint, never truth. A live list always wins.
    let server = stub_config("stub", "normal");
    let mut cache = McpSchemaCache::new();
    cache.update(&server, vec![tool_def("stale")]);
    assert_eq!(cache.tools_for(&server)[0].name, "stale");

    let (client, live_tools) = McpClient::connect(&server, test_limits()).await.unwrap();
    cache.update(&server, live_tools);
    let names: Vec<&str> = cache
        .tools_for(&server)
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(names, vec!["echo"], "the live list replaces the cache");
    client.shutdown().await;
}

#[tokio::test]
async fn calling_a_cached_tool_that_no_longer_exists_is_a_clear_error() {
    // The cache advertises `ghost`, but the live server does not have it. A call
    // to it must be a clear error result, not a panic.
    let pool = McpPool::new(test_limits());
    let server = stub_config("stub", "normal");
    let mut cache = McpSchemaCache::new();
    cache.update(&server, vec![tool_def("ghost")]);

    let tools = tools_for(&pool, std::slice::from_ref(&server), &cache)
        .await
        .unwrap();
    let ghost = tools
        .iter()
        .find(|t| t.name() == "mcp__stub__ghost")
        .expect("the cached tool is advertised");

    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let ctx = rho_core::ToolContext {
        session_root: std::env::temp_dir(),
        cancel: rho_core::CancelToken::new(),
        updates: tx,
        agent_events: {
            let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(16);
            agent_tx
        },
    };
    let output = ghost.execute(serde_json::json!({}), ctx).await.unwrap();
    assert!(output.is_error, "a gone tool is a clear error");
}

#[test]
fn a_cache_round_trips_through_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp-schema-cache.json");
    let server = config("srv", "srv");
    let mut cache = McpSchemaCache::new();
    cache.update(&server, vec![tool_def("search")]);
    cache.save(&path).unwrap();

    let loaded = McpSchemaCache::load(&path).unwrap();
    assert_eq!(loaded.tools_for(&server)[0].name, "search");

    // A missing file yields an empty cache, not an error.
    let empty = McpSchemaCache::load(&dir.path().join("missing.json")).unwrap();
    assert!(empty.tools_for(&server).is_empty());
}
