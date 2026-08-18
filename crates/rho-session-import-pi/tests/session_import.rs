//! SPEC-14 stage T4 red tests: the pi import path, feature F-54, section 9.
//!
//! SPEC-14 section 9 states the importer lives in the crate `rho-session-import-pi`,
//! so these tests live here, not in `rho-core`. Every test fails on an unimplemented
//! body. It uses `tempfile`. It never reads a real `~/.pi` file, so the result does not
//! change per machine. No `sleep`. No network.

use std::fs;
use std::path::Path;

use rho_core::{ContentBlock, Record, RecordId, Role};
use rho_session_import_pi::import_pi_session;
use tempfile::tempdir;

/// The known set that rho drops. See SPEC-14 section 9.
const KNOWN_DROPPABLE: &[&str] = &["thinking_level_change", "session_info", "custom"];

/// Write a pi session fixture. It covers the session header, a user text message, an
/// assistant message with a tool call, a tool result, a `model_change`, a
/// `thinking_level_change`, a `session_info`, and one unknown custom type.
///
/// The function returns the fixture as an ordered list of `(pi_type, id, parent_id)`
/// tuples too, so a test can assert the mapping invariant without hard-coding a count.
fn write_pi_fixture(dir: &Path) -> (std::path::PathBuf, Vec<(String, String, Option<String>)>) {
    let records = [
        serde_json::json!({
            "type": "session", "version": 3, "id": "s0",
            "timestamp": "2026-06-25T22:17:00.000Z", "cwd": "/work"
        }),
        serde_json::json!({
            "type": "message", "id": "m1", "parentId": "s0",
            "timestamp": "2026-06-25T22:17:01.000Z",
            "message": { "role": "user", "content": [ { "type": "text", "text": "hello" } ] }
        }),
        serde_json::json!({
            "type": "message", "id": "m2", "parentId": "m1",
            "timestamp": "2026-06-25T22:17:02.000Z",
            "message": { "role": "assistant", "content": [
                { "type": "toolCall", "id": "call-1", "name": "bash", "arguments": { "cmd": "ls" } } ] }
        }),
        serde_json::json!({
            "type": "message", "id": "m3", "parentId": "m2",
            "timestamp": "2026-06-25T22:17:03.000Z",
            "message": { "role": "toolResult", "content": [
                { "type": "toolResult", "tool_call_id": "call-1",
                  "content": [ { "type": "text", "text": "file.txt" } ], "is_error": false } ] }
        }),
        serde_json::json!({
            "type": "model_change", "id": "m4", "parentId": "m3",
            "timestamp": "2026-06-25T22:17:04.000Z",
            "provider": "openrouter", "model": "anthropic/claude"
        }),
        serde_json::json!({
            "type": "thinking_level_change", "id": "m5", "parentId": "m4",
            "timestamp": "2026-06-25T22:17:05.000Z", "level": "high"
        }),
        serde_json::json!({
            "type": "session_info", "id": "m6", "parentId": "m5",
            "timestamp": "2026-06-25T22:17:06.000Z", "title": "a title"
        }),
        serde_json::json!({
            "type": "custom", "id": "m7", "parentId": "m6",
            "timestamp": "2026-06-25T22:17:07.000Z", "payload": { "x": 1 }
        }),
    ];
    let shape: Vec<(String, String, Option<String>)> = records
        .iter()
        .map(|r| {
            (
                r["type"].as_str().unwrap().to_string(),
                r["id"].as_str().unwrap().to_string(),
                r.get("parentId")
                    .and_then(|p| p.as_str())
                    .map(|s| s.to_string()),
            )
        })
        .collect();
    let lines: Vec<String> = records
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect();
    let path = dir.join("20260625_abc.jsonl");
    fs::write(&path, lines.join("\n")).expect("write pi fixture");
    (path, shape)
}

#[test]
fn every_pi_record_maps_one_to_one_or_drops_from_the_known_set() {
    // The invariant over any pi file: every pi record maps one to one to a rho record,
    // only the known set drops, and the tree shape survives because every kept record
    // keeps its parentId.
    let dir = tempdir().expect("temp dir");
    let (path, shape) = write_pi_fixture(dir.path());

    let entries = import_pi_session(&path).expect("import");

    // Count what should survive: every pi record whose type is not in the known set.
    let kept_shape: Vec<&(String, String, Option<String>)> = shape
        .iter()
        .filter(|(ty, _, _)| !KNOWN_DROPPABLE.contains(&ty.as_str()))
        .collect();

    assert_eq!(
        entries.len(),
        kept_shape.len(),
        "every kept pi record maps one to one, and only the known set drops"
    );

    // No known-droppable id survives, and no unknown record is invented.
    let imported_ids: Vec<String> = entries.iter().map(|e| e.id.0.clone()).collect();
    let expected_ids: Vec<String> = kept_shape.iter().map(|(_, id, _)| id.clone()).collect();
    assert_eq!(
        imported_ids, expected_ids,
        "the id set is exactly the kept set, in order"
    );

    // The tree shape survives: every kept record keeps its parentId.
    for (_, id, parent) in kept_shape {
        let entry = entries
            .iter()
            .find(|e| &e.id.0 == id)
            .expect("kept record present");
        let expected_parent = parent.clone().map(RecordId);
        assert_eq!(
            &entry.parent_id, &expected_parent,
            "the parent pointer survives for {id}"
        );
    }

    // The header maps to a Session record, and the tool call stays a ToolCall.
    assert!(
        matches!(entries[0].record, Record::Session { .. }),
        "the pi session header maps to Record::Session"
    );
    let has_tool_call = entries.iter().any(|e| match &e.record {
        Record::Message { message } => message
            .content
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolCall { .. })),
        _ => false,
    });
    assert!(has_tool_call, "a pi toolCall maps to a rho ToolCall block");
    let has_tool_role = entries.iter().any(|e| {
        matches!(
        &e.record, Record::Message { message } if message.role == Role::Tool)
    });
    assert!(has_tool_role, "a pi toolResult message maps to Role::Tool");
}

#[test]
fn drops_a_pi_thinking_level_change() {
    // A thinking_level_change is not imported.
    let dir = tempdir().expect("temp dir");
    let (path, _shape) = write_pi_fixture(dir.path());
    let entries = import_pi_session(&path).expect("import");
    assert!(
        entries.iter().all(|e| e.id.0 != "m5"),
        "the thinking_level_change record (id m5) is dropped"
    );
}

#[test]
fn leaves_the_original_pi_file_unchanged() {
    // The pi file is byte-identical after import. The assertion is the whole file
    // content, not the modification time.
    let dir = tempdir().expect("temp dir");
    let (path, _shape) = write_pi_fixture(dir.path());
    let before = fs::read(&path).expect("read before");
    let _ = import_pi_session(&path);
    let after = fs::read(&path).expect("read after");
    assert_eq!(before, after, "the import never changes the pi file");
}
