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

// --- edit: replace_all, diagnostics, and guards ----------------------------

fn edit_error_text(error: &ToolError) -> String {
    error.to_string()
}

#[tokio::test]
async fn tool_edit_replace_all_replaces_every_match() {
    let mut h = Harness::new();
    h.write_file("a.txt", "dup\nkeep\ndup\ndup\n");
    let out = EditTool
        .execute(
            serde_json::json!({
                "path": "a.txt", "old_text": "dup", "new_text": "X", "replace_all": true
            }),
            h.ctx(),
        )
        .await
        .unwrap();
    assert_eq!(h.read_file("a.txt"), "X\nkeep\nX\nX\n");
    assert!(
        text_of(&out).contains('3'),
        "the output must report how many spans changed: {}",
        text_of(&out)
    );
}

#[tokio::test]
async fn tool_edit_ambiguous_error_offers_replace_all() {
    // The error must name both ways out. A model that only hears "not unique" adds
    // context forever, when replacing every match was the intent.
    let mut h = Harness::new();
    h.write_file("a.txt", "dup\ndup\n");
    let error = EditTool
        .execute(
            serde_json::json!({ "path": "a.txt", "old_text": "dup", "new_text": "x" }),
            h.ctx(),
        )
        .await
        .unwrap_err();
    let text = edit_error_text(&error);
    assert!(text.contains('2'), "name the count: {text}");
    assert!(
        text.contains("replace_all"),
        "offer the escape hatch: {text}"
    );
}

#[tokio::test]
async fn tool_edit_rejects_an_empty_old_text() {
    // A guard against a real defect in another harness. An empty pattern matches at
    // every character boundary, so `replace_all` would rewrite the whole file:
    // "abc" becomes "XaXbXcX". Refuse it instead.
    let mut h = Harness::new();
    h.write_file("a.txt", "abc");
    for replace_all in [false, true] {
        let error = EditTool
            .execute(
                serde_json::json!({
                    "path": "a.txt", "old_text": "", "new_text": "X",
                    "replace_all": replace_all
                }),
                h.ctx(),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, ToolError::InvalidArguments(_)));
        assert_eq!(h.read_file("a.txt"), "abc", "the file must not change");
    }
}

#[tokio::test]
async fn tool_edit_rejects_an_unchanged_edit() {
    // Equal texts mean the model believes it changed something. Reporting success
    // would be a silent failure, and it would waste a turn.
    let mut h = Harness::new();
    h.write_file("a.txt", "same\n");
    let error = EditTool
        .execute(
            serde_json::json!({ "path": "a.txt", "old_text": "same", "new_text": "same" }),
            h.ctx(),
        )
        .await
        .unwrap_err();
    assert!(edit_error_text(&error).contains("differ"), "{error}");
}

#[tokio::test]
async fn tool_edit_explains_a_trailing_whitespace_mismatch() {
    // The most common near miss. The span exists, but the model added or dropped
    // surrounding whitespace. Saying so turns a dead end into a recoverable error.
    let mut h = Harness::new();
    h.write_file("a.txt", "alpha\nbeta\n");
    let error = EditTool
        .execute(
            serde_json::json!({ "path": "a.txt", "old_text": "  beta  ", "new_text": "B" }),
            h.ctx(),
        )
        .await
        .unwrap_err();
    let text = edit_error_text(&error);
    assert!(
        text.contains("whitespace"),
        "the message must name the cause: {text}"
    );
}

#[tokio::test]
async fn tool_edit_explains_an_indentation_mismatch_with_a_line_number() {
    let mut h = Harness::new();
    h.write_file("a.rs", "fn main() {\n    let x = 1;\n    let y = 2;\n}\n");
    let error = EditTool
        .execute(
            serde_json::json!({
                "path": "a.rs",
                "old_text": "let x = 1;\nlet y = 2;",
                "new_text": "let z = 3;"
            }),
            h.ctx(),
        )
        .await
        .unwrap_err();
    let text = edit_error_text(&error);
    assert!(
        text.contains("indentation"),
        "the message must name indentation: {text}"
    );
    assert!(text.contains('2'), "and the line it found: {text}");
}

#[tokio::test]
async fn tool_edit_accepts_the_familiar_argument_names() {
    // Models are heavily trained on other harnesses, which name these arguments
    // file_path, old_string, and new_string. rho advertises its own consistent names,
    // and it also accepts those, so a model reaching for a name it knows still works.
    let mut h = Harness::new();
    h.write_file("a.txt", "alpha\n");
    EditTool
        .execute(
            serde_json::json!({
                "file_path": "a.txt", "old_string": "alpha", "new_string": "ALPHA"
            }),
            h.ctx(),
        )
        .await
        .unwrap();
    assert_eq!(h.read_file("a.txt"), "ALPHA\n");
}

#[tokio::test]
async fn tool_edit_shows_context_after_the_edit() {
    // A consecutive edit needs to know what the file looks like now. Returning a few
    // lines around the change saves a whole read call.
    let mut h = Harness::new();
    h.write_file("a.txt", "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n");
    let out = EditTool
        .execute(
            serde_json::json!({ "path": "a.txt", "old_text": "five", "new_text": "FIVE" }),
            h.ctx(),
        )
        .await
        .unwrap();
    let text = text_of(&out);
    assert!(text.contains("FIVE"), "the new text must show: {text}");
    assert!(text.contains("four"), "a line before must show: {text}");
    assert!(text.contains("six"), "a line after must show: {text}");
    assert!(
        !text.contains("one"),
        "a distant line must not show, or the context is not bounded: {text}"
    );
}

#[tokio::test]
async fn tool_read_and_write_accept_the_familiar_path_name() {
    // The alias reasoning from `edit` applies to every file tool. Inconsistency would
    // be its own trap: a model that learns `file_path` works for one tool would be
    // surprised by the next.
    let mut h = Harness::new();
    WriteTool
        .execute(
            serde_json::json!({ "file_path": "b.txt", "content": "hello\n" }),
            h.ctx(),
        )
        .await
        .unwrap();
    assert_eq!(h.read_file("b.txt"), "hello\n");

    let out = ReadTool
        .execute(serde_json::json!({ "file_path": "b.txt" }), h.ctx())
        .await
        .unwrap();
    assert!(text_of(&out).contains("hello"));
}
