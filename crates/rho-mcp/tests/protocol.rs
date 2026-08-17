//! Protocol and robustness tests for `rho-mcp`.
//!
//! The stub MCP server is a second binary in this crate. Its path comes from
//! `CARGO_BIN_EXE_rho_stub_mcp_server`. No test reaches the network. No test
//! uses a real multi-second sleep; a bounded timeout guards a hang instead.

mod common;

use common::{stub_config, test_limits};
use rho_mcp::{McpClient, McpError};

#[tokio::test]
async fn connect_performs_initialize_then_lists_tools() {
    let (client, tools) = McpClient::connect(&stub_config("stub", "normal"), test_limits())
        .await
        .expect("connect must succeed");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");
    client.shutdown().await;
}

#[tokio::test]
async fn call_returns_the_tool_output() {
    let (client, _tools) = McpClient::connect(&stub_config("stub", "normal"), test_limits())
        .await
        .unwrap();
    let output = client
        .call("echo", serde_json::json!({ "text": "hello" }))
        .await
        .unwrap();
    assert!(!output.is_error);
    let text = format!("{:?}", output.content);
    assert!(
        text.contains("hello"),
        "the output echoes the argument: {text}"
    );
    client.shutdown().await;
}

#[tokio::test]
async fn a_server_error_becomes_a_tool_error_not_a_panic() {
    let (client, _tools) = McpClient::connect(&stub_config("stub", "normal"), test_limits())
        .await
        .unwrap();
    // The stub returns a JSON-RPC error for an unknown tool.
    let output = client
        .call("no_such_tool", serde_json::json!({}))
        .await
        .unwrap();
    assert!(output.is_error, "an unknown tool is an error result");
    client.shutdown().await;
}

#[tokio::test]
async fn a_paged_tools_list_is_fully_collected() {
    let (client, tools) = McpClient::connect(&stub_config("stub", "paged"), test_limits())
        .await
        .unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["first", "second"], "both pages are collected");
    client.shutdown().await;
}

#[tokio::test]
async fn an_unknown_protocol_version_is_reported_clearly() {
    let result = McpClient::connect(&stub_config("stub", "badversion"), test_limits()).await;
    let error = match result {
        Ok(_) => panic!("an unknown version must fail"),
        Err(error) => error,
    };
    match error {
        McpError::UnknownProtocolVersion { server, version } => {
            assert_eq!(server, "stub");
            assert_eq!(version, "1999-01-01");
        }
        other => panic!("wrong error: {other}"),
    }
}

#[tokio::test]
async fn a_server_that_never_answers_times_out_and_fails_one_call() {
    let (client, _tools) = McpClient::connect(&stub_config("stub", "hang"), test_limits())
        .await
        .unwrap();
    // The stub answers the handshake but never answers a call. The call must
    // time out. A bounded timeout guards against a hang in the client.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client.call("echo", serde_json::json!({ "text": "x" })),
    )
    .await
    .expect("the call must return, not hang");
    assert!(
        matches!(result, Err(McpError::CallTimeout { .. })),
        "a call to a silent server times out: {result:?}"
    );
    client.shutdown().await;
}

#[tokio::test]
async fn a_server_that_dies_mid_call_returns_an_error_and_the_session_survives() {
    let (client, _tools) = McpClient::connect(&stub_config("stub", "crash"), test_limits())
        .await
        .unwrap();
    let result = client
        .call("echo", serde_json::json!({ "text": "x" }))
        .await;
    assert!(result.is_err(), "a crash mid-call is an error: {result:?}");
    // The client is still usable: a second call returns an error, not a panic.
    let again = client
        .call("echo", serde_json::json!({ "text": "y" }))
        .await;
    assert!(again.is_err(), "the session survives a server crash");
    client.shutdown().await;
}

#[tokio::test]
async fn a_garbage_line_does_not_panic_the_client() {
    // The stub emits a non-JSON line before the handshake response. The client
    // must drop it and still complete the handshake.
    let (client, tools) = McpClient::connect(&stub_config("stub", "garbage"), test_limits())
        .await
        .expect("a garbage line must not break the handshake");
    assert_eq!(tools.len(), 1);
    client.shutdown().await;
}

#[tokio::test]
async fn a_line_longer_than_the_cap_is_refused() {
    // The stub writes a 5 MB line, then the real result. The client must refuse
    // the over-long line, keep memory bounded, and still read the result.
    let (client, _tools) = McpClient::connect(&stub_config("stub", "bigline"), test_limits())
        .await
        .unwrap();
    let output = client
        .call("echo", serde_json::json!({ "text": "small" }))
        .await
        .unwrap();
    let text = format!("{:?}", output.content);
    assert!(text.contains("small"), "the result still arrives: {text}");
    assert!(!text.contains("zzzz"), "the over-long line is refused");
    client.shutdown().await;
}

#[tokio::test]
async fn a_schema_larger_than_the_cap_is_refused() {
    let result = McpClient::connect(&stub_config("stub", "bigschema"), test_limits()).await;
    let error = match result {
        Ok(_) => panic!("an over-large schema must be refused"),
        Err(error) => error,
    };
    match error {
        McpError::SchemaTooLarge { server, .. } => assert_eq!(server, "stub"),
        other => panic!("wrong error: {other}"),
    }
}

#[tokio::test]
async fn a_server_advertising_too_many_tools_is_capped() {
    let mut limits = test_limits();
    limits.max_tools_per_server = 10;
    let (client, tools) = McpClient::connect(&stub_config("stub", "manytools"), limits)
        .await
        .unwrap();
    assert_eq!(tools.len(), 10, "the tool list is capped");
    client.shutdown().await;
}
