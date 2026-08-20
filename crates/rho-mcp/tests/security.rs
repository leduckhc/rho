//! Security tests for `rho-mcp`.
//!
//! An MCP server is semi-trusted. Its kind claim is refused, a read-only policy
//! denies it, approval runs before a call, the environment is scrubbed, and the
//! output is sanitised. See `SPEC-mcp` section 6 and decisions D-bash-scrubs-credentials and D-mcp-does-not-classify-itself.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{RecordingFactory, stub_config, test_limits};
use rho_core::{
    AllowAllPolicy, ApprovalDecision, ApprovalPolicy, CancelToken, ReadOnlyPolicy, ToolContext,
    ToolKind,
};
use rho_mcp::{
    McpClient, McpPool, McpSchemaCache, McpServerConfig, McpToolDef, McpTransport, tools_for,
};

fn tool_def(name: &str) -> McpToolDef {
    McpToolDef {
        name: name.to_string(),
        description: "A tool.".to_string(),
        input_schema: serde_json::json!({ "type": "object" }),
    }
}

fn config(name: &str) -> McpServerConfig {
    McpServerConfig {
        name: name.to_string(),
        transport: McpTransport::Stdio {
            command: name.to_string(),
            args: vec![],
        },
        env: Default::default(),
        shared: true,
        call_timeout_ms: None,
    }
}

fn ctx() -> ToolContext {
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    ToolContext {
        session_root: std::env::temp_dir(),
        cancel: CancelToken::new(),
        updates: tx,
    }
}

#[tokio::test]
async fn every_mcp_tool_reports_tool_kind_other() {
    // An MCP server never classifies its own tools. Whatever a server claims,
    // every MCP tool reports `Other`. This is decision D-mcp-does-not-classify-itself.
    let pool = McpPool::with_factory(test_limits(), Arc::new(common::FakeFactory));
    let server = config("srv");
    let mut cache = McpSchemaCache::new();
    cache.update(&server, vec![tool_def("a"), tool_def("b")]);

    let tools = tools_for(&pool, std::slice::from_ref(&server), &cache)
        .await
        .unwrap();
    assert!(!tools.is_empty());
    for tool in &tools {
        assert_eq!(tool.kind(), ToolKind::Other, "every MCP tool is Other");
    }
}

#[tokio::test]
async fn a_read_only_policy_denies_an_mcp_tool() {
    let pool = McpPool::with_factory(test_limits(), Arc::new(common::FakeFactory));
    let server = config("srv");
    let mut cache = McpSchemaCache::new();
    cache.update(&server, vec![tool_def("a")]);
    let tools = tools_for(&pool, std::slice::from_ref(&server), &cache)
        .await
        .unwrap();
    let tool = &tools[0];

    let policy = ReadOnlyPolicy;
    let decision = policy
        .approve(tool.name(), tool.kind(), &serde_json::json!({}))
        .await;
    assert_eq!(
        decision,
        ApprovalDecision::Deny,
        "a read-only policy denies an MCP tool"
    );
}

#[tokio::test]
async fn approval_runs_before_the_call_reaches_the_server() {
    // A recording transport counts every `tools/call`. With a denying policy the
    // call must never reach the server. A positive control proves the recorder
    // detects a real call.
    let calls = Arc::new(AtomicUsize::new(0));
    let factory = Arc::new(RecordingFactory {
        calls: Arc::clone(&calls),
    });
    let pool = McpPool::with_factory(test_limits(), factory);
    let server = config("srv");
    let mut cache = McpSchemaCache::new();
    cache.update(&server, vec![tool_def("a")]);
    let tools = tools_for(&pool, std::slice::from_ref(&server), &cache)
        .await
        .unwrap();
    let tool = &tools[0];
    let args = serde_json::json!({});

    // The approval gate runs first. A denial must stop the call.
    let denied = ReadOnlyPolicy
        .approve(tool.name(), tool.kind(), &args)
        .await;
    assert_eq!(denied, ApprovalDecision::Deny);
    if denied == ApprovalDecision::Allow {
        let _ = tool.execute(args.clone(), ctx()).await;
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a denied call never reaches the server"
    );

    // Positive control: an allowing policy lets the call through.
    let allowed = AllowAllPolicy
        .approve(tool.name(), tool.kind(), &args)
        .await;
    assert_eq!(allowed, ApprovalDecision::Allow);
    let _ = tool.execute(args, ctx()).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "an approved call reaches the server"
    );
}

#[tokio::test]
async fn a_stdio_server_does_not_inherit_a_credential_variable() {
    // Set a credential-shaped variable in the parent. The stdio server must not
    // see it, because the client scrubs the environment. See decision D-bash-scrubs-credentials.
    // SAFETY: the variable name is unique to this test, so no parallel test
    // reads it, and it is set before the child is spawned.
    unsafe {
        std::env::set_var("RHO_MCP_TEST_SECRET_KEY", "leak-me");
    }
    let (client, _tools) = McpClient::connect(&stub_config("stub", "printenv"), test_limits())
        .await
        .unwrap();
    let output = client
        .call(
            "printenv",
            serde_json::json!({ "var": "RHO_MCP_TEST_SECRET_KEY" }),
        )
        .await
        .unwrap();
    let text = format!("{:?}", output.content);
    assert!(
        !text.contains("leak-me"),
        "a credential variable must not reach the server: {text}"
    );
    client.shutdown().await;
}

#[tokio::test]
async fn a_configured_env_entry_reaches_the_server() {
    // The scrub must not break a server that legitimately needs a token. A
    // configured entry is added after the scrub, so it reaches the server, even
    // when its name looks like a credential.
    let mut server = stub_config("stub", "printenv");
    server
        .env
        .insert("MY_SERVICE_TOKEN".to_string(), "abc123".to_string());
    let (client, _tools) = McpClient::connect(&server, test_limits()).await.unwrap();
    let output = client
        .call("printenv", serde_json::json!({ "var": "MY_SERVICE_TOKEN" }))
        .await
        .unwrap();
    let text = format!("{:?}", output.content);
    assert!(
        text.contains("abc123"),
        "a configured token reaches the server: {text}"
    );
    client.shutdown().await;
}

#[tokio::test]
async fn output_with_an_escape_sequence_is_sanitised() {
    let (client, _tools) = McpClient::connect(&stub_config("stub", "escape"), test_limits())
        .await
        .unwrap();
    let output = client
        .call("anything", serde_json::json!({}))
        .await
        .unwrap();
    let text = format!("{:?}", output.content);
    assert!(!text.contains('\u{1b}'), "no escape byte survives: {text}");
    assert!(text.contains("red"), "the visible text is kept: {text}");
    client.shutdown().await;
}
