//! SPEC-14 stage T4 red tests: ephemeral mode, the lifecycle, cancel, and redaction.
//!
//! Every test drives the public `session` surface. It must fail on an unimplemented
//! body, never on a type error. It uses `tempfile`, no `sleep`, and no network.

mod common;

use std::fs;
use std::path::Path;

use rho_core::{
    AgentEvent, AgentStopReason, ContentBlock, Message, Record, RecordId, Role, SessionLog,
    SessionReader, SessionRecorder, SessionStore, StreamEvent, ToolOutput, Usage, branch_messages,
    decode, encode,
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
            role: Role::Assistant,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
        },
    }
}

/// True when the file holds a ToolCall with the given id. A decode failure panics,
/// because a skipped line can hide the very record the pairing check needs.
fn file_contains_tool_call(path: &Path, call_id: &str) -> bool {
    let text = fs::read_to_string(path).expect("read file");
    for line in text.lines().filter(|l| !l.is_empty()) {
        let entry: rho_core::Entry = decode(line)
            .unwrap_or_else(|e| panic!("a session line failed to decode: {e:?}: {line}"));
        if let Record::Message { message } = entry.record {
            for block in message.content {
                if let ContentBlock::ToolCall { id, .. } = block
                    && id == call_id
                {
                    return true;
                }
            }
        }
    }
    false
}

/// True when the message list holds a ToolCall with the given id.
fn messages_contain_tool_call(messages: &[Message], call_id: &str) -> bool {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .any(|b| matches!(b, ContentBlock::ToolCall { id, .. } if id == call_id))
}

/// True when every ToolCall in the file has a matching ToolResult.
fn file_pairing_is_complete(path: &Path) -> bool {
    let text = fs::read_to_string(path).expect("read file");
    let mut calls = std::collections::HashSet::new();
    let mut results = std::collections::HashSet::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        // A skipped line can hide the missing pairing, so a decode failure must panic,
        // never `continue`. See F2 and decision D-049.
        let entry: rho_core::Entry = decode(line)
            .unwrap_or_else(|e| panic!("a session line failed to decode: {e:?}: {line}"));
        if let Record::Message { message } = entry.record {
            for block in message.content {
                match block {
                    ContentBlock::ToolCall { id, .. } => {
                        calls.insert(id);
                    }
                    ContentBlock::ToolResult { tool_call_id, .. } => {
                        results.insert(tool_call_id);
                    }
                    _ => {}
                }
            }
        }
    }
    calls.iter().all(|id| results.contains(id))
}

// --- ephemeral and degrade -------------------------------------------------

#[test]
fn ephemeral_mode_writes_no_file() {
    let dir = tempdir().expect("temp dir");
    let mut log = SessionLog::Off;
    let id = log.record(message_record("x"), None);
    assert!(id.is_none(), "an off log writes nothing and returns no id");
    // An off log holds no path, so a directory it never touches cannot prove anything.
    // The contract that is observable: it stays ephemeral and returns no id for any
    // record kind, including a record that names a parent.
    assert!(log.is_ephemeral(), "an off log is ephemeral");
    let again = log.record(message_record("y"), Some(RecordId("p".to_string())));
    assert!(
        again.is_none(),
        "an off log returns no id for a later record either"
    );
    assert!(
        log.is_ephemeral(),
        "an off log is still ephemeral after a second record"
    );
    let entries: Vec<_> = fs::read_dir(dir.path()).expect("read dir").collect();
    assert!(entries.is_empty(), "Off creates no file in the directory");
}

#[test]
fn a_write_failure_degrades_to_ephemeral_with_a_warning() {
    // A failing writer switches to ephemeral and the run continues. It never ends the
    // run. The writer opens on a path whose parent does not exist, so a write fails.
    let dir = tempdir().expect("temp dir");
    let store = SessionStore::new(dir.path());
    let writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let mut log = SessionLog::File(writer);
    // Make the underlying file unwritable by removing the directory out from under it.
    fs::remove_dir_all(dir.path()).ok();
    // D-041 forbids a silent degrade, so the fallback must emit a warning. Capturing the
    // subscriber makes the warning assertable, so a silent degrade fails this test.
    let logs = common::capture_warnings(|| {
        let _ = log.record(message_record("first"), None);
        // The run continues: a second record still returns without a panic.
        let _ = log.record(message_record("second"), None);
    });
    assert!(
        log.is_ephemeral(),
        "a write failure degrades the log to ephemeral"
    );
    assert!(
        !logs.trim().is_empty(),
        "a write failure must emit a warning, never a silent degrade: log was empty"
    );
}

#[test]
fn session_log_is_ephemeral_reports_off_and_degraded() {
    let off = SessionLog::Off;
    assert!(off.is_ephemeral(), "Off is ephemeral");

    let dir = tempdir().expect("temp dir");
    let store = SessionStore::new(dir.path());
    let writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let mut degraded = SessionLog::File(writer);
    assert!(
        !degraded.is_ephemeral(),
        "a healthy file log is not ephemeral"
    );
    fs::remove_dir_all(dir.path()).ok();
    let _ = degraded.record(message_record("x"), None);
    assert!(
        degraded.is_ephemeral(),
        "the log is ephemeral after a write failure"
    );
}

#[test]
fn recorder_is_ephemeral_follows_its_log() {
    let recorder = SessionRecorder::new(SessionLog::Off);
    assert!(recorder.is_ephemeral(), "a recorder over Off is ephemeral");

    let (_dir, store) = temp_store();
    let writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let recorder = SessionRecorder::new(SessionLog::File(writer));
    assert!(
        !recorder.is_ephemeral(),
        "a recorder over a healthy file is not ephemeral"
    );
}

// --- lifecycle -------------------------------------------------------------

#[test]
fn close_writes_a_closed_record() {
    let (_dir, store) = temp_store();
    let mut writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    writer.append(message_record("x"), None).expect("append");
    writer.close().expect("close");
    let read = SessionReader::read(writer.path()).expect("read");
    assert!(
        read.entries
            .iter()
            .any(|e| matches!(e.record, Record::Closed)),
        "close appends the Closed record"
    );
}

#[test]
fn close_is_idempotent() {
    let (_dir, store) = temp_store();
    let mut writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    writer.append(message_record("x"), None).expect("append");
    writer.close().expect("first close");
    let after_first = fs::read(writer.path()).expect("read");
    writer.close().expect("second close");
    let after_second = fs::read(writer.path()).expect("read");
    assert_eq!(after_first, after_second, "a second close writes nothing");
}

#[test]
fn cancel_keeps_the_session_open() {
    // After a cancel the session accepts a new prompt. The recorder writes a stop, then
    // still records a following prompt.
    let (_dir, store) = temp_store();
    let writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let mut recorder = SessionRecorder::new(SessionLog::File(writer));
    recorder.record_cancel();
    let id = recorder.record_prompt(&[ContentBlock::Text {
        text: "again".to_string(),
    }]);
    assert!(
        id.is_some(),
        "a cancelled session still records the next prompt"
    );
}

#[test]
fn cancel_writes_a_stop_record() {
    let (_dir, store) = temp_store();
    let writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let path = writer.path().to_path_buf();
    let mut recorder = SessionRecorder::new(SessionLog::File(writer));
    recorder.record_cancel();
    let read = SessionReader::read(&path).expect("read");
    let stops: Vec<_> = read
        .entries
        .iter()
        .filter(|e| matches!(&e.record, Record::Stop { stop_reason } if *stop_reason == AgentStopReason::Canceled))
        .collect();
    assert_eq!(
        stops.len(),
        1,
        "a cancel appends exactly one Stop with Canceled"
    );
}

#[test]
fn cancel_leaves_no_half_written_tool_pairing() {
    // The invariant holds on all three paths: after a cancel, after a crash with an
    // unmatched call, and after a truncated tail that dropped a result line.

    // Path 1: a cancel during an open tool call.
    let (_dir, store) = temp_store();
    let writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let path = writer.path().to_path_buf();
    let mut recorder = SessionRecorder::new(SessionLog::File(writer));
    recorder.observe(&AgentEvent::TurnStart);
    recorder.observe(&AgentEvent::ToolStart {
        id: "call-1".to_string(),
        name: "bash".to_string(),
        kind: rho_core::ToolKind::Read,
    });
    recorder.record_cancel();
    assert!(
        file_contains_tool_call(&path, "call-1"),
        "the open tool call must be on disk before the pairing check; a record_cancel \
         that writes nothing would make the completeness check vacuously true"
    );
    assert!(
        file_pairing_is_complete(&path),
        "a cancel completes every open tool pairing on disk"
    );

    // Path 2: a crash left a trailing ToolCall with no result. The rebuild repairs it.
    let dir2 = tempdir().expect("temp dir");
    let crash = dir2.path().join("crash.jsonl");
    let header = serde_json::to_string(&serde_json::json!({
        "id": "h", "parentId": null, "timestamp": "2026-06-25T22:17:00.000Z",
        "type": "session", "version": 1, "cwd": "/work",
        "approval": "read-only", "sandbox": "off",
    }))
    .unwrap();
    let call = serde_json::to_string(&serde_json::json!({
        "id": "m1", "parentId": "h", "timestamp": "2026-06-25T22:17:02.000Z",
        "type": "message",
        "message": { "role": "assistant", "content": [
            { "type": "tool_call", "id": "call-2", "name": "bash", "arguments": {} } ] },
    }))
    .unwrap();
    fs::write(&crash, format!("{header}\n{call}")).expect("write crash fixture");
    let read = SessionReader::read(&crash).expect("read");
    let messages = branch_messages(&read.entries, &RecordId("m1".to_string()));
    assert!(
        messages_contain_tool_call(&messages, "call-2"),
        "the unmatched call must be present before the pairing check, or it is vacuous"
    );
    assert!(
        messages_pairing_is_complete(&messages),
        "a crash tail with an unmatched call rebuilds a matched pairing"
    );

    // Path 3: a truncated tail dropped the ToolResult line. The rebuild repairs it too.
    let dir3 = tempdir().expect("temp dir");
    let trunc = dir3.path().join("trunc.jsonl");
    let result = serde_json::to_string(&serde_json::json!({
        "id": "m2", "parentId": "m1", "timestamp": "2026-06-25T22:17:03.000Z",
        "type": "message",
        "message": { "role": "tool", "content": [
            { "type": "tool_result", "tool_call_id": "call-2", "content": [], "is_error": false } ] },
    }))
    .unwrap();
    let half = &result[..result.len() / 2];
    fs::write(&trunc, format!("{header}\n{call}\n{half}")).expect("write trunc fixture");
    let read = SessionReader::read(&trunc).expect("read");
    let messages = branch_messages(&read.entries, &RecordId("m1".to_string()));
    assert!(
        messages_contain_tool_call(&messages, "call-2"),
        "the unmatched call must be present before the pairing check, or it is vacuous"
    );
    assert!(
        messages_pairing_is_complete(&messages),
        "a dropped result line rebuilds a matched pairing"
    );
}

/// True when every ToolCall in a message list has a matching ToolResult.
fn messages_pairing_is_complete(messages: &[Message]) -> bool {
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

#[test]
fn a_usage_record_round_trips() {
    // A Record::Usage encodes and decodes back to the same Usage, so the usage counters
    // survive a resume.
    let usage = Usage {
        input_tokens: 100,
        output_tokens: 40,
        cache_read_tokens: 10,
        cache_write_tokens: 5,
        cost_usd: Some(0.0123),
    };
    let entry = rho_core::Entry {
        id: RecordId("u1".to_string()),
        parent_id: None,
        timestamp: "2026-06-25T22:17:00.000Z".to_string(),
        record: Record::Usage { usage },
    };
    let line = encode(&entry).expect("encode");
    let round: rho_core::Entry = decode(&line).expect("decode");
    let Record::Usage { usage: back } = round.record else {
        panic!("a usage record decodes to a usage record");
    };
    assert_eq!(back, usage, "the usage counters survive a round trip");
}

#[test]
fn list_reads_only_the_first_line() {
    // list on many files reads one line each. The summary comes from the header record.
    let (_dir, store) = temp_store();
    for i in 0..5 {
        let mut writer = store
            .create(&format!("s{i}"), Path::new("/work"), "read-only", "off")
            .expect("create");
        writer.append(message_record("body"), None).expect("append");
    }
    let summaries = store.list().expect("list");
    assert_eq!(summaries.len(), 5, "list finds every session file");
    let ids: std::collections::HashSet<String> =
        summaries.iter().map(|s| s.session_id.clone()).collect();
    for i in 0..5 {
        assert!(
            ids.contains(&format!("s{i}")),
            "the summary carries the session id s{i}"
        );
    }
    for summary in &summaries {
        assert_eq!(
            summary.cwd,
            Path::new("/work"),
            "the summary comes from the header"
        );
        assert!(summary.size_bytes > 0, "the summary carries file metadata");
        assert!(
            summary.path.exists(),
            "the summary path points at a real session file, not an empty default"
        );
    }
}

#[test]
fn delete_removes_the_file_and_its_branches() {
    let (_dir, store) = temp_store();
    let path = {
        let mut writer = store
            .create("s", Path::new("/work"), "read-only", "off")
            .expect("create");
        let a = writer.append(message_record("root"), None).expect("a");
        writer
            .append(message_record("branch"), Some(a))
            .expect("branch");
        writer.path().to_path_buf()
    };
    store.delete("s").expect("delete");
    assert!(!path.exists(), "the file and every branch in it are gone");
}

#[test]
fn delete_does_not_touch_a_fork() {
    // A fork is a separate file with its own id, so delete on the parent leaves it.
    let (_dir, store) = temp_store();
    let (parent_path, from_id) = {
        let mut writer = store
            .create("s", Path::new("/work"), "read-only", "off")
            .expect("create");
        let a = writer.append(message_record("root"), None).expect("a");
        (writer.path().to_path_buf(), a)
    };
    let fork_path = {
        let fork = store.fork(&parent_path, &from_id, "forked").expect("fork");
        fork.path().to_path_buf()
    };
    store.delete("s").expect("delete parent");
    assert!(fork_path.exists(), "a fork survives a delete of its parent");
}

#[test]
fn fork_copies_the_branch_and_keeps_the_original() {
    let (_dir, store) = temp_store();
    let (parent_path, from_id) = {
        let mut writer = store
            .create("s", Path::new("/work"), "read-only", "off")
            .expect("create");
        let a = writer.append(message_record("root"), None).expect("a");
        (writer.path().to_path_buf(), a)
    };
    let original_bytes = fs::read(&parent_path).expect("read original");
    let fork_path = {
        let fork = store.fork(&parent_path, &from_id, "forked").expect("fork");
        fork.path().to_path_buf()
    };
    let after = fs::read(&parent_path).expect("read original again");
    assert_eq!(
        after, original_bytes,
        "the original file is byte-identical after a fork"
    );
    let forked = SessionReader::read(&fork_path).expect("read fork");
    assert!(
        !forked.entries.is_empty(),
        "the new file holds the copied branch"
    );
}

// --- redaction, the security core ------------------------------------------

#[test]
fn no_credential_reaches_the_file() {
    // A message with a secret-shaped tool argument writes a masked value, and the raw
    // value never appears in the file.
    let (_dir, store) = temp_store();
    let writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let path = writer.path().to_path_buf();
    let mut recorder = SessionRecorder::new(SessionLog::File(writer));
    let secret = "sk-live-super-secret-value-1234567890";
    recorder.observe(&AgentEvent::ToolEnd {
        id: "call-1".to_string(),
        output: ToolOutput {
            content: vec![ContentBlock::ToolResult {
                tool_call_id: "call-1".to_string(),
                content: vec![ContentBlock::Text {
                    text: "ok".to_string(),
                }],
                is_error: false,
            }],
            is_error: false,
        },
    });
    recorder.record_prompt(&[ContentBlock::ToolCall {
        id: "call-2".to_string(),
        name: "http".to_string(),
        arguments: serde_json::json!({ "api_key": secret }),
    }]);
    let bytes = fs::read(&path).expect("read file");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        !text.contains(secret),
        "the raw credential never reaches the file"
    );
    assert!(
        text.contains("***"),
        "the masked value is written in its place"
    );
}

#[test]
fn a_redacted_tool_argument_is_masked_on_the_way_in() {
    // record_prompt and observe redact arguments through rho-redact before the record is
    // written.
    let (_dir, store) = temp_store();
    let writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let path = writer.path().to_path_buf();
    let mut recorder = SessionRecorder::new(SessionLog::File(writer));
    let secret = "AKIA-secret-access-key-value";
    recorder.observe(&AgentEvent::Stream(StreamEvent::Usage(Usage::default())));
    recorder.record_prompt(&[ContentBlock::ToolCall {
        id: "c1".to_string(),
        name: "aws".to_string(),
        arguments: serde_json::json!({ "access_key": secret, "region": "eu-west-1" }),
    }]);
    let text = fs::read_to_string(&path).expect("read");
    assert!(
        !text.contains(secret),
        "the argument is masked on the way in"
    );
    assert!(text.contains("eu-west-1"), "a non-secret argument is kept");
}

#[test]
fn redact_json_secrets_masks_a_secret_keyed_field() {
    // rho_redact::redact_json_secrets masks the value under a key that looks_like_a_secret
    // flags, and keeps every other value, every key name, and the tree shape. This guards
    // the new function in SPEC-14 section 5a.
    let input = serde_json::json!({
        "api_key": "sk-live-1234567890",
        "region": "eu-west-1",
        "nested": { "password": "hunter2", "keep": "value" },
        "list": [ { "auth_token": "abc" }, { "plain": "ok" } ],
    });
    let out = rho_redact::redact_json_secrets(&input);
    assert_eq!(
        out["api_key"],
        serde_json::json!("***"),
        "a flagged key is masked"
    );
    assert_eq!(
        out["region"],
        serde_json::json!("eu-west-1"),
        "an unflagged value is kept"
    );
    assert_eq!(
        out["nested"]["password"],
        serde_json::json!("***"),
        "recursion into objects"
    );
    assert_eq!(
        out["nested"]["keep"],
        serde_json::json!("value"),
        "the sibling value is kept"
    );
    assert_eq!(
        out["list"][0]["auth_token"],
        serde_json::json!("***"),
        "recursion into arrays"
    );
    assert_eq!(
        out["list"][1]["plain"],
        serde_json::json!("ok"),
        "the tree shape survives"
    );
    // Every key name survives.
    assert!(out.get("api_key").is_some(), "the flagged key name is kept");
    assert!(
        out["nested"].get("password").is_some(),
        "the nested key name is kept"
    );
}
