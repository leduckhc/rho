//! Tests for the interactive recording seam, `rho_tui::record_turn`.
//!
//! The seam folds one loop transition into the session file. The event loop calls it, and
//! never inlines a recorder call. These tests drive the seam directly, because the five
//! loop-arm calls own the terminal and a cargo test cannot reach them. The pty drive in
//! `docs/verification/` covers the loop wiring. See
//! `SPEC-the-interactive-session-records-itself` sections 11 and 14.
//!
//! Every test isolates the filesystem with `tempfile`. None sleeps, and none touches the
//! network.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rho_core::{
    AgentEvent, AgentStopReason, ContentBlock, NewSession, Record, Role, SessionId, SessionLog,
    SessionReader, SessionRecorder, SessionStore, SessionWriter, StopReason, ToolKind,
};
use rho_tui::{TuiState, TurnRecord, record_turn};

/// A stable session id. Minting takes a time and a suffix, so no test sleeps.
fn sid(suffix: u16) -> SessionId {
    SessionId::mint(1_756_000_000_000, suffix)
}

/// A store under a temporary directory.
fn temp_store() -> (tempfile::TempDir, SessionStore) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = SessionStore::new(dir.path().join("sessions"));
    (dir, store)
}

/// A file-backed recorder, and the path it writes.
fn file_recorder(store: &SessionStore, suffix: u16) -> (SessionRecorder, PathBuf) {
    let id = sid(suffix);
    let writer = store
        .create(NewSession {
            id: &id,
            cwd: Path::new("/work"),
            approval: "allow-all",
            sandbox: "off",
            provider: "anthropic",
            model: "sonnet-4.5",
            forked_from: None,
        })
        .expect("a created session");
    let path = writer.path().to_path_buf();
    (SessionRecorder::new(SessionLog::File(writer)), path)
}

/// Every record in a file, in file order.
fn records(path: &Path) -> Vec<Record> {
    SessionReader::read(path)
        .expect("the file reads back")
        .entries
        .into_iter()
        .map(|entry| entry.record)
        .collect()
}

/// The events of one assistant turn that answers with text.
fn text_turn_events(text: &str) -> Vec<AgentEvent> {
    vec![
        AgentEvent::TurnStart,
        AgentEvent::Stream(rho_core::StreamEvent::MessageStart {
            role: Role::Assistant,
        }),
        AgentEvent::Stream(rho_core::StreamEvent::TextStart { index: 0 }),
        AgentEvent::Stream(rho_core::StreamEvent::TextDelta {
            index: 0,
            delta: text.to_string(),
        }),
        AgentEvent::Stream(rho_core::StreamEvent::TextEnd { index: 0 }),
        AgentEvent::TurnEnd {
            stop_reason: StopReason::EndTurn,
        },
    ]
}

/// A user prompt of one text block.
fn prompt(text: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text {
        text: text.to_string(),
    }]
}

/// The role of one message record, or `None` for a non-message record.
fn message_role(record: &Record) -> Option<Role> {
    match record {
        Record::Message { message } => Some(message.role),
        _ => None,
    }
}

/// A sink that fails every write, and counts the calls, so a test proves the degrade path.
#[derive(Clone)]
struct FailingCountingSink {
    writes: Arc<AtomicUsize>,
}

impl Write for FailingCountingSink {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        Err(io::Error::other("the sink failed on purpose"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// How many error rows the transcript holds.
fn error_rows(state: &TuiState) -> usize {
    state
        .live_rows()
        .iter()
        .filter(|row| matches!(row, rho_tui::Row::Error { .. }))
        .count()
}

/// The seam records the prompt, folds the turn, and writes the close, in that order. A seam
/// that skips the prompt fails this test.
#[test]
fn the_turn_seam_records_prompt_events_and_close_in_order() {
    let (_dir, store) = temp_store();
    let (recorder, path) = file_recorder(&store, 0x0001);
    let mut recorder = Some(recorder);
    let mut state = TuiState::default();

    let input = prompt("fix the bug");
    record_turn(&mut recorder, &mut state, TurnRecord::Prompt(&input));
    for event in text_turn_events("the fix is on line 42") {
        record_turn(&mut recorder, &mut state, TurnRecord::Event(&event));
    }
    record_turn(&mut recorder, &mut state, TurnRecord::Close);

    let ordered = records(&path);
    let user = ordered
        .iter()
        .position(|r| message_role(r) == Some(Role::User))
        .expect("a user message");
    let assistant = ordered
        .iter()
        .position(|r| message_role(r) == Some(Role::Assistant))
        .expect("an assistant message");
    let close = ordered
        .iter()
        .position(|r| matches!(r, Record::Closed))
        .expect("a close");
    assert!(
        user < assistant && assistant < close,
        "the file holds the user prompt, then the assistant answer, then the close: {ordered:?}"
    );
}

/// A `Selection` step writes one `ModelChange` whose provider and model match the step.
#[test]
fn a_selection_step_records_a_model_change() {
    let (_dir, store) = temp_store();
    let (recorder, path) = file_recorder(&store, 0x0002);
    let mut recorder = Some(recorder);
    let mut state = TuiState::default();

    record_turn(
        &mut recorder,
        &mut state,
        TurnRecord::Selection {
            provider: "anthropic",
            model: "opus-4.1",
        },
    );

    // The store writes one ModelChange at create time. The step appends a second.
    let changes: Vec<_> = records(&path)
        .into_iter()
        .filter_map(|r| match r {
            Record::ModelChange { provider, model } => Some((provider, model)),
            _ => None,
        })
        .collect();
    assert_eq!(
        changes.last(),
        Some(&("anthropic".to_string(), "opus-4.1".to_string())),
        "the step appends a ModelChange with the step's provider and model: {changes:?}"
    );
    assert_eq!(
        changes
            .iter()
            .filter(|(_, model)| model == "opus-4.1")
            .count(),
        1,
        "the step writes exactly one ModelChange for the new model: {changes:?}"
    );
}

/// A `None` recorder is a no-op, and it never panics.
#[test]
fn an_absent_recorder_records_nothing() {
    let mut recorder: Option<SessionRecorder> = None;
    let mut state = TuiState::default();
    let input = prompt("hello");
    record_turn(&mut recorder, &mut state, TurnRecord::Prompt(&input));
    record_turn(&mut recorder, &mut state, TurnRecord::Cancel);
    record_turn(&mut recorder, &mut state, TurnRecord::Close);
    assert!(
        state.live_rows().is_empty(),
        "an absent recorder writes nothing and pushes no row"
    );
}

/// A recorder over a sink that fails its first write degrades on the first step. Every later
/// step adds zero writes to the sink, so no record reaches the writer after the degrade.
#[test]
fn a_degraded_recorder_routes_no_further_record_to_its_writer() {
    let writes = Arc::new(AtomicUsize::new(0));
    let sink = FailingCountingSink {
        writes: Arc::clone(&writes),
    };
    let writer = SessionWriter::with_sink("count.jsonl", Box::new(sink));
    let mut recorder = Some(SessionRecorder::new(SessionLog::File(writer)));
    let mut state = TuiState::default();

    let input = prompt("first");
    record_turn(&mut recorder, &mut state, TurnRecord::Prompt(&input));
    let after_first = writes.load(Ordering::SeqCst);
    assert_eq!(
        after_first, 1,
        "the first record attempts exactly one write, which fails"
    );

    let second = prompt("second");
    record_turn(&mut recorder, &mut state, TurnRecord::Prompt(&second));
    record_turn(
        &mut recorder,
        &mut state,
        TurnRecord::Selection {
            provider: "anthropic",
            model: "opus-4.1",
        },
    );
    record_turn(&mut recorder, &mut state, TurnRecord::Cancel);
    assert_eq!(
        writes.load(Ordering::SeqCst),
        after_first,
        "no record reaches the writer after the degrade"
    );
}

/// A recorder over a failing sink, folded through `record_turn`, turns ephemeral and pushes
/// exactly one transcript error row. A seam that only warns through `tracing` fails this test.
#[test]
fn a_mid_run_write_failure_reaches_the_transcript() {
    let writes = Arc::new(AtomicUsize::new(0));
    let sink = FailingCountingSink {
        writes: Arc::clone(&writes),
    };
    let writer = SessionWriter::with_sink("fail.jsonl", Box::new(sink));
    let mut recorder = Some(SessionRecorder::new(SessionLog::File(writer)));
    let mut state = TuiState::default();

    let input = prompt("first");
    record_turn(&mut recorder, &mut state, TurnRecord::Prompt(&input));
    assert_eq!(
        error_rows(&state),
        1,
        "a fresh degrade pushes exactly one transcript error row"
    );

    // A second record must not push a second error row, because the log already degraded.
    let second = prompt("second");
    record_turn(&mut recorder, &mut state, TurnRecord::Prompt(&second));
    assert_eq!(
        error_rows(&state),
        1,
        "a later record on an already-degraded log pushes no further error row"
    );
}

/// A scripted turn with an open tool call, then a `Cancel`, writes one stop and a synthetic
/// result for the open call. The file holds no `ToolCall` without a `ToolResult`, and it holds
/// no close. A later `Prompt` still appends, so the session stayed open.
#[test]
fn a_cancel_records_a_stop_and_completes_open_tool_calls() {
    let (_dir, store) = temp_store();
    let (recorder, path) = file_recorder(&store, 0x0003);
    let mut recorder = Some(recorder);
    let mut state = TuiState::default();

    record_turn(
        &mut recorder,
        &mut state,
        TurnRecord::Event(&AgentEvent::TurnStart),
    );
    let tool_start = AgentEvent::ToolStart {
        id: "call-1".to_string(),
        name: "bash".to_string(),
        kind: ToolKind::Read,
    };
    record_turn(&mut recorder, &mut state, TurnRecord::Event(&tool_start));
    record_turn(&mut recorder, &mut state, TurnRecord::Cancel);

    let after_cancel = records(&path);
    let stops = after_cancel
        .iter()
        .filter(|r| matches!(r, Record::Stop { stop_reason } if *stop_reason == AgentStopReason::Canceled))
        .count();
    assert_eq!(stops, 1, "a cancel writes exactly one Canceled stop");
    assert!(
        !after_cancel.iter().any(|r| matches!(r, Record::Closed)),
        "a cancel keeps the session open, so it writes no close"
    );
    assert!(
        pairing_is_complete(&after_cancel),
        "a cancel completes every open tool pairing: {after_cancel:?}"
    );

    // The session stays open, so a later prompt still appends.
    let again = prompt("try again");
    record_turn(&mut recorder, &mut state, TurnRecord::Prompt(&again));
    let users = records(&path)
        .into_iter()
        .filter(|r| message_role(r) == Some(Role::User))
        .count();
    assert_eq!(
        users, 1,
        "the session stayed open and recorded the next prompt"
    );
}

/// True when every `ToolCall` id in the records has a matching `ToolResult`.
fn pairing_is_complete(records: &[Record]) -> bool {
    let mut calls = Vec::new();
    let mut results = Vec::new();
    for record in records {
        if let Record::Message { message } = record {
            for block in &message.content {
                match block {
                    ContentBlock::ToolCall { id, .. } => calls.push(id.clone()),
                    ContentBlock::ToolResult { tool_call_id, .. } => {
                        results.push(tool_call_id.clone())
                    }
                    _ => {}
                }
            }
        }
    }
    calls.iter().all(|id| results.contains(id))
}
