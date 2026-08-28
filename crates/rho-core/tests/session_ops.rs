//! SPEC-sessions stage T4 red tests: ephemeral mode, the lifecycle, cancel, and redaction.
//!
//! Every test drives the public `session` surface. It must fail on an unimplemented
//! body, never on a type error. It uses `tempfile`, no `sleep`, and no network.

mod common;

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rho_core::{
    AgentEvent, AgentStopReason, ContentBlock, Message, NewSession, Record, RecordId, Role,
    SessionId, SessionLog, SessionReader, SessionRecorder, SessionStore, SessionWriter,
    StreamEvent, ToolOutput, Usage, branch_messages, decode, encode,
};
use tempfile::tempdir;

// --- sink seams ------------------------------------------------------------
//
// The writer holds one open sink for the life of the session, and writes one record as
// one write. The controller measured a held handle at ~1us per record against ~17.5us
// for a reopen per record over 20000 records on macos arm64, so a per-record reopen was
// rejected. A held handle cannot be forced to fail by removing the directory, because an
// already-open file descriptor keeps succeeding on Unix after its path is unlinked. So
// the degrade tests inject a sink that fails on demand, through `SessionWriter::with_sink`,
// the same style of seam as `SessionReader::read_from`.

/// A sink that counts write calls, so a test can prove one write per record.
struct CountingSink {
    writes: Arc<AtomicUsize>,
}

impl Write for CountingSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A sink that fails on the Nth write, so a test can force a write failure without
/// removing the session directory. See the note above.
struct FailingSink {
    writes: usize,
    fail_at: usize,
}

impl Write for FailingSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        if self.writes >= self.fail_at {
            return Err(io::Error::other("the sink failed on purpose"));
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

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
        // never `continue`. See F2 and decision D-cancel-keeps-the-session-open.
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

/// A stable session id. Minting takes a time and a suffix, so no test sleeps.
fn sid(suffix: u16) -> SessionId {
    SessionId::mint(1_756_000_000_000, suffix)
}

/// The create request these tests use.
///
/// `SessionStore::create` takes one struct, because a four-argument constructor already hid a
/// fake model id and an approve-all policy in this project. See
/// `D-no-four-argument-session-new`. It writes the header and one `ModelChange` record, so
/// every created file starts with two lines.
fn new_session<'a>(
    id: &'a SessionId,
    cwd: &'a std::path::Path,
    approval: &'a str,
    sandbox: &'a str,
) -> NewSession<'a> {
    NewSession {
        id,
        cwd,
        approval,
        sandbox,
        provider: "testkit",
        model: "test-model",
        forked_from: None,
    }
}

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
    // A failing sink switches the log to ephemeral and the run continues. It never ends
    // the run. This injects a sink that fails on the second write, rather than removing
    // the session directory: the writer now holds one open handle, and a held handle
    // keeps succeeding after its path is unlinked, so the directory trick could not
    // force a failure. See the sink-seam note at the top of this file. D-write-failure-degrades forbids a
    // silent degrade, so the fallback must emit a warning.
    let writer = SessionWriter::with_sink(
        "degrade.jsonl",
        Box::new(FailingSink {
            writes: 0,
            fail_at: 2,
        }),
    );
    let mut log = SessionLog::File(writer);
    let logs = common::capture_warnings(|| {
        let _ = log.record(message_record("first"), None); // write 1 succeeds
        // The run continues: the second record fails the sink, degrades, and returns
        // without a panic.
        let _ = log.record(message_record("second"), None); // write 2 fails
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

    // A file log is healthy until a write fails. This injects a sink that fails on its
    // first write, rather than removing the directory, because the writer holds one
    // open handle now. See the sink-seam note at the top of this file.
    let writer = SessionWriter::with_sink(
        "degrade2.jsonl",
        Box::new(FailingSink {
            writes: 0,
            fail_at: 1,
        }),
    );
    let mut degraded = SessionLog::File(writer);
    assert!(
        !degraded.is_ephemeral(),
        "a healthy file log is not ephemeral"
    );
    let _ = degraded.record(message_record("x"), None); // write 1 fails
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
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
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
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
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
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
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
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
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
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
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
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
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
    let messages = branch_messages(
        &read.entries,
        &RecordId("m1".to_string()),
        Some(&read.header_id),
    )
    .expect("a whole chain rebuilds");
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
    let messages = branch_messages(
        &read.entries,
        &RecordId("m1".to_string()),
        Some(&read.header_id),
    )
    .expect("a whole chain rebuilds");
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
fn delete_removes_the_file_and_its_branches() {
    let (_dir, store) = temp_store();
    let path = {
        let mut writer = store
            .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
            .expect("create");
        let a = writer.append(message_record("root"), None).expect("a");
        writer
            .append(message_record("branch"), Some(a))
            .expect("branch");
        writer.path().to_path_buf()
    };
    store.delete(&sid(1)).expect("delete");
    assert!(!path.exists(), "the file and every branch in it are gone");
}

#[test]
fn delete_does_not_touch_a_fork() {
    // A fork is a separate file with its own id, so delete on the parent leaves it.
    let (_dir, store) = temp_store();
    let (parent_path, from_id) = {
        let mut writer = store
            .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
            .expect("create");
        let a = writer.append(message_record("root"), None).expect("a");
        (writer.path().to_path_buf(), a)
    };
    let fork_path = {
        let fork = store.fork(&parent_path, &from_id, &sid(2)).expect("fork");
        fork.path().to_path_buf()
    };
    store.delete(&sid(1)).expect("delete parent");
    assert!(fork_path.exists(), "a fork survives a delete of its parent");
}

#[test]
fn fork_copies_the_branch_and_keeps_the_original() {
    let (_dir, store) = temp_store();
    let (parent_path, from_id) = {
        let mut writer = store
            .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
            .expect("create");
        let a = writer.append(message_record("root"), None).expect("a");
        (writer.path().to_path_buf(), a)
    };
    let original_bytes = fs::read(&parent_path).expect("read original");
    let fork_path = {
        let fork = store.fork(&parent_path, &from_id, &sid(2)).expect("fork");
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
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
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
        state: None,
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
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
        .expect("create");
    let path = writer.path().to_path_buf();
    let mut recorder = SessionRecorder::new(SessionLog::File(writer));
    let secret = "AKIA-secret-access-key-value";
    recorder.observe(&AgentEvent::Stream(StreamEvent::Usage(Usage::default())));
    recorder.record_prompt(&[ContentBlock::ToolCall {
        id: "c1".to_string(),
        name: "aws".to_string(),
        arguments: serde_json::json!({ "access_key": secret, "region": "eu-west-1" }),
        state: None,
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
    // the new function in SPEC-sessions section 5a.
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

// --- the write contract ----------------------------------------------------

#[test]
fn the_append_path_writes_once_per_record() {
    // SPEC-sessions section 3: a buffered writer, one write per record. A counting sink proves
    // exactly one write per append, so a writer that batches two records into one write
    // fails this. The sink counts write calls; the writer builds the line and its
    // newline into one buffer, so one record is one write.
    let writes = Arc::new(AtomicUsize::new(0));
    let sink = CountingSink {
        writes: Arc::clone(&writes),
    };
    let mut writer = SessionWriter::with_sink("count.jsonl", Box::new(sink));
    for i in 0..5 {
        writer
            .append(message_record(&format!("m{i}")), None)
            .expect("append");
    }
    assert_eq!(
        writes.load(Ordering::SeqCst),
        5,
        "five records must cause exactly five writes, never a batched write"
    );
}

#[test]
fn a_session_file_holds_one_timestamp_format() {
    // Decision: epoch milliseconds as a decimal string, everywhere. A pi import converts
    // an RFC 3339 timestamp into this format, so a resumed import never holds two formats
    // in one file. Assert every line's timestamp, over every record kind, parses as u64.
    let (_dir, store) = temp_store();
    let mut writer = store
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
        .expect("create"); // Session header
    let a = writer
        .append(
            Record::ModelChange {
                provider: "openrouter".to_string(),
                model: "anthropic/claude".to_string(),
            },
            None,
        )
        .expect("model change");
    let b = writer
        .append(message_record("hello"), Some(a))
        .expect("message");
    let c = writer
        .append(
            Record::Usage {
                usage: Usage::default(),
            },
            Some(b),
        )
        .expect("usage");
    writer
        .append(
            Record::Stop {
                stop_reason: AgentStopReason::EndTurn,
            },
            Some(c),
        )
        .expect("stop");
    writer.close().expect("close"); // Closed record

    let text = fs::read_to_string(writer.path()).expect("read");
    let mut lines = 0;
    for line in text.lines().filter(|l| !l.is_empty()) {
        lines += 1;
        let value: serde_json::Value = serde_json::from_str(line).expect("a session line is json");
        let timestamp = value
            .get("timestamp")
            .and_then(|t| t.as_str())
            .expect("every record carries a timestamp string");
        assert!(
            timestamp.parse::<u64>().is_ok(),
            "the timestamp {timestamp:?} must be epoch milliseconds, one format per file"
        );
    }
    // Seven, not six. `create` writes the `ModelChange` record itself now, so a row reads the
    // model from a bounded head read. See `SPEC-session-store-wiring` section 7.
    assert_eq!(lines, 7, "the fixture covers every record kind");
}

// --- what a real drive of the operations found -------------------------------
//
// The controller ran the operations for real, in a scratch binary under /tmp: create
// three sessions, close each one, list, resume, widen, fork, delete, then read a missing
// file twice. Two faults came out that no test covered. See SPEC-sessions section 8.

#[test]
fn append_to_a_closed_session_reopens_it_and_keeps_closed_last() {
    // A closed file ends with a `Closed` record, and SPEC-sessions calls that the last record.
    // A real drive appended two records after it, so the file held `Closed` in the middle
    // and any reader would report a closed session that kept talking. A resume must
    // either refuse, or state that the session reopened. It states it.
    let (_dir, store) = temp_store();
    let mut writer = store
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
        .expect("create");
    writer
        .append(message_record("before"), None)
        .expect("append");
    writer.close().expect("close");

    let mut reopened = store.append_to(writer.path()).expect("append_to");
    reopened
        .append(message_record("after"), None)
        .expect("append after a close");

    let text = fs::read_to_string(writer.path()).expect("read the file");
    let kinds: Vec<String> = text
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).expect("a json line");
            value
                .get("type")
                .and_then(|t| t.as_str())
                .expect("every record names its type")
                .to_string()
        })
        .collect();

    // The invariant: a `closed` record is never followed by anything except a `reopened`
    // record. So a reader can always tell a closed session from a reopened one.
    for (index, kind) in kinds.iter().enumerate() {
        if kind == "closed" && index + 1 < kinds.len() {
            assert_eq!(
                kinds[index + 1],
                "reopened",
                "a closed record may only be followed by a reopened record, got {kinds:?}"
            );
        }
    }
    assert!(
        kinds.contains(&"reopened".to_string()),
        "a resume after a close must record that the session reopened, got {kinds:?}"
    );
}

#[test]
fn an_io_error_names_the_path() {
    // A real drive read a missing file and got "No such file or directory (os error 2)".
    // The message did not say which file, so a user with 500 sessions learns nothing. A
    // config error already names its path, and a session error must do the same.
    let (dir, _store) = temp_store();
    let missing = dir.path().join("no-such-session.jsonl");
    let error = SessionReader::read(&missing).expect_err("a missing file is an error");
    let message = error.to_string();
    assert!(
        message.contains("no-such-session.jsonl"),
        "an io error must name the path that failed, got {message:?}"
    );
}
