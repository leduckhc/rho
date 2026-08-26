//! The recorder folds a live run into records.
//!
//! See `SPEC-session-store-wiring` section 6d and `D-a-recorder-writes-the-assistant-turn`.
//!
//! **This is the defect that made the whole feature unusable.** `SessionRecorder::observe`
//! wrote a tool result, a usage record, and a stop record. It wrote no assistant message and
//! no tool call, because its match had no `TurnEnd` arm and it read no text or tool-call
//! stream event. Its own doc comment claimed otherwise.
//!
//! A resume then rebuilt a message list with a `ToolResult` that matched no `ToolCall`, and
//! every provider refuses that request. No test caught it, because every test that wrote a
//! `Message` record built the record by hand.
//!
//! Every test here isolates the filesystem with `tempfile`. None sleeps, none touches the
//! network.

use std::path::Path;

use rho_core::{
    AgentEvent, AgentStopReason, ContentBlock, Entry, NewSession, ProviderState, ReasoningOwner,
    Record, Role, SessionId, SessionLog, SessionReader, SessionRecorder, SessionStore, StopReason,
    StreamEvent, ToolKind, ToolOutput, branch_messages,
};

fn temp_store() -> (tempfile::TempDir, SessionStore) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = SessionStore::new(dir.path().join("sessions"));
    (dir, store)
}

fn id(suffix: u16) -> SessionId {
    SessionId::mint(1_756_000_000_000, suffix)
}

fn new_session<'a>(id: &'a SessionId, cwd: &'a Path) -> NewSession<'a> {
    NewSession {
        id,
        cwd,
        approval: "read-only",
        sandbox: "off",
        provider: "testkit",
        model: "test-model",
        forked_from: None,
    }
}

/// A recorder over a real session file, and the path it writes.
fn recorder(store: &SessionStore, suffix: u16) -> (SessionRecorder, std::path::PathBuf) {
    let writer = store
        .create(new_session(&id(suffix), Path::new("/work")))
        .expect("a created session");
    let path = writer.path().to_path_buf();
    (SessionRecorder::new(SessionLog::File(writer)), path)
}

/// Every record in the file, in file order.
fn entries(path: &Path) -> Vec<Entry> {
    SessionReader::read(path)
        .expect("the file reads back")
        .entries
}

/// The messages in the file, in file order.
fn messages(path: &Path) -> Vec<rho_core::Message> {
    entries(path)
        .into_iter()
        .filter_map(|entry| match entry.record {
            Record::Message { message } => Some(message),
            _ => None,
        })
        .collect()
}

/// Script one assistant turn that answers with text alone.
fn text_turn(recorder: &mut SessionRecorder, deltas: &[&str]) {
    recorder.observe(&AgentEvent::TurnStart);
    recorder.observe(&AgentEvent::Stream(StreamEvent::MessageStart {
        role: Role::Assistant,
    }));
    recorder.observe(&AgentEvent::Stream(StreamEvent::TextStart { index: 0 }));
    for delta in deltas {
        recorder.observe(&AgentEvent::Stream(StreamEvent::TextDelta {
            index: 0,
            delta: (*delta).to_string(),
        }));
    }
    recorder.observe(&AgentEvent::Stream(StreamEvent::TextEnd { index: 0 }));
    recorder.observe(&AgentEvent::TurnEnd {
        stop_reason: StopReason::EndTurn,
    });
}

/// Script one assistant turn that calls a tool, and the tool result that follows it.
fn tool_turn(
    recorder: &mut SessionRecorder,
    call_id: &str,
    name: &str,
    arguments: serde_json::Value,
    output: &str,
) {
    recorder.observe(&AgentEvent::TurnStart);
    recorder.observe(&AgentEvent::Stream(StreamEvent::MessageStart {
        role: Role::Assistant,
    }));
    recorder.observe(&AgentEvent::Stream(StreamEvent::ToolCallStart {
        index: 0,
        id: call_id.to_string(),
        name: name.to_string(),
    }));
    recorder.observe(&AgentEvent::Stream(StreamEvent::ToolCallEnd {
        index: 0,
        arguments,
        state: None,
    }));
    recorder.observe(&AgentEvent::TurnEnd {
        stop_reason: StopReason::ToolUse,
    });
    recorder.observe(&AgentEvent::ToolStart {
        id: call_id.to_string(),
        name: name.to_string(),
        kind: ToolKind::Read,
    });
    recorder.observe(&AgentEvent::ToolEnd {
        id: call_id.to_string(),
        output: ToolOutput {
            content: vec![ContentBlock::Text {
                text: output.to_string(),
            }],
            is_error: false,
        },
    });
}

// ---------------------------------------------------------------------------

#[test]
fn a_run_records_the_assistant_text_of_a_turn() {
    let (_guard, store) = temp_store();
    let (mut recorder, path) = recorder(&store, 0x0001);

    text_turn(&mut recorder, &["The bug is ", "on line 42."]);

    let assistant: Vec<&rho_core::Message> = {
        let all = messages(&path);
        let kept: Vec<rho_core::Message> = all
            .into_iter()
            .filter(|m| m.role == Role::Assistant)
            .collect();
        assert_eq!(
            kept.len(),
            1,
            "one turn writes exactly one assistant message"
        );
        Box::leak(Box::new(kept)).iter().collect()
    };
    let text = assistant[0]
        .content
        .iter()
        .find_map(|block| match block {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .expect("an assistant text block");

    assert_eq!(
        text, "The bug is on line 42.",
        "the deltas of one turn join into one text block"
    );
}

#[test]
fn an_empty_turn_writes_no_assistant_record() {
    let (_guard, store) = temp_store();
    let (mut recorder, path) = recorder(&store, 0x0002);

    recorder.observe(&AgentEvent::TurnStart);
    recorder.observe(&AgentEvent::TurnEnd {
        stop_reason: StopReason::EndTurn,
    });

    let assistant = messages(&path)
        .into_iter()
        .filter(|m| m.role == Role::Assistant)
        .count();
    assert_eq!(assistant, 0, "a turn with no content writes nothing");
}

#[test]
fn a_run_records_a_tool_call_before_its_result() {
    let (_guard, store) = temp_store();
    let (mut recorder, path) = recorder(&store, 0x0003);

    tool_turn(
        &mut recorder,
        "call-1",
        "read",
        serde_json::json!({ "path": "src/parse.rs" }),
        "1.2 KiB of source",
    );

    let records = entries(&path);
    let call_line = records
        .iter()
        .position(|entry| holds_tool_call(entry, "call-1"))
        .expect("the tool call must be on disk");
    let result_line = records
        .iter()
        .position(|entry| holds_tool_result(entry, "call-1"))
        .expect("the tool result must be on disk");

    assert!(
        call_line < result_line,
        "the file order is the call, then its result; got the call at {call_line} and the \
         result at {result_line}"
    );
    // The arguments the provider sent must survive, or a replay sends an empty object.
    let arguments = tool_call_arguments(&records, "call-1").expect("the arguments");
    assert_eq!(arguments["path"], "src/parse.rs");
}

#[test]
fn every_tool_call_on_disk_has_a_result_on_disk() {
    // The invariant, over a run with three tool calls. It is the pairing rule, checked on the
    // file the run wrote, and it is what makes a resume a valid provider request.
    let (_guard, store) = temp_store();
    let (mut recorder, path) = recorder(&store, 0x0004);

    for n in 0..3 {
        tool_turn(
            &mut recorder,
            &format!("call-{n}"),
            "read",
            serde_json::json!({ "path": format!("file-{n}.rs") }),
            "some output",
        );
    }
    text_turn(&mut recorder, &["All three files read."]);

    let records = entries(&path);
    let calls: Vec<String> = records.iter().flat_map(tool_call_ids).collect();
    let results: Vec<String> = records.iter().flat_map(tool_result_ids).collect();

    assert_eq!(calls.len(), 3, "the file holds every tool call");
    for call in &calls {
        assert!(
            results.contains(call),
            "every tool call on disk must have a result on disk; {call} has none"
        );
    }
}

#[test]
fn a_recorded_run_replays_as_a_valid_message_list() {
    // The resume path, end to end. Read the file back, rebuild the branch, and assert the
    // pairing is complete and the assistant text survives. This is the test that proves the
    // feature works, and it fails against a recorder with no TurnEnd arm.
    let (_guard, store) = temp_store();
    let (mut recorder, path) = recorder(&store, 0x0005);

    recorder.record_prompt(&[ContentBlock::Text {
        text: "fix the parser".to_string(),
    }]);
    tool_turn(
        &mut recorder,
        "call-1",
        "read",
        serde_json::json!({ "path": "src/parse.rs" }),
        "the source",
    );
    text_turn(&mut recorder, &["The bug is on line 42."]);
    recorder.observe(&AgentEvent::AgentEnd {
        stop_reason: AgentStopReason::EndTurn,
    });

    let read = SessionReader::read(&path).expect("the file reads back");
    let head = read.entries.last().expect("a record").id.clone();
    let rebuilt = branch_messages(&read.entries, &head, Some(&read.header_id))
        .expect("the whole chain rebuilds");

    // Every tool call in the rebuilt list has a result, and no result stands alone. A provider
    // refuses either half.
    let mut calls = Vec::new();
    let mut results = Vec::new();
    for message in &rebuilt {
        for block in &message.content {
            match block {
                ContentBlock::ToolCall { id, .. } => calls.push(id.clone()),
                ContentBlock::ToolResult { tool_call_id, .. } => results.push(tool_call_id.clone()),
                _ => {}
            }
        }
    }
    assert_eq!(
        calls,
        vec!["call-1".to_string()],
        "the call survives a resume"
    );
    assert_eq!(
        results, calls,
        "every call is matched, and no result stands alone"
    );

    let roles: Vec<Role> = rebuilt.iter().map(|m| m.role).collect();
    assert!(
        roles.contains(&Role::Assistant),
        "a resume must replay the assistant turns, got {roles:?}"
    );
    let text: String = rebuilt
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<String>>()
        .join(" ");
    assert!(
        text.contains("The bug is on line 42."),
        "the answer survives a resume, got {text:?}"
    );
}

#[test]
fn a_reasoning_payload_survives_the_recorder_verbatim() {
    // A rewritten payload cannot replay, so rule 9 keeps it byte for byte.
    let (_guard, store) = temp_store();
    let (mut recorder, path) = recorder(&store, 0x0006);
    let state = ProviderState {
        owner: ReasoningOwner {
            provider: "testkit".to_string(),
            model: "test-model".to_string(),
        },
        value: serde_json::json!({ "signature": "abc123", "nested": { "n": 7 } }),
    };

    recorder.observe(&AgentEvent::TurnStart);
    recorder.observe(&AgentEvent::Stream(StreamEvent::MessageStart {
        role: Role::Assistant,
    }));
    recorder.observe(&AgentEvent::Stream(StreamEvent::ThinkingStart { index: 0 }));
    recorder.observe(&AgentEvent::Stream(StreamEvent::ThinkingDelta {
        index: 0,
        delta: "I will read the file first.".to_string(),
    }));
    recorder.observe(&AgentEvent::Stream(StreamEvent::ThinkingEnd {
        index: 0,
        state: Some(state.clone()),
    }));
    recorder.observe(&AgentEvent::TurnEnd {
        stop_reason: StopReason::EndTurn,
    });

    let block = messages(&path)
        .into_iter()
        .flat_map(|m| m.content.into_iter())
        .find(|block| matches!(block, ContentBlock::ReasoningReplay { .. }))
        .expect("a reasoning replay block on disk");
    let ContentBlock::ReasoningReplay { text, state: back } = block else {
        unreachable!();
    };

    assert_eq!(text, "I will read the file first.");
    assert_eq!(back, Some(state), "the payload survives verbatim");
}

#[test]
fn a_cancel_records_the_real_tool_arguments() {
    // The cancel path invented an empty object, because it never held the arguments. A resume
    // then replayed a call with no arguments, which is a different call.
    let (_guard, store) = temp_store();
    let (mut recorder, path) = recorder(&store, 0x0007);

    recorder.observe(&AgentEvent::TurnStart);
    recorder.observe(&AgentEvent::Stream(StreamEvent::MessageStart {
        role: Role::Assistant,
    }));
    recorder.observe(&AgentEvent::Stream(StreamEvent::ToolCallStart {
        index: 0,
        id: "call-1".to_string(),
        name: "bash".to_string(),
    }));
    recorder.observe(&AgentEvent::Stream(StreamEvent::ToolCallEnd {
        index: 0,
        arguments: serde_json::json!({ "command": "cargo test" }),
        state: None,
    }));
    // The cancel lands before the turn ended, so the recorder must flush what it holds.
    recorder.record_cancel();

    let records = entries(&path);
    let arguments = tool_call_arguments(&records, "call-1")
        .expect("a cancel must write the call it already holds");
    assert_eq!(
        arguments["command"], "cargo test",
        "the cancel writes the arguments the provider sent, not an empty object"
    );
    let results: Vec<String> = records.iter().flat_map(tool_result_ids).collect();
    assert!(
        results.contains(&"call-1".to_string()),
        "a cancel completes every open pairing on disk"
    );
}

#[test]
fn a_recorded_secret_named_argument_is_masked() {
    // Redaction still runs on the folded turn. It matches a **key name** only, so this test
    // states its own limit: a secret in a tool result, or on a bash command line, reaches the
    // file verbatim. See `SPEC-session-store-wiring` section 9.
    let (_guard, store) = temp_store();
    let (mut recorder, path) = recorder(&store, 0x0008);

    tool_turn(
        &mut recorder,
        "call-1",
        "http",
        serde_json::json!({ "url": "https://example.com", "api_key": "sk-live-should-not-land" }),
        "ok",
    );

    let text = std::fs::read_to_string(&path).expect("the file");
    assert!(
        !text.contains("sk-live-should-not-land"),
        "a credential-shaped argument key must be masked in the file the run wrote"
    );
    assert!(
        text.contains("https://example.com"),
        "an ordinary argument survives"
    );
}

#[test]
fn an_empty_name_is_refused_by_the_recorder() {
    let (_guard, store) = temp_store();
    let (mut recorder, path) = recorder(&store, 0x0009);

    for blank in ["", "   ", "\n\t"] {
        recorder
            .record_name(blank)
            .map(|_| ())
            .expect_err("an empty title must be refused, so a row never shows a blank name");
    }
    recorder
        .record_name("a real title")
        .expect("a real title is written");

    let names: Vec<String> = entries(&path)
        .into_iter()
        .filter_map(|entry| match entry.record {
            Record::Name { title } => Some(title),
            _ => None,
        })
        .collect();
    assert_eq!(names, vec!["a real title".to_string()]);
}

#[test]
fn a_title_costs_no_model_call() {
    // A title is one leaf record and one automatic first line. It calls no provider, so it
    // costs no money and no time. The recorder holds no provider at all, and that is the
    // structural proof: there is nothing here that could make a request.
    let (_guard, store) = temp_store();
    let (mut recorder, path) = recorder(&store, 0x000a);

    recorder.record_prompt(&[ContentBlock::Text {
        text: "fix the parser".to_string(),
    }]);
    recorder
        .record_name("an explicit title")
        .expect("the title is written");

    let rows = store.rows().expect("the rows build");
    let rho_core::SessionRow::Session(summary) = &rows[0] else {
        panic!("a readable row");
    };
    assert_eq!(summary.title, "an explicit title");
    assert!(path.exists());
}

// ---------------------------------------------------------------------------
// The helpers that read a record's tool blocks.
// ---------------------------------------------------------------------------

fn tool_call_ids(entry: &Entry) -> Vec<String> {
    let Record::Message { message } = &entry.record else {
        return Vec::new();
    };
    message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolCall { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect()
}

fn tool_result_ids(entry: &Entry) -> Vec<String> {
    let Record::Message { message } = &entry.record else {
        return Vec::new();
    };
    message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolResult { tool_call_id, .. } => Some(tool_call_id.clone()),
            _ => None,
        })
        .collect()
}

fn holds_tool_call(entry: &Entry, id: &str) -> bool {
    tool_call_ids(entry).iter().any(|found| found == id)
}

fn holds_tool_result(entry: &Entry, id: &str) -> bool {
    tool_result_ids(entry).iter().any(|found| found == id)
}

fn tool_call_arguments(records: &[Entry], id: &str) -> Option<serde_json::Value> {
    records.iter().find_map(|entry| {
        let Record::Message { message } = &entry.record else {
            return None;
        };
        message.content.iter().find_map(|block| match block {
            ContentBlock::ToolCall {
                id: found,
                arguments,
                ..
            } if found == id => Some(arguments.clone()),
            _ => None,
        })
    })
}
