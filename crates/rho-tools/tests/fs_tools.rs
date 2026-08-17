//! Behaviour tests for the filesystem tools: `read`, `write`, `edit`, `list`.

mod common;

use common::Harness;
use rho_core::{Tool, ToolError};
use rho_tools::{EditTool, ListTool, ReadTool, WriteTool};

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
async fn tool_read_returns_file_contents() {
    let mut h = Harness::new();
    h.write_file("a.txt", "hello\nworld\n");
    let out = ReadTool
        .execute(serde_json::json!({ "path": "a.txt" }), h.ctx())
        .await
        .unwrap();
    assert_eq!(text_of(&out), "hello\nworld");
}

#[tokio::test]
async fn tool_read_offset_limit_slices_lines() {
    let mut h = Harness::new();
    h.write_file("a.txt", "l1\nl2\nl3\nl4\nl5\n");
    let out = ReadTool
        .execute(
            serde_json::json!({ "path": "a.txt", "offset": 2, "limit": 2 }),
            h.ctx(),
        )
        .await
        .unwrap();
    assert_eq!(text_of(&out), "l2\nl3");
}

#[tokio::test]
async fn tool_read_offset_and_limit_beyond_end_returns_empty() {
    let mut h = Harness::new();
    h.write_file("a.txt", "l1\nl2\n");
    let out = ReadTool
        .execute(
            serde_json::json!({ "path": "a.txt", "offset": 99, "limit": 5 }),
            h.ctx(),
        )
        .await
        .unwrap();
    assert_eq!(text_of(&out), "");
}

#[tokio::test]
async fn tool_read_missing_file_is_io_error() {
    let mut h = Harness::new();
    let error = ReadTool
        .execute(serde_json::json!({ "path": "nope.txt" }), h.ctx())
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::Io(_)));
}

#[tokio::test]
async fn tool_read_directory_is_invalid_arguments() {
    let mut h = Harness::new();
    std::fs::create_dir(h.root().join("sub")).unwrap();
    let error = ReadTool
        .execute(serde_json::json!({ "path": "sub" }), h.ctx())
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::InvalidArguments(_)));
}

#[tokio::test]
async fn tool_read_binary_file_is_invalid_arguments() {
    let mut h = Harness::new();
    std::fs::write(h.root().join("b.bin"), [0xff, 0xfe, 0x00, 0x01]).unwrap();
    let error = ReadTool
        .execute(serde_json::json!({ "path": "b.bin" }), h.ctx())
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::InvalidArguments(_)));
}

#[tokio::test]
async fn tool_read_truncates_a_very_large_file() {
    let mut h = Harness::new();
    // One long line over the 100000-byte cap.
    let big = "x".repeat(150_000);
    h.write_file("big.txt", &big);
    let out = ReadTool
        .execute(serde_json::json!({ "path": "big.txt" }), h.ctx())
        .await
        .unwrap();
    let text = text_of(&out);
    assert!(text.contains("[truncated"), "must note truncation");
    assert!(text.len() < big.len(), "must be shorter than the input");
}

#[tokio::test]
async fn tool_write_creates_file_and_parents() {
    let mut h = Harness::new();
    let out = WriteTool
        .execute(
            serde_json::json!({ "path": "nested/dir/new.txt", "content": "hi" }),
            h.ctx(),
        )
        .await
        .unwrap();
    assert!(!out.is_error);
    assert_eq!(h.read_file("nested/dir/new.txt"), "hi");
}

#[tokio::test]
async fn tool_write_overwrites_existing_file() {
    let mut h = Harness::new();
    h.write_file("a.txt", "old");
    WriteTool
        .execute(
            serde_json::json!({ "path": "a.txt", "content": "new" }),
            h.ctx(),
        )
        .await
        .unwrap();
    assert_eq!(h.read_file("a.txt"), "new");
}

#[tokio::test]
async fn tool_write_rejects_path_escape() {
    let mut h = Harness::new();
    let error = WriteTool
        .execute(
            serde_json::json!({ "path": "../escape.txt", "content": "x" }),
            h.ctx(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::PathEscape(_)));
}

#[tokio::test]
async fn tool_edit_replaces_unique_span() {
    let mut h = Harness::new();
    h.write_file("a.txt", "alpha\nbeta\ngamma\n");
    EditTool
        .execute(
            serde_json::json!({ "path": "a.txt", "old_text": "beta", "new_text": "BETA" }),
            h.ctx(),
        )
        .await
        .unwrap();
    assert_eq!(h.read_file("a.txt"), "alpha\nBETA\ngamma\n");
}

#[tokio::test]
async fn tool_edit_fails_on_absent_span() {
    let mut h = Harness::new();
    h.write_file("a.txt", "alpha\n");
    let error = EditTool
        .execute(
            serde_json::json!({ "path": "a.txt", "old_text": "zzz", "new_text": "y" }),
            h.ctx(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::InvalidArguments(_)));
}

#[tokio::test]
async fn tool_edit_fails_on_ambiguous_span() {
    let mut h = Harness::new();
    h.write_file("a.txt", "dup\ndup\n");
    let error = EditTool
        .execute(
            serde_json::json!({ "path": "a.txt", "old_text": "dup", "new_text": "x" }),
            h.ctx(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::InvalidArguments(_)));
    // The file must be unchanged after an ambiguous edit.
    assert_eq!(h.read_file("a.txt"), "dup\ndup\n");
}

#[tokio::test]
async fn tool_list_returns_entries() {
    let mut h = Harness::new();
    h.write_file("a.txt", "");
    std::fs::create_dir(h.root().join("sub")).unwrap();
    let out = ListTool
        .execute(serde_json::json!({}), h.ctx())
        .await
        .unwrap();
    let text = text_of(&out);
    assert!(text.contains("a.txt"));
    assert!(text.contains("sub/"), "a directory ends with a slash");
}
