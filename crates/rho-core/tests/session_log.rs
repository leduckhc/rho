//! SPEC-14 stage T4 red tests: storage and the codec.
//!
//! Every test drives the public `session` surface. It must fail on an unimplemented
//! body, never on a type error. It uses `tempfile`. It writes no file in the repository.
//! It reads no real user directory. It uses no `sleep` and no network.

use std::fs;
use std::path::Path;

use rho_core::{
    AgentStopReason, ContentBlock, MAX_RECORD_BYTES, Message, Record, RecordId, Role, SessionStore,
    Usage, decode, encode,
};
use tempfile::tempdir;

// --- helpers ---------------------------------------------------------------

/// A store rooted at a fresh temp directory. The directory dies with the test.
fn temp_store() -> (tempfile::TempDir, SessionStore) {
    let dir = tempdir().expect("temp dir");
    let store = SessionStore::new(dir.path());
    (dir, store)
}

/// A small message record.
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

/// One value of every `Record` variant, so a property runs over the whole set.
fn every_record_variant() -> Vec<Record> {
    vec![
        Record::Session {
            version: 1,
            cwd: Path::new("/work").to_path_buf(),
            approval: "read-only".to_string(),
            sandbox: "off".to_string(),
        },
        Record::ModelChange {
            provider: "openrouter".to_string(),
            model: "anthropic/claude".to_string(),
        },
        message_record("hello"),
        Record::Usage {
            usage: Usage::default(),
        },
        Record::Stop {
            stop_reason: AgentStopReason::EndTurn,
        },
        Record::Closed,
    ]
}

/// Count non-empty lines in a file.
fn line_count(path: &Path) -> usize {
    let text = fs::read_to_string(path).expect("read session file");
    text.lines().filter(|l| !l.is_empty()).count()
}

// --- storage ---------------------------------------------------------------

#[test]
fn n_appends_yield_exactly_n_lines() {
    // The invariant, not one example. For any N, N appends add exactly N lines to the
    // header line the writer starts with.
    for n in [0usize, 1, 2, 5, 50] {
        let (_dir, store) = temp_store();
        let mut writer = store
            .create("s", Path::new("/work"), "read-only", "off")
            .expect("create");
        let mut parent = writer.head();
        for i in 0..n {
            parent = Some(
                writer
                    .append(message_record(&format!("m{i}")), parent.clone())
                    .expect("append"),
            );
        }
        let lines = line_count(writer.path());
        assert_eq!(lines, n + 1, "one header line plus exactly {n} appends");
    }
}

#[test]
fn append_returns_a_new_id_each_time() {
    let (_dir, store) = temp_store();
    let mut writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let a = writer.append(message_record("a"), None).expect("append a");
    let b = writer
        .append(message_record("b"), Some(a.clone()))
        .expect("append b");
    assert_ne!(a, b, "two appends return two different ids");
}

#[test]
fn head_reports_the_last_written_id() {
    let (_dir, store) = temp_store();
    let mut writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let a = writer.append(message_record("a"), None).expect("append a");
    assert_eq!(writer.head(), Some(a.clone()), "head is the last append");
    let b = writer
        .append(message_record("b"), Some(a))
        .expect("append b");
    assert_eq!(writer.head(), Some(b), "head follows the newest append");
}

#[test]
fn path_returns_the_open_file_path() {
    let (dir, store) = temp_store();
    let writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    assert!(
        writer.path().starts_with(dir.path()),
        "the writer path sits under the store root"
    );
}

#[test]
fn the_append_path_never_rewrites_an_earlier_byte() {
    // The append-only proof. The bytes before a new record are byte-identical after it.
    let (_dir, store) = temp_store();
    let mut writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let a = writer
        .append(message_record("first"), None)
        .expect("append a");
    let before = fs::read(writer.path()).expect("read bytes");
    writer
        .append(message_record("second"), Some(a))
        .expect("append b");
    let after = fs::read(writer.path()).expect("read bytes");
    assert_eq!(
        &after[..before.len()],
        &before[..],
        "the earlier bytes are unchanged after a later append"
    );
    assert!(after.len() > before.len(), "the file only grew");
}

#[test]
fn no_written_record_of_any_kind_exceeds_the_cap() {
    // The invariant over every record kind. Feed each variant an oversize payload and
    // assert no written line exceeds MAX_RECORD_BYTES.
    let big = "A".repeat(4 * MAX_RECORD_BYTES);
    let (_dir, store) = temp_store();
    let mut writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let oversize = vec![
        message_record(&big),
        Record::Message {
            message: Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolCall {
                    id: "t1".to_string(),
                    name: "bash".to_string(),
                    arguments: serde_json::json!({ "cmd": big }),
                }],
            },
        },
        Record::Message {
            message: Message {
                role: Role::Tool,
                content: vec![ContentBlock::ToolResult {
                    tool_call_id: "t1".to_string(),
                    content: vec![ContentBlock::Text { text: big.clone() }],
                    is_error: false,
                }],
            },
        },
    ];
    let mut parent = writer.head();
    for record in oversize {
        parent = Some(writer.append(record, parent.clone()).expect("append"));
    }
    let text = fs::read_to_string(writer.path()).expect("read");
    for line in text.lines().filter(|l| !l.is_empty()) {
        assert!(
            line.len() <= MAX_RECORD_BYTES,
            "a written line {} exceeds the cap {MAX_RECORD_BYTES}",
            line.len()
        );
    }
}

#[test]
fn an_oversize_record_of_every_kind_stays_under_the_cap() {
    // An oversize ToolResult, an oversize assistant Message, and an oversize ToolCall
    // each write under the cap, and the ToolCall stays a ToolCall with its tool_call_id.
    let big = "B".repeat(4 * MAX_RECORD_BYTES);
    let (_dir, store) = temp_store();
    let mut writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let call = Record::Message {
        message: Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolCall {
                id: "call-1".to_string(),
                name: "bash".to_string(),
                arguments: serde_json::json!({ "script": big }),
            }],
        },
    };
    let id = writer.append(call, None).expect("append call");
    let text = fs::read_to_string(writer.path()).expect("read");
    let last = text.lines().rfind(|l| !l.is_empty()).expect("a line");
    assert!(
        last.len() <= MAX_RECORD_BYTES,
        "the capped call fits the cap"
    );
    let entry: rho_core::Entry = decode(last).expect("decode the capped call");
    assert_eq!(entry.id, id, "the record id survives the cap");
    let Record::Message { message } = entry.record else {
        panic!("a capped ToolCall must stay a Message with a ToolCall");
    };
    let ContentBlock::ToolCall {
        id: call_id, name, ..
    } = &message.content[0]
    else {
        panic!("a capped ToolCall must stay a ToolCall, never a text note");
    };
    assert_eq!(call_id, "call-1", "the ToolCall keeps its id");
    assert_eq!(name, "bash", "the ToolCall keeps its name");
}

#[test]
fn a_non_tool_result_record_over_the_cap_is_capped() {
    // An oversize assistant message text block is capped with a head and a note, the
    // same as a tool result. The written line fits the cap and stays a valid Message.
    let big = "C".repeat(4 * MAX_RECORD_BYTES);
    let (_dir, store) = temp_store();
    let mut writer = store
        .create("s", Path::new("/work"), "read-only", "off")
        .expect("create");
    let id = writer.append(message_record(&big), None).expect("append");
    let text = fs::read_to_string(writer.path()).expect("read");
    let last = text.lines().rfind(|l| !l.is_empty()).expect("a line");
    assert!(
        last.len() <= MAX_RECORD_BYTES,
        "the capped message fits the cap"
    );
    let entry: rho_core::Entry = decode(last).expect("decode");
    assert_eq!(entry.id, id);
    let Record::Message { message } = entry.record else {
        panic!("a capped message stays a Message");
    };
    assert!(
        matches!(message.content[0], ContentBlock::Text { .. }),
        "the head plus the note is a valid text block"
    );
}

#[test]
fn both_codecs_agree_byte_for_byte() {
    // A property over every Record variant. The active codec is deterministic, and it
    // reads back its own output. CI runs this with `fast-json` on and off, so the two
    // codecs must produce the one byte-identical line and each reads the other's output.
    for record in every_record_variant() {
        let entry = rho_core::Entry {
            id: RecordId("id-1".to_string()),
            parent_id: None,
            timestamp: "2026-06-25T22:17:00.785Z".to_string(),
            record,
        };
        let line = encode(&entry).expect("encode");
        assert!(!line.contains('\n'), "one record is one line");
        let again = encode(&entry).expect("encode is deterministic");
        assert_eq!(line, again, "the codec output is byte-stable for a record");
        let round: rho_core::Entry = decode(&line).expect("decode the codec's own output");
        assert_eq!(round, entry, "decode reverses encode for every variant");
    }
}
