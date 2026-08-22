//! The `read_tool_result` tool. SPEC-tool-result-handle section 8.

use rho_core::{
    FileResultStore, ResultLimits, ResultStore, Tool, ToolContext, ToolError, ToolKind,
};
use rho_tools::ReadToolResultTool;
use std::sync::Arc;
use tempfile::TempDir;

/// A tool over a real store that already holds one payload.
async fn tool_with(payload: &str) -> (TempDir, ReadToolResultTool, String) {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(FileResultStore::open(dir.path()).await.unwrap());
    let handle = store.put(payload).await.unwrap();
    let tool = ReadToolResultTool::new(store, ResultLimits::default());
    (dir, tool, handle)
}

fn ctx(root: &std::path::Path) -> ToolContext {
    // The receivers are dropped, because this tool streams nothing.
    let (updates, _updates_rx) = tokio::sync::mpsc::channel(16);
    let (agent_events, _agent_rx) = tokio::sync::mpsc::channel(16);
    ToolContext {
        session_root: root.to_path_buf(),
        cancel: rho_core::CancelToken::new(),
        updates,
        agent_events,
    }
}

/// The text of the first block of a successful run.
fn text(output: &rho_core::ToolOutput) -> String {
    output
        .content
        .iter()
        .filter_map(|b| match b {
            rho_core::ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn read_tool_result_returns_a_range() {
    let (dir, tool, handle) = tool_with(&"r".repeat(50_000)).await;

    let output = tool
        .execute(
            serde_json::json!({ "handle": handle, "start_byte": 10, "byte_count": 100 }),
            ctx(dir.path()),
        )
        .await
        .unwrap();

    let body = text(&output);
    assert!(body.contains("start_byte=\"10\""), "{body}");
    assert!(body.contains("end_byte=\"110\""), "{body}");
    assert!(body.contains("total_bytes=\"50000\""), "{body}");
    // Count the payload, not the envelope. The envelope holds its own letters.
    let start = body.find(">\n").unwrap() + 2;
    let end = body.rfind("\n</tool_result>").unwrap();
    assert_eq!(&body[start..end], &"r".repeat(100));
}

#[tokio::test]
async fn read_tool_result_searches_for_a_literal() {
    let (dir, tool, handle) = tool_with("alpha\nbeta FOUND-IT\ngamma\n").await;

    let output = tool
        .execute(
            serde_json::json!({ "handle": handle, "query": "FOUND-IT" }),
            ctx(dir.path()),
        )
        .await
        .unwrap();

    let body = text(&output);
    assert!(body.contains("beta FOUND-IT"), "{body}");
    assert!(body.contains("2:"), "the match must name its line: {body}");
    assert!(
        !body.contains("<tool_result "),
        "a query returns matches, not a range"
    );
}

#[tokio::test]
async fn read_tool_result_defaults_its_range() {
    let (dir, tool, handle) = tool_with("START-HERE and then more text").await;

    let output = tool
        .execute(serde_json::json!({ "handle": handle }), ctx(dir.path()))
        .await
        .unwrap();

    let body = text(&output);
    assert!(
        body.contains("START-HERE"),
        "no offset must read from zero: {body}"
    );
    assert!(body.contains("start_byte=\"0\""), "{body}");
}

#[tokio::test]
async fn read_tool_result_rejects_a_missing_handle() {
    let (dir, tool, _handle) = tool_with("body").await;

    let error = tool
        .execute(serde_json::json!({ "byte_count": 10 }), ctx(dir.path()))
        .await
        .expect_err("a call with no handle is an error");

    assert!(matches!(error, ToolError::InvalidArguments(_)), "{error:?}");
}

#[tokio::test]
async fn read_tool_result_explains_an_unknown_handle() {
    let (dir, tool, _handle) = tool_with("body").await;

    let error = tool
        .execute(
            serde_json::json!({ "handle": "tr-0123456789abcdef-999999" }),
            ctx(dir.path()),
        )
        .await
        .expect_err("an unknown handle is an error");

    let message = error.to_string();
    assert!(
        message.contains("copied exactly"),
        "the message must say where a handle comes from: {message}"
    );
    assert!(
        message.contains("session"),
        "and that it belongs to one session: {message}"
    );
}

#[tokio::test]
async fn read_tool_result_refuses_a_malformed_handle() {
    let (dir, tool, _handle) = tool_with("body").await;

    let error = tool
        .execute(
            serde_json::json!({ "handle": "../../etc/passwd" }),
            ctx(dir.path()),
        )
        .await
        .expect_err("a malformed handle is an error, never a panic");

    assert!(matches!(error, ToolError::InvalidArguments(_)), "{error:?}");
    assert!(error.to_string().contains("is not a result handle"));
}

#[tokio::test]
async fn read_tool_result_is_a_read_kind() {
    let (_dir, tool, _handle) = tool_with("body").await;

    // A read-only policy allows only a non-mutating kind. `Other` would be denied, and this
    // tool would then be useless in the mode that needs it most.
    assert_eq!(tool.kind(), ToolKind::Read);
    assert!(tool.kind().is_read_only());
}

#[tokio::test]
async fn read_tool_result_reports_reading_past_the_end() {
    let (dir, tool, handle) = tool_with("short body").await;

    let output = tool
        .execute(
            serde_json::json!({ "handle": handle, "start_byte": 9000 }),
            ctx(dir.path()),
        )
        .await
        .unwrap();

    let body = text(&output);
    assert!(
        body.contains("nothing after"),
        "the model must learn it is done: {body}"
    );
    assert!(body.contains("10 bytes"), "and the true size: {body}");
}
