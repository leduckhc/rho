//! SPEC-sessions stage T4 red tests: storage and the codec.
//!
//! Every test drives the public `session` surface. It must fail on an unimplemented
//! body, never on a type error. It uses `tempfile`. It writes no file in the repository.
//! It reads no real user directory. It uses no `sleep` and no network.

use std::fs;
use std::path::Path;

use rho_core::{
    AgentStopReason, ContentBlock, MAX_RECORD_BYTES, Message, NewSession, Record, RecordId, Role,
    SessionError, SessionId, SessionReader, SessionStore, Usage, decode, encode,
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
            session_id: Some("20260824-000000-0001".to_string()),
            forked_from: None,
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

/// Every file under `dir`, walked recursively. Used to find a spilled sidecar.
fn all_files_under(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            out.extend(all_files_under(&path));
        } else {
            out.push(path);
        }
    }
    out
}

/// True when some file under `dir`, other than the session file, holds `needle`.
/// This is how a test proves the full payload spilled to a sidecar. See D-cap-a-large-tool-result.
fn a_sidecar_holds(dir: &Path, session_file: &Path, needle: &str) -> bool {
    all_files_under(dir).into_iter().any(|path| {
        path != session_file
            && fs::read_to_string(&path)
                .map(|content| content.contains(needle))
                .unwrap_or(false)
    })
}

// --- storage ---------------------------------------------------------------

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
fn n_appends_yield_exactly_n_lines() {
    // The invariant, not one example. For any N, N appends add exactly N lines to the two
    // lines `create` writes: the header, then the `ModelChange` record that states the model
    // on the second line. See `SPEC-session-store-wiring` section 7.
    for n in [0usize, 1, 2, 5, 50] {
        let (_dir, store) = temp_store();
        let mut writer = store
            .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
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
        assert_eq!(
            lines,
            n + 2,
            "a header line, a model line, and exactly {n} appends"
        );
    }
}

#[test]
fn append_returns_a_new_id_each_time() {
    let (_dir, store) = temp_store();
    let mut writer = store
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
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
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
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
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
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
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
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
    let (dir, store) = temp_store();
    let mut writer = store
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
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
                    state: None,
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
        // A byte-truncating writer keeps the line short but corrupts it. So every capped
        // line must still decode as an Entry, and the record kind must survive with
        // valid content. See F4.
        let entry: rho_core::Entry =
            decode(line).expect("a capped line must still decode as an Entry");
        match entry.record {
            Record::Session { .. } => {}
            // `create` writes this on the second line, so a row reads the model with no full
            // read. It carries no payload, so there is nothing to cap.
            Record::ModelChange { .. } => {}
            Record::Message { message } => {
                assert!(
                    !message.content.is_empty(),
                    "a capped message keeps a valid content block, never a corrupt line"
                );
                match &message.content[0] {
                    ContentBlock::Text { .. } => {}
                    ContentBlock::ToolCall { id, name, .. } => {
                        assert!(!id.is_empty(), "a capped ToolCall keeps its id");
                        assert!(!name.is_empty(), "a capped ToolCall keeps its name");
                    }
                    ContentBlock::ToolResult { tool_call_id, .. } => {
                        assert!(
                            !tool_call_id.is_empty(),
                            "a capped ToolResult keeps its tool_call_id"
                        );
                    }
                    other => panic!("unexpected content block in this fixture: {other:?}"),
                }
            }
            other => panic!("unexpected record kind in this fixture: {other:?}"),
        }
    }
    // The three oversize payloads must have spilled their full bytes to a sidecar,
    // rather than sit inline. See D-cap-a-large-tool-result.
    assert!(
        a_sidecar_holds(dir.path(), writer.path(), &big),
        "the full oversize payload must spill to a sidecar under the session directory"
    );
}

#[test]
fn an_oversize_record_of_every_kind_stays_under_the_cap() {
    // An oversize ToolResult, an oversize assistant Message, and an oversize ToolCall
    // each write under the cap, and the ToolCall stays a ToolCall with its tool_call_id.
    let big = "B".repeat(4 * MAX_RECORD_BYTES);
    let (dir, store) = temp_store();
    let mut writer = store
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
        .expect("create");
    let call = Record::Message {
        message: Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolCall {
                id: "call-1".to_string(),
                name: "bash".to_string(),
                arguments: serde_json::json!({ "script": big }),
                state: None,
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
    // The full arguments must spill to a sidecar, never sit inline over the cap. The
    // pairing invariant depends on the ToolCall staying a ToolCall, asserted above. See
    // SPEC-sessions section 3 and D-cap-a-large-tool-result.
    assert!(
        a_sidecar_holds(dir.path(), writer.path(), &big),
        "the full ToolCall arguments must spill to a sidecar under the session directory"
    );
}

#[test]
fn a_non_tool_result_record_over_the_cap_is_capped() {
    // An oversize assistant message text block is capped with a head and a note, the
    // same as a tool result. The written line fits the cap and stays a valid Message.
    let big = "C".repeat(4 * MAX_RECORD_BYTES);
    let (dir, store) = temp_store();
    let mut writer = store
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
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
    let ContentBlock::Text { text: capped } = &message.content[0] else {
        panic!("the head plus the note is a valid text block");
    };
    // The note must state the full byte count, so a reader knows the tail was dropped.
    // A byte-truncating writer drops the tail with no note and fails this. See SPEC-sessions
    // section 3 and D-cap-a-large-tool-result.
    assert!(
        capped.contains(&big.len().to_string()),
        "the note must state the full byte count {}, note was {:?}",
        big.len(),
        &capped[capped.len().saturating_sub(200)..]
    );
    // The full payload must spill to a sidecar under the session directory.
    assert!(
        a_sidecar_holds(dir.path(), writer.path(), &big),
        "the full oversize text must spill to a sidecar under the session directory"
    );
}

#[test]
fn both_codecs_agree_byte_for_byte() {
    // A property over every Record variant. The active codec is deterministic, and it
    // reads back its own output.
    //
    // This test runs under whichever codec the build selects. `fast-json` selects
    // `sonic-rs`, and the default selects `serde_json`. So the cross-codec rule in
    // SPEC-sessions section 3 holds only when the suite runs in both modes. Stage T10 adds
    // that CI matrix. Until T10 lands, do not claim that CI proves it. The golden
    // vector below is what makes the two modes comparable at all: it pins the exact
    // bytes, so a codec that drifts fails in either mode.
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

#[test]
fn the_codec_matches_a_golden_line_in_either_mode() {
    // The cross-codec rule needs a fixed point that does not depend on the build. So one
    // record has its bytes pinned here. A codec that spells a field differently, orders a
    // key differently, or escapes a character differently fails this test, whether the
    // build selects `serde_json` or `sonic-rs`. See SPEC-sessions section 3 and ADR-jsonl-codec.
    let entry = rho_core::Entry {
        id: RecordId("id-1".to_string()),
        parent_id: Some(RecordId("id-0".to_string())),
        timestamp: "2026-06-25T22:17:00.785Z".to_string(),
        record: Record::Stop {
            stop_reason: AgentStopReason::EndTurn,
        },
    };
    let line = encode(&entry).expect("encode");
    assert_eq!(
        line,
        r#"{"id":"id-1","parentId":"id-0","timestamp":"2026-06-25T22:17:00.785Z","type":"stop","reason":"end_turn"}"#,
        "the codec must write the pinned bytes, in either codec mode"
    );
    let round: rho_core::Entry = decode(&line).expect("decode the golden line");
    assert_eq!(round, entry, "the golden line decodes back to the record");
}

// --- typed errors reached through the public surface ------------------------

#[test]
fn reading_a_missing_file_is_an_io_error() {
    // SessionError::Io is reached through the public reader, not by constructing it.
    // Opening a file that does not exist is an io failure, so read returns Io.
    let dir = tempdir().expect("temp dir");
    let missing = dir.path().join("does-not-exist.jsonl");
    let result = SessionReader::read(&missing);
    assert!(
        matches!(result, Err(SessionError::Io(_))),
        "reading a missing session file is a SessionError::Io, got {result:?}"
    );
}

#[test]
fn encoding_an_unserializable_value_is_an_encode_error() {
    // SessionError::Encode is reached through the public `encode` seam, not by
    // constructing it. A map with a non-string key cannot serialize to JSON, so the
    // codec's error must surface as SessionError::Encode.
    let mut map: std::collections::BTreeMap<(i32, i32), i32> = std::collections::BTreeMap::new();
    map.insert((1, 2), 3);
    let result = encode(&map);
    assert!(
        matches!(result, Err(SessionError::Encode(_))),
        "a value the codec cannot encode must surface as SessionError::Encode, got {result:?}"
    );
}

// ---------------------------------------------------------------------------
// The benchmark docs/benchmarks.md quotes. It lived outside the repository.
// ---------------------------------------------------------------------------

/// Measure the append and the resume, so `docs/benchmarks.md` can name a command.
///
/// The page carried five numbers for this path — the total append time, the per-record
/// cost, the line count, the file size, and the full-read time — and its command block
/// held two comments saying the bench "lives outside the repository". So the numbers
/// could not be reproduced from the tree, which the release checklist forbids and which
/// `AGENTS.md` step 13 calls a claim to delete or prove. This proves it.
///
/// It drives the real `SessionWriter` and the real `SessionReader`, never a copy of their
/// logic. The shape matches the original: 20000 appends of a 150-byte assistant message,
/// then one full read.
///
/// It asserts the invariants and prints the timings. A timing is not asserted, because a
/// shared machine makes that flaky, and a flaky benchmark is worse than a slow one.
#[test]
fn an_append_and_resume_benchmark() {
    const RECORDS: usize = 20_000;
    const TEXT_BYTES: usize = 150;

    let (dir, store) = temp_store();
    let mut writer = store
        .create(new_session(&sid(1), Path::new("/work"), "read-only", "off"))
        .expect("create");
    let path = writer.path().to_path_buf();
    let text = "x".repeat(TEXT_BYTES);
    let mut parent = writer.head();

    let started = std::time::Instant::now();
    for _ in 0..RECORDS {
        parent = Some(
            writer
                .append(message_record(&text), parent.clone())
                .expect("append"),
        );
    }
    let append_elapsed = started.elapsed();

    let bytes = fs::metadata(&path).expect("metadata").len();
    let lines = fs::read_to_string(&path).expect("read").lines().count();

    let started = std::time::Instant::now();
    let read = SessionReader::read(&path).expect("read the session");
    let read_elapsed = started.elapsed();

    // The invariants. `create` writes the header and one `ModelChange` line, so the file
    // holds two lines more than the appends. See `n_appends_yield_exactly_n_lines`.
    assert_eq!(lines, RECORDS + 2, "every append is exactly one line");
    assert_eq!(
        read.entries.len(),
        RECORDS + 1,
        "the reader returns every entry after the header"
    );

    let per_record = append_elapsed / RECORDS as u32;
    let mib_per_second = (bytes as f64 / 1_048_576.0) / read_elapsed.as_secs_f64();
    println!("appends: {RECORDS} in {append_elapsed:?}, {per_record:?} per record");
    println!("lines: {lines} for {RECORDS} appends, plus the header");
    println!("file size: {bytes} bytes");
    println!(
        "resume: {} entries in {read_elapsed:?}, {mib_per_second:.1} MiB per second",
        read.entries.len()
    );
    drop(dir);
}
