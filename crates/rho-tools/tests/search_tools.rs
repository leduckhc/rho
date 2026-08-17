//! Behaviour tests for the search tools: `glob` and `grep`, plus path
//! confinement and symlink-escape checks.

mod common;

use common::Harness;
use rho_core::{Tool, ToolError, confine};
use rho_tools::{GlobTool, GrepTool};
use std::path::Path;

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
async fn tool_glob_matches_pattern() {
    let mut h = Harness::new();
    h.write_file("src/a.rs", "");
    h.write_file("src/b.rs", "");
    h.write_file("src/c.txt", "");
    let out = GlobTool
        .execute(serde_json::json!({ "pattern": "src/**/*.rs" }), h.ctx())
        .await
        .unwrap();
    let text = text_of(&out);
    assert!(text.contains("src/a.rs"));
    assert!(text.contains("src/b.rs"));
    assert!(!text.contains("c.txt"));
}

#[tokio::test]
async fn tool_glob_does_not_follow_symlink_escape() {
    let mut h = Harness::new();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.rs"), "secret").unwrap();
    std::os::unix::fs::symlink(outside.path(), h.root().join("link")).unwrap();
    let out = GlobTool
        .execute(serde_json::json!({ "pattern": "**/*.rs" }), h.ctx())
        .await
        .unwrap();
    assert!(
        !text_of(&out).contains("secret.rs"),
        "the walk must not follow a symlink out of the root"
    );
}

#[tokio::test]
async fn tool_grep_finds_matches() {
    let mut h = Harness::new();
    h.write_file("a.txt", "one\ntarget here\nthree\n");
    let out = GrepTool
        .execute(serde_json::json!({ "pattern": "target" }), h.ctx())
        .await
        .unwrap();
    let text = text_of(&out);
    assert!(text.contains("a.txt"));
    assert!(text.contains("target here"));
}

#[tokio::test]
async fn tool_grep_honours_gitignore() {
    let mut h = Harness::new();
    h.write_file(".gitignore", "ignored.txt\n");
    h.write_file("ignored.txt", "match me\n");
    h.write_file("kept.txt", "match me\n");
    let out = GrepTool
        .execute(serde_json::json!({ "pattern": "match me" }), h.ctx())
        .await
        .unwrap();
    let text = text_of(&out);
    assert!(text.contains("kept.txt"));
    assert!(!text.contains("ignored.txt"), "gitignore is honoured");
}

#[tokio::test]
async fn tool_grep_does_not_follow_symlink_escape() {
    let mut h = Harness::new();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "match me\n").unwrap();
    std::os::unix::fs::symlink(outside.path(), h.root().join("link")).unwrap();
    let out = GrepTool
        .execute(serde_json::json!({ "pattern": "match me" }), h.ctx())
        .await
        .unwrap();
    assert!(
        !text_of(&out).contains("secret.txt"),
        "grep must not follow a symlink out of the root"
    );
}

// --- Path confinement, proven at the tool boundary ---

#[test]
fn path_confine_allows_child_path() {
    let root = tempfile::tempdir().unwrap();
    let resolved = confine(root.path(), Path::new("file.txt")).unwrap();
    assert!(resolved.starts_with(root.path().canonicalize().unwrap()));
}

#[test]
fn path_confine_rejects_parent_escape() {
    let root = tempfile::tempdir().unwrap();
    let error = confine(root.path(), Path::new("../escape.txt")).unwrap_err();
    assert!(matches!(error, ToolError::PathEscape(_)));
}

#[test]
fn path_confine_rejects_absolute_outside_root() {
    let root = tempfile::tempdir().unwrap();
    let error = confine(root.path(), Path::new("/etc/passwd")).unwrap_err();
    assert!(matches!(error, ToolError::PathEscape(_)));
}

#[test]
fn path_confine_rejects_symlink_escape() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
    let error = confine(root.path(), Path::new("link/secret.txt")).unwrap_err();
    assert!(matches!(error, ToolError::PathEscape(_)));
}
