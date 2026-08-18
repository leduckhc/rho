//! SPEC-14 stage T4 red tests: resume and branch.
//!
//! Every test drives the public `session` surface. It must fail on an unimplemented
//! body, never on a type error. It uses `tempfile`, no `sleep`, and no network.

use std::fs;
use std::path::Path;

use rho_core::{
    ContentBlock, Entry, MAX_LINE_BYTES, Message, Record, RecordId, Role, SessionError,
    SessionReader, SessionStore, StoredApproval, StoredSandbox, branch_messages,
    check_resume_permission,
};
use tempfile::tempdir;

// --- helpers ---------------------------------------------------------------

fn temp_store() -> (tempfile::TempDir, SessionStore) {
    let dir = tempdir().expect("temp dir");
    let store = SessionStore::new(dir.path());
    (dir, store)
}

fn message_record(text: &str) -> Record {
    Record::Message {
        message: Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
        },
    }
}

/// Write a session file by hand, header first, then one JSON line per record body.
/// This lets a test build a fixture without the writer, for a truncated tail or a
/// crash tail.
fn write_lines(path: &Path, lines: &[String]) {
    fs::write(path, lines.join("\n")).expect("write fixture");
}

/// A valid header line, minted with serde_json directly, so the fixture needs no codec.
fn header_line(version: u32, approval: &str, sandbox: &str) -> String {
    serde_json::to_string(&serde_json::json!({
        "id": "h",
        "parentId": null,
        "timestamp": "2026-06-25T22:17:00.000Z",
        "type": "session",
        "version": version,
        "cwd": "/work",
        "approval": approval,
        "sandbox": sandbox,
    }))
    .unwrap()
}

/// A message line with a chosen id and parent.
fn message_line(id: &str, parent: Option<&str>, text: &str) -> String {
    serde_json::to_string(&serde_json::json!({
        "id": id,
        "parentId": parent,
        "timestamp": "2026-06-25T22:17:01.000Z",
        "type": "message",
        "message": { "role": "user", "content": [ { "type": "text", "text": text } ] },
    }))
    .unwrap()
}

// --- resume ----------------------------------------------------------------

#[test]
fn resume_reads_every_whole_record() {
    let (_dir, store) = temp_store();
    let mut writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let a = writer.append(message_record("one"), None).expect("a");
    writer.append(message_record("two"), Some(a)).expect("b");
    let read = SessionReader::read(writer.path()).expect("read");
    assert_eq!(
        read.entries.len(),
        2,
        "a clean file loads every appended record"
    );
    assert!(!read.truncated_tail, "a clean file is not truncated");
}

#[test]
fn resume_rebuilds_the_messages_in_file_order() {
    let (dir, _store) = temp_store();
    let path = dir.path().join("s.jsonl");
    write_lines(
        &path,
        &[
            header_line(1, "read-only", "off"),
            message_line("m1", Some("h"), "first"),
            message_line("m2", Some("m1"), "second"),
        ],
    );
    let read = SessionReader::read(&path).expect("read");
    let messages = branch_messages(&read.entries, &RecordId("m2".to_string()));
    let texts: Vec<String> = messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        texts,
        vec!["first", "second"],
        "messages come back in file order"
    );
}

#[test]
fn resume_recovers_after_a_truncated_last_line() {
    let (dir, _store) = temp_store();
    let path = dir.path().join("s.jsonl");
    let good = message_line("m1", Some("h"), "kept");
    let half = good[..good.len() / 2].to_string(); // a cut last line
    write_lines(
        &path,
        &[header_line(1, "read-only", "off"), good.clone(), half],
    );
    let read = SessionReader::read(&path).expect("read");
    assert!(read.truncated_tail, "the half line sets truncated_tail");
    assert_eq!(
        read.entries.len(),
        1,
        "every whole record before the cut is kept"
    );
}

#[test]
fn resume_warns_on_a_truncated_last_line() {
    // The resume path warns when the read reports a truncated tail. The reader exposes
    // the signal the warning is built on.
    let (dir, _store) = temp_store();
    let path = dir.path().join("s.jsonl");
    let good = message_line("m1", Some("h"), "kept");
    let half = good[..good.len() / 3].to_string();
    write_lines(&path, &[header_line(1, "read-only", "off"), good, half]);
    let read = SessionReader::read(&path).expect("read");
    assert!(
        read.truncated_tail,
        "truncated_tail is the one signal the resume warning uses"
    );
}

#[test]
fn resume_after_a_model_change_appends_a_model_change_record() {
    // A new model adds a record and keeps the old ones.
    let (_dir, store) = temp_store();
    let mut writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let a = writer.append(message_record("before"), None).expect("a");
    let reopened = SessionReader::read(writer.path()).expect("read");
    let before = reopened.entries.len();
    writer
        .append(
            Record::ModelChange {
                provider: "openrouter".to_string(),
                model: "new/model".to_string(),
            },
            Some(a),
        )
        .expect("append model change");
    let after = SessionReader::read(writer.path()).expect("read");
    assert_eq!(
        after.entries.len(),
        before + 1,
        "the model change is additive"
    );
    assert!(
        after
            .entries
            .iter()
            .any(|e| matches!(e.record, Record::ModelChange { .. })),
        "a ModelChange record is present"
    );
    assert!(
        after
            .entries
            .iter()
            .any(|e| matches!(&e.record, Record::Message { .. })),
        "the old records survive"
    );
}

#[test]
fn resume_reopens_the_file_with_append_to() {
    let (_dir, store) = temp_store();
    let path = {
        let mut writer = store
            .create("s", Path::new("/work"), "read-only", "off")
            .expect("create");
        writer.append(message_record("first"), None).expect("a");
        writer.path().to_path_buf()
    };
    let mut reopened = store.append_to(&path).expect("append_to");
    reopened
        .append(message_record("later"), None)
        .expect("append after reopen");
    let read = SessionReader::read(&path).expect("read");
    assert_eq!(
        read.entries.len(),
        2,
        "a later append lands after the earlier records"
    );
    let last = read.entries.last().expect("a record");
    let Record::Message { message } = &last.record else {
        panic!("the last record is the message just appended");
    };
    assert!(
        matches!(&message.content[0], ContentBlock::Text { text } if text == "later"),
        "the reopened writer appended after the earlier records"
    );
}

#[test]
fn a_giant_line_does_not_exhaust_memory() {
    // A file with a multi-megabyte line over MAX_LINE_BYTES returns a decode error and
    // never allocates the whole line. The giant line is VALID JSON on purpose. A naive
    // `read_to_string` reader would decode it and return Ok, which is the bug. A bounded
    // reader caps the line and returns a decode error.
    let (dir, _store) = temp_store();
    let path = dir.path().join("s.jsonl");
    let huge = "z".repeat(MAX_LINE_BYTES + 1024);
    write_lines(
        &path,
        &[
            header_line(1, "read-only", "off"),
            message_line("m1", Some("h"), &huge),
        ],
    );
    let result = SessionReader::read(&path);
    assert!(
        matches!(result, Err(SessionError::Decode(_))),
        "a line over MAX_LINE_BYTES is a decode error, never an unbounded allocation"
    );
}

#[test]
fn resume_refuses_an_unknown_version() {
    let (dir, _store) = temp_store();
    let path = dir.path().join("s.jsonl");
    write_lines(&path, &[header_line(9999, "read-only", "off")]);
    let result = SessionReader::read(&path);
    assert!(
        matches!(result, Err(SessionError::Version(9999))),
        "an unknown header version stops the resume with SessionError::Version"
    );
}

#[test]
fn resume_does_not_widen_a_read_only_session() {
    // The header names `read-only`. A run that would use `allow-all` must be refused,
    // because a resume that widens a permission is the D-013 family: an insecure default
    // that nobody chose. The check is a comparison of two stored names, per SPEC-14 8a.
    let (_dir, store) = temp_store();
    let mut writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    writer.append(message_record("x"), None).expect("append");
    let read = SessionReader::read(writer.path()).expect("read");

    let refused = check_resume_permission(
        &read.header,
        StoredApproval::AllowAll,
        StoredSandbox::Off,
        false,
    );
    assert!(
        matches!(
            refused,
            Err(SessionError::Widen {
                field: "approval",
                ..
            })
        ),
        "a read-only session must refuse an allow-all resume, got {refused:?}"
    );
}

#[test]
fn resume_allows_a_narrower_mode() {
    // A run that keeps or narrows both modes needs no flag.
    let (_dir, store) = temp_store();
    let writer = store
        .create("s", Path::new("/work"), "ask", "confined")
        .expect("create");
    let read = SessionReader::read(writer.path()).expect("read");

    let kept = check_resume_permission(
        &read.header,
        StoredApproval::Ask,
        StoredSandbox::Confined,
        false,
    );
    assert!(kept.is_ok(), "the same modes must resume, got {kept:?}");

    let narrowed = check_resume_permission(
        &read.header,
        StoredApproval::ReadOnly,
        StoredSandbox::Strict,
        false,
    );
    assert!(
        narrowed.is_ok(),
        "a narrower run must resume with no flag, got {narrowed:?}"
    );
}

#[test]
fn allow_widen_permits_a_wider_resume() {
    // The override is the one way to widen, and it must be explicit.
    let (_dir, store) = temp_store();
    let writer = store
        .create("s", Path::new("/work"), "read-only", "strict")
        .expect("create");
    let read = SessionReader::read(writer.path()).expect("read");

    let allowed = check_resume_permission(
        &read.header,
        StoredApproval::AllowAll,
        StoredSandbox::Off,
        true,
    );
    assert!(
        allowed.is_ok(),
        "an explicit --allow-widen must permit a wider resume, got {allowed:?}"
    );
}

#[test]
fn an_unknown_mode_name_parses_to_the_strictest_mode() {
    // A name this build does not know must never widen. This is the opposite of
    // ToolKind::Other, which counted an unknown kind as safe and failed open. See D-017.
    assert_eq!(
        StoredApproval::parse("something-new"),
        StoredApproval::ReadOnly,
        "an unknown approval name must be the strictest mode"
    );
    assert_eq!(
        StoredSandbox::parse("something-new"),
        StoredSandbox::Strict,
        "an unknown sandbox name must be the strictest mode"
    );
}

#[test]
fn resume_repairs_an_unmatched_tool_call_after_a_crash() {
    // A file that ends with a ToolCall and no matching ToolResult rebuilds a matched
    // pairing with a synthetic error result, so the next provider request is valid.
    let (dir, _store) = temp_store();
    let path = dir.path().join("s.jsonl");
    let call_line = serde_json::to_string(&serde_json::json!({
        "id": "m1",
        "parentId": "h",
        "timestamp": "2026-06-25T22:17:02.000Z",
        "type": "message",
        "message": {
            "role": "assistant",
            "content": [ { "type": "tool_call", "id": "call-9", "name": "bash", "arguments": {} } ],
        },
    }))
    .unwrap();
    write_lines(&path, &[header_line(1, "read-only", "off"), call_line]);
    let read = SessionReader::read(&path).expect("read");
    let messages = branch_messages(&read.entries, &RecordId("m1".to_string()));
    assert!(
        pairing_is_complete(&messages),
        "a resume repairs a trailing ToolCall with a synthetic error ToolResult"
    );
    assert!(
        result_is_error_for(&messages, "call-9"),
        "the synthetic result is an error that says the call did not finish"
    );
}

/// True when every ToolCall in the message list has a matching ToolResult.
fn pairing_is_complete(messages: &[Message]) -> bool {
    let mut calls = std::collections::HashSet::new();
    let mut results = std::collections::HashSet::new();
    for message in messages {
        for block in &message.content {
            match block {
                ContentBlock::ToolCall { id, .. } => {
                    calls.insert(id.clone());
                }
                ContentBlock::ToolResult { tool_call_id, .. } => {
                    results.insert(tool_call_id.clone());
                }
                _ => {}
            }
        }
    }
    calls.iter().all(|id| results.contains(id))
}

/// True when the message list holds an error ToolResult for the given call id.
fn result_is_error_for(messages: &[Message], call_id: &str) -> bool {
    messages.iter().flat_map(|m| m.content.iter()).any(|b| {
        matches!(
            b,
            ContentBlock::ToolResult { tool_call_id, is_error, .. }
                if tool_call_id == call_id && *is_error
        )
    })
}

// --- branch ----------------------------------------------------------------

#[test]
fn a_branch_keeps_the_original_records() {
    // A branch appends and deletes nothing. The original records survive an append that
    // names an earlier record as its parent.
    let (_dir, store) = temp_store();
    let mut writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let a = writer.append(message_record("root"), None).expect("a");
    let b = writer
        .append(message_record("leaf-1"), Some(a.clone()))
        .expect("b");
    let before = SessionReader::read(writer.path())
        .expect("read")
        .entries
        .len();
    // Branch from `a`, not from the current head `b`.
    let _c = writer
        .append(message_record("leaf-2"), Some(a))
        .expect("branch append");
    let after = SessionReader::read(writer.path()).expect("read");
    assert_eq!(
        after.entries.len(),
        before + 1,
        "a branch only adds a record"
    );
    assert!(
        after.entries.iter().any(|e| e.id == b),
        "the original leaf still lives in the file"
    );
}

#[test]
fn a_branch_links_the_new_record_to_its_parent() {
    let (_dir, store) = temp_store();
    let mut writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let a = writer.append(message_record("root"), None).expect("a");
    let c = writer
        .append(message_record("branch"), Some(a.clone()))
        .expect("branch");
    let read = SessionReader::read(writer.path()).expect("read");
    let entry = read
        .entries
        .iter()
        .find(|e| e.id == c)
        .expect("the branch record");
    assert_eq!(
        entry.parent_id,
        Some(a),
        "the new record names its chosen parent"
    );
}

#[test]
fn branch_messages_walks_one_branch_only() {
    // A head resolves to its own branch, not a sibling. Two leaves share a root. The
    // walk from one leaf must not include the other leaf's message.
    let (dir, _store) = temp_store();
    let path = dir.path().join("s.jsonl");
    write_lines(
        &path,
        &[
            header_line(1, "read-only", "off"),
            message_line("root", Some("h"), "root"),
            message_line("leaf-a", Some("root"), "branch-a"),
            message_line("leaf-b", Some("root"), "branch-b"),
        ],
    );
    let read = SessionReader::read(&path).expect("read");
    let messages = branch_messages(&read.entries, &RecordId("leaf-a".to_string()));
    let texts: Vec<String> = messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(
        texts.contains(&"branch-a".to_string()),
        "the chosen branch is present"
    );
    assert!(
        !texts.contains(&"branch-b".to_string()),
        "the sibling branch is not on this walk"
    );
}

// Keep `Entry` referenced so the import proves the type name, even if a helper changes.
#[allow(dead_code)]
fn _entry_type_is_public(e: Entry) -> Entry {
    e
}
