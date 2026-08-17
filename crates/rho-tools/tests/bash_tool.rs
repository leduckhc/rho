//! Behaviour tests for the `bash` tool.
//!
//! No test uses a real multi-second sleep. The timeout test sets a tiny timeout
//! against a long command, so it finishes fast. The cancel test reads one
//! streamed line, then cancels, so it is deterministic and fast.

mod common;

use common::Harness;
use rho_core::{Tool, ToolError};
use rho_tools::BashTool;
use std::time::Duration;

fn text_of(output: &rho_core::ToolOutput) -> String {
    output
        .content
        .iter()
        .filter_map(|b| match b {
            rho_core::ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

#[tokio::test]
async fn tool_bash_streams_output_lines() {
    let mut h = Harness::new();
    let ctx = h.ctx();
    let out = BashTool
        .execute(serde_json::json!({ "command": "echo one; echo two" }), ctx)
        .await
        .unwrap();
    // The streamed lines reached the update channel.
    // Give the sink task a chance to drain the final lines.
    tokio::task::yield_now().await;
    let lines = h.lines();
    assert!(
        lines.iter().any(|l| l == "one"),
        "streamed lines: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l == "two"),
        "streamed lines: {lines:?}"
    );
    // The combined output holds both lines.
    let text = text_of(&out);
    assert!(text.contains("one"));
    assert!(text.contains("two"));
}

#[tokio::test]
async fn tool_bash_reports_nonzero_exit() {
    let mut h = Harness::new();
    let out = BashTool
        .execute(serde_json::json!({ "command": "exit 3" }), h.ctx())
        .await
        .unwrap();
    assert!(out.is_error, "a non-zero exit is an error");
    assert!(text_of(&out).contains("exit code 3"));
}

#[tokio::test]
async fn tool_bash_enforces_timeout() {
    let mut h = Harness::new();
    // A tiny timeout against a long command. The command never nears its sleep,
    // so the test finishes in about the timeout, not in the sleep.
    let error = BashTool
        .execute(
            serde_json::json!({ "command": "sleep 30", "timeout_ms": 50 }),
            h.ctx(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::Timeout(_)), "got {error:?}");
}

#[tokio::test]
async fn tool_bash_cancel_kills_process() {
    let mut h = Harness::new();
    let cancel = h.cancel.clone();
    let ctx = h.ctx();
    // The command prints a marker, then sleeps. Cancel after the marker arrives.
    let run = tokio::spawn(async move {
        BashTool
            .execute(
                serde_json::json!({ "command": "echo started; sleep 30" }),
                ctx,
            )
            .await
    });

    // Wait for the marker line, then cancel. This is deterministic, not timed.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if h.lines().iter().any(|l| l == "started") {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("the command never streamed its marker");
        }
        tokio::task::yield_now().await;
    }
    cancel.cancel();

    let result = run.await.unwrap();
    assert!(matches!(result, Err(ToolError::Canceled)), "got {result:?}");
}

#[tokio::test]
async fn tool_bash_truncates_large_output() {
    let mut h = Harness::new();
    // `yes` streams forever. `head` bounds it. The output crosses the cap.
    let out = BashTool
        .execute(
            serde_json::json!({ "command": "for i in $(seq 1 20000); do echo AAAAAAAAAAAAAAAAAAAA; done" }),
            h.ctx(),
        )
        .await
        .unwrap();
    let text = text_of(&out);
    assert!(text.contains("[truncated"), "must note truncation");
    assert!(text.len() < 200_000, "must be bounded near the cap");
}
