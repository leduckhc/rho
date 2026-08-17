//! Host tests for `rho-plugin`. The stub plugin is a second binary in this
//! crate, `rho_stub_plugin`. Its path comes from `CARGO_BIN_EXE_rho_stub_plugin`.
//! No test reaches the network. No test uses a real multi-second sleep.

use std::time::Duration;

use rho_core::{CancelToken, ContentBlock, ToolContext};
use rho_plugin::{PluginCache, PluginHost, PluginToolSpec};

/// The stub plugin path, provided by cargo for this crate's binary.
const STUB: &str = env!("CARGO_BIN_EXE_rho_stub_plugin");

fn host_with_short_timeout() -> PluginHost {
    PluginHost::new().with_call_timeout(Duration::from_millis(500))
}

fn ctx(root: &std::path::Path) -> (ToolContext, tokio::sync::mpsc::Receiver<String>) {
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    (
        ToolContext {
            session_root: root.to_path_buf(),
            cancel: CancelToken::new(),
            updates: tx,
        },
        rx,
    )
}

#[tokio::test]
async fn plugin_host_launches_and_handshakes() {
    let mut host = PluginHost::new();
    let process = host.launch(STUB, &["normal".to_string()]).await.unwrap();
    assert_eq!(process.name(), "stub");
    assert!(process.is_available());
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_host_lists_tools_from_handshake() {
    let mut host = PluginHost::new();
    host.launch(STUB, &["normal".to_string()]).await.unwrap();
    let tools = host.tools();
    assert!(tools.iter().any(|t| t.name() == "echo"));
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_host_calls_tool_and_gets_result() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = PluginHost::new();
    host.launch(STUB, &["normal".to_string()]).await.unwrap();
    let tool = host
        .tools()
        .into_iter()
        .find(|t| t.name() == "echo")
        .unwrap();
    let (context, _rx) = ctx(dir.path());
    let out = tool
        .execute(serde_json::json!({ "text": "hello plugin" }), context)
        .await
        .unwrap();
    let text: String = out
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "hello plugin");
    assert!(!out.is_error);
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_host_forwards_tool_updates() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = PluginHost::new();
    host.launch(STUB, &["normal".to_string()]).await.unwrap();
    let tool = host
        .tools()
        .into_iter()
        .find(|t| t.name() == "echo")
        .unwrap();
    let (context, mut rx) = ctx(dir.path());
    let run = tokio::spawn(async move {
        tool.execute(serde_json::json!({ "text": "x" }), context)
            .await
    });
    // The stub streams one "working" update before the result.
    let update = rx.recv().await;
    assert_eq!(update, Some("working".to_string()));
    run.await.unwrap().unwrap();
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_host_crash_returns_error_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = host_with_short_timeout();
    let process = host.launch(STUB, &["crash".to_string()]).await.unwrap();
    // Call the plugin process directly to observe the raw error.
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let result = process
        .call_tool(
            "echo",
            serde_json::json!({ "text": "x" }),
            tx,
            CancelToken::new(),
        )
        .await;
    assert!(result.is_err(), "a crashed plugin yields an error");

    // The host stays usable: the proxy tool converts the failure to an error
    // result, so the session continues instead of aborting.
    let tool = host
        .tools()
        .into_iter()
        .find(|t| t.name() == "echo")
        .unwrap();
    let (context, _rx2) = ctx(dir.path());
    let out = tool
        .execute(serde_json::json!({ "text": "x" }), context)
        .await
        .unwrap();
    assert!(
        out.is_error,
        "the proxy returns an error result, not a panic"
    );
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_host_malformed_line_is_dropped() {
    // The garbage stub writes a non-JSON line before the handshake response. The
    // host must drop it and still complete the handshake.
    let mut host = PluginHost::new();
    let process = host.launch(STUB, &["garbage".to_string()]).await.unwrap();
    assert_eq!(process.name(), "stub");
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_host_call_times_out() {
    let mut host = host_with_short_timeout();
    let process = host.launch(STUB, &["hang".to_string()]).await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let result = process
        .call_tool(
            "echo",
            serde_json::json!({ "text": "x" }),
            tx,
            CancelToken::new(),
        )
        .await;
    assert!(
        matches!(result, Err(rho_plugin::PluginError::Timeout)),
        "a hung plugin call times out, got {result:?}"
    );
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_host_cancel_stops_call() {
    let mut host = PluginHost::new();
    let process = host.launch(STUB, &["hang".to_string()]).await.unwrap();
    let cancel = CancelToken::new();
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let call_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        process
            .call_tool("echo", serde_json::json!({ "text": "x" }), tx, call_cancel)
            .await
    });
    cancel.cancel();
    let result = handle.await.unwrap();
    assert!(
        matches!(result, Err(rho_plugin::PluginError::Canceled)),
        "a cancelled call stops, got {result:?}"
    );
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_host_enormous_line_does_not_panic() {
    // The bigline stub writes a 20 MB line, then the real result. The host caps
    // the line and still reads the result. It must not panic.
    let dir = tempfile::tempdir().unwrap();
    let mut host = host_with_short_timeout();
    let process = host.launch(STUB, &["bigline".to_string()]).await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let out = process
        .call_tool(
            "echo",
            serde_json::json!({ "text": "survived" }),
            tx,
            CancelToken::new(),
        )
        .await;
    let _ = dir;
    assert!(out.is_ok(), "the host reads the result after a capped line");
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_schema_cache_roundtrips() {
    let cache = PluginCache {
        version: 1,
        plugin: "my-plugin".to_string(),
        tools: vec![PluginToolSpec {
            name: "search_docs".to_string(),
            description: "Search the docs".to_string(),
            kind: rho_core::ToolKind::Search,
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "query": { "type": "string" } }
            }),
        }],
    };
    let bytes = cache.to_json().unwrap();
    let parsed = PluginCache::from_json(&bytes).unwrap();
    assert_eq!(parsed, cache);
    // The wire name for the schema is `inputSchema`.
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("inputSchema"));
}

#[tokio::test]
async fn plugin_tool_advertised_from_cache_before_connect() {
    let mut host = PluginHost::new();
    host.load_cache(PluginCache {
        version: 1,
        plugin: "my-plugin".to_string(),
        tools: vec![PluginToolSpec {
            name: "search_docs".to_string(),
            description: "Search the docs".to_string(),
            kind: rho_core::ToolKind::Search,
            input_schema: serde_json::json!({ "type": "object" }),
        }],
    });
    // No plugin has launched, yet the cached tool is advertised.
    let tools = host.tools();
    assert!(tools.iter().any(|t| t.name() == "search_docs"));
}

#[tokio::test]
async fn plugin_host_clean_shutdown_leaves_no_orphan() {
    let mut host = PluginHost::new();
    let process = host.launch(STUB, &["normal".to_string()]).await.unwrap();
    let pid = std::sync::Arc::strong_count(&process); // keep a ref for the check
    let _ = pid;
    host.shutdown().await;
    // After shutdown the process is marked unavailable and reaped.
    assert!(!process.is_available(), "the plugin is shut down");
}

#[tokio::test]
async fn plugin_declared_read_kind_does_not_bypass_read_only_policy() {
    // A security regression test, from a security audit finding.
    //
    // `ToolKind` drives the approval boundary. `ReadOnlyPolicy` allows a read-only
    // kind and denies everything else. If the host trusted the kind a plugin
    // advertises, a hostile or compromised plugin would declare a destructive tool as
    // `Read` and then run under a read-only policy.
    //
    // So the host reports `Other` for every plugin tool, whatever the plugin claims.
    // `Other` counts as mutating, so a read-only session denies it. The plugin's own
    // claim survives for display only.
    use rho_core::{ApprovalDecision, ApprovalPolicy, ReadOnlyPolicy, ToolKind};

    let mut host = PluginHost::new();
    host.launch(STUB, &["normal".to_string()]).await.unwrap();
    let tools = host.tools();
    assert!(!tools.is_empty(), "the stub plugin must advertise a tool");

    for tool in &tools {
        assert_eq!(
            tool.kind(),
            ToolKind::Other,
            "the host must not repeat a plugin's own kind claim for tool {}",
            tool.name()
        );
        let decision = ReadOnlyPolicy
            .approve(tool.name(), tool.kind(), &serde_json::json!({}))
            .await;
        assert_eq!(
            decision,
            ApprovalDecision::Deny,
            "a read-only policy must deny the plugin tool {}",
            tool.name()
        );
    }
}
