//! Reducer tests. The reducer is pure, so these tests need no terminal.
//! See `SPEC-tui` section 7.

use rho_core::{AgentEvent, AgentStopReason, StreamEvent, ToolKind, ToolOutput};
use rho_tui::{ActivityState, Row, ToolRowStatus, TuiState};

fn text_start() -> AgentEvent {
    AgentEvent::Stream(StreamEvent::TextStart { index: 0 })
}

fn text_delta(delta: &str) -> AgentEvent {
    AgentEvent::Stream(StreamEvent::TextDelta {
        index: 0,
        delta: delta.to_string(),
    })
}

#[test]
fn reducer_text_delta_appends_to_assistant_row() {
    let mut state = TuiState::default();
    state.apply(&text_start());
    state.apply(&text_delta("Hello, "));
    state.apply(&text_delta("world"));

    assert_eq!(state.rows.len(), 1);
    assert_eq!(
        state.rows[0],
        Row::Assistant {
            text: "Hello, world".to_string()
        }
    );
}

#[test]
fn reducer_thinking_delta_builds_thinking_row() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::Stream(StreamEvent::ThinkingStart { index: 0 }));
    state.apply(&AgentEvent::Stream(StreamEvent::ThinkingDelta {
        index: 0,
        delta: "step one".to_string(),
    }));

    assert_eq!(
        state.rows[0],
        Row::Thinking {
            text: "step one".to_string()
        }
    );
}

#[test]
fn reducer_tool_call_end_pushes_pending_tool_row() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::Stream(StreamEvent::ToolCallStart {
        index: 0,
        id: "call-1".to_string(),
        name: "read".to_string(),
    }));
    state.apply(&AgentEvent::Stream(StreamEvent::ToolCallEnd {
        index: 0,
        arguments: serde_json::json!({"path": "a.txt"}),
    }));

    assert_eq!(state.rows.len(), 1);
    match &state.rows[0] {
        Row::Tool {
            id, name, status, ..
        } => {
            assert_eq!(id, "call-1");
            assert_eq!(name, "read");
            assert_eq!(*status, ToolRowStatus::Pending);
        }
        other => panic!("expected a tool row, got {other:?}"),
    }
}

#[test]
fn reducer_tool_start_sets_running() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::Stream(StreamEvent::ToolCallStart {
        index: 0,
        id: "call-1".to_string(),
        name: "read".to_string(),
    }));
    state.apply(&AgentEvent::Stream(StreamEvent::ToolCallEnd {
        index: 0,
        arguments: serde_json::json!({}),
    }));
    state.apply(&AgentEvent::ToolStart {
        id: "call-1".to_string(),
        name: "read".to_string(),
        kind: ToolKind::Read,
    });

    match &state.rows[0] {
        Row::Tool { status, kind, .. } => {
            assert_eq!(*status, ToolRowStatus::Running);
            assert_eq!(*kind, ToolKind::Read);
        }
        other => panic!("expected a tool row, got {other:?}"),
    }
}

#[test]
fn reducer_tool_end_error_sets_failed() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::ToolStart {
        id: "call-1".to_string(),
        name: "bash".to_string(),
        kind: ToolKind::Execute,
    });
    state.apply(&AgentEvent::ToolEnd {
        id: "call-1".to_string(),
        output: ToolOutput {
            content: vec![],
            is_error: true,
        },
    });

    match &state.rows[0] {
        Row::Tool { status, .. } => assert_eq!(*status, ToolRowStatus::Failed),
        other => panic!("expected a tool row, got {other:?}"),
    }
}

#[test]
fn reducer_tool_update_sets_preview() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::ToolStart {
        id: "call-1".to_string(),
        name: "bash".to_string(),
        kind: ToolKind::Execute,
    });
    state.apply(&AgentEvent::ToolUpdate {
        id: "call-1".to_string(),
        output: "line two".to_string(),
    });

    match &state.rows[0] {
        Row::Tool { preview, .. } => assert_eq!(preview, "line two"),
        other => panic!("expected a tool row, got {other:?}"),
    }
}

#[test]
fn reducer_agent_end_sets_idle_and_stop() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart);
    assert_eq!(state.activity, ActivityState::Running);
    state.apply(&AgentEvent::AgentEnd {
        stop_reason: AgentStopReason::EndTurn,
    });

    assert_eq!(state.activity, ActivityState::Idle);
    assert_eq!(state.last_stop, Some(AgentStopReason::EndTurn));
    assert!(!state.status.is_empty());
}

#[test]
fn reducer_is_pure_same_events_same_state() {
    let events = vec![
        AgentEvent::TurnStart,
        text_start(),
        text_delta("a"),
        text_delta("b"),
        AgentEvent::AgentEnd {
            stop_reason: AgentStopReason::EndTurn,
        },
    ];

    let mut first = TuiState::default();
    let mut second = TuiState::default();
    for event in &events {
        first.apply(event);
    }
    for event in &events {
        second.apply(event);
    }

    assert_eq!(first, second);
}

// --- Background task rows, from SPEC-background-tasks ------------------------------------

fn task_id(text: &str) -> rho_core::TaskId {
    rho_core::TaskId(text.to_string())
}

#[test]
fn task_start_adds_a_task_row() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TaskStart {
        id: task_id("t1"),
        command: "cargo test".to_string(),
        reason: rho_core::BackgroundReason::KnownLongRunning,
    });
    assert!(
        state.rows.iter().any(|row| matches!(
            row,
            Row::Task { command, finished, .. } if command == "cargo test" && !finished
        )),
        "a running task row must appear: {:?}",
        state.rows
    );
}

#[test]
fn task_progress_updates_the_row_in_place() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TaskStart {
        id: task_id("t1"),
        command: "cargo test".to_string(),
        reason: rho_core::BackgroundReason::KnownLongRunning,
    });
    state.apply(&AgentEvent::TaskProgressed {
        id: task_id("t1"),
        progress: rho_core::TaskProgress {
            percent: Some(42),
            message: Some("compiling".to_string()),
            done: Some(6),
            total: Some(10),
        },
    });
    let count = state
        .rows
        .iter()
        .filter(|row| matches!(row, Row::Task { .. }))
        .count();
    assert_eq!(count, 1, "progress must update the row, not add one");
    let text = format!("{:?}", state.rows);
    assert!(text.contains("42%"), "percent must show: {text}");
    assert!(text.contains("6/10"), "counts must show: {text}");
    assert!(text.contains("compiling"), "message must show: {text}");
}

#[test]
fn task_end_marks_success_and_failure_differently() {
    for (state_value, want_failed) in [
        (rho_core::TaskState::Exited { code: 0 }, false),
        (rho_core::TaskState::Exited { code: 1 }, true),
        (rho_core::TaskState::TimedOut, true),
        (rho_core::TaskState::Canceled, true),
    ] {
        let mut state = TuiState::default();
        state.apply(&AgentEvent::TaskStart {
            id: task_id("t1"),
            command: "x".to_string(),
            reason: rho_core::BackgroundReason::ModelRequested,
        });
        state.apply(&AgentEvent::TaskEnd {
            id: task_id("t1"),
            state: state_value.clone(),
            output_tail: String::new(),
        });
        let row = state
            .rows
            .iter()
            .find_map(|row| match row {
                Row::Task {
                    finished, failed, ..
                } => Some((*finished, *failed)),
                _ => None,
            })
            .expect("a task row");
        assert!(row.0, "{state_value:?} must mark the row finished");
        assert_eq!(row.1, want_failed, "{state_value:?} failed flag");
    }
}

#[test]
fn a_task_row_survives_the_turn_ending() {
    // The point of a background task. It outlives the turn that started it, so its row
    // must not be cleared when the turn ends or when the run settles.
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TaskStart {
        id: task_id("t1"),
        command: "cargo test".to_string(),
        reason: rho_core::BackgroundReason::KnownLongRunning,
    });
    state.apply(&AgentEvent::TurnEnd {
        stop_reason: rho_core::StopReason::EndTurn,
    });
    state.apply(&AgentEvent::AgentEnd {
        stop_reason: rho_core::AgentStopReason::EndTurn,
    });
    assert!(
        state.rows.iter().any(|row| matches!(
            row,
            Row::Task {
                finished: false,
                ..
            }
        )),
        "the running task row must survive the run ending"
    );
}

#[test]
fn a_task_progress_message_cannot_corrupt_the_display() {
    // A child prints whatever it likes, so a progress message is untrusted input.
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TaskStart {
        id: task_id("t1"),
        command: "evil\u{1b}[2Jcommand".to_string(),
        reason: rho_core::BackgroundReason::ModelRequested,
    });
    state.apply(&AgentEvent::TaskProgressed {
        id: task_id("t1"),
        progress: rho_core::TaskProgress {
            percent: None,
            message: Some("step\u{1b}[31m one\r\n".to_string()),
            done: None,
            total: None,
        },
    });
    let text = format!("{:?}", state.rows);
    assert!(
        !text.contains('\u{1b}'),
        "no escape character may survive: {text}"
    );
}

#[test]
fn a_task_event_for_an_unknown_id_is_ignored() {
    // A late event must not panic and must not invent a row.
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TaskProgressed {
        id: task_id("ghost"),
        progress: rho_core::TaskProgress::default(),
    });
    state.apply(&AgentEvent::TaskEnd {
        id: task_id("ghost"),
        state: rho_core::TaskState::Exited { code: 0 },
        output_tail: String::new(),
    });
    assert!(state.rows.is_empty(), "no row must be invented");
}

// --- Subagent rows, from SPEC-subagents section 9 ---------------------------------

fn agent_id(n: u64) -> rho_core::AgentId {
    rho_core::AgentId(n)
}

fn report(outcome: rho_core::AgentOutcome) -> rho_core::AgentReport {
    rho_core::AgentReport {
        agent: "scout".to_string(),
        outcome,
        summary: "found it".to_string(),
        usage: rho_core::Usage {
            input_tokens: 1400,
            output_tokens: 42,
            ..Default::default()
        },
        turns: 3,
        transcript: None,
    }
}

#[test]
fn a_spawned_agent_adds_a_row() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::AgentSpawned {
        id: agent_id(1),
        agent: "scout".to_string(),
        depth: 1,
    });
    assert!(
        state.rows.iter().any(|row| matches!(
            row,
            Row::Agent { name, depth, finished, .. } if name == "scout" && *depth == 1 && !finished
        )),
        "{:?}",
        state.rows
    );
}

#[test]
fn agent_progress_updates_the_row_in_place() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::AgentSpawned {
        id: agent_id(1),
        agent: "scout".to_string(),
        depth: 0,
    });
    state.apply(&AgentEvent::AgentProgressed {
        id: agent_id(1),
        turns: 2,
        usage: rho_core::Usage {
            input_tokens: 2500,
            output_tokens: 100,
            ..Default::default()
        },
    });
    let count = state
        .rows
        .iter()
        .filter(|row| matches!(row, Row::Agent { .. }))
        .count();
    assert_eq!(count, 1, "progress must update the row, not add one");
    let text = format!("{:?}", state.rows);
    assert!(text.contains("2.5k"), "a large count is compact: {text}");
}

#[test]
fn a_finished_agent_marks_success_and_failure_differently() {
    for (outcome, want_failed) in [
        (rho_core::AgentOutcome::Done, false),
        (rho_core::AgentOutcome::OutOfTurns, true),
        (rho_core::AgentOutcome::Canceled, true),
        (
            rho_core::AgentOutcome::Failed {
                reason: "the child died".to_string(),
            },
            true,
        ),
    ] {
        let mut state = TuiState::default();
        state.apply(&AgentEvent::AgentSpawned {
            id: agent_id(1),
            agent: "scout".to_string(),
            depth: 0,
        });
        state.apply(&AgentEvent::AgentFinished {
            id: agent_id(1),
            report: report(outcome.clone()),
        });
        let row = state
            .rows
            .iter()
            .find_map(|row| match row {
                Row::Agent {
                    finished, failed, ..
                } => Some((*finished, *failed)),
                _ => None,
            })
            .expect("an agent row");
        assert!(row.0, "{outcome:?} must mark the row finished");
        assert_eq!(row.1, want_failed, "{outcome:?} failed flag");
    }
}

#[test]
fn an_agent_summary_never_reaches_the_transcript_rows() {
    // The core promise of SPEC-subagents section 6, checked at the frontend too. The row shows
    // the cost and the outcome. The child's answer belongs in the parent's tool result,
    // not as an assistant row that would read as the parent's own words.
    let mut state = TuiState::default();
    state.apply(&AgentEvent::AgentSpawned {
        id: agent_id(1),
        agent: "scout".to_string(),
        depth: 0,
    });
    state.apply(&AgentEvent::AgentFinished {
        id: agent_id(1),
        report: report(rho_core::AgentOutcome::Done),
    });
    assert!(
        !state
            .rows
            .iter()
            .any(|row| matches!(row, Row::Assistant { text } if text.contains("found it"))),
        "the child summary must not become an assistant row: {:?}",
        state.rows
    );
}

#[test]
fn a_failure_reason_with_an_escape_sequence_is_sanitised() {
    // A reason can carry any bytes, because a child's failure may quote a file.
    let mut state = TuiState::default();
    state.apply(&AgentEvent::AgentSpawned {
        id: agent_id(1),
        agent: "sc\u{1b}[2Jout".to_string(),
        depth: 0,
    });
    state.apply(&AgentEvent::AgentFinished {
        id: agent_id(1),
        report: report(rho_core::AgentOutcome::Failed {
            reason: "boom\u{1b}[31m".to_string(),
        }),
    });
    let text = format!("{:?}", state.rows);
    assert!(!text.contains('\u{1b}'), "no escape may survive: {text}");
}

#[test]
fn an_agent_event_for_an_unknown_id_is_ignored() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::AgentProgressed {
        id: agent_id(99),
        turns: 1,
        usage: rho_core::Usage::default(),
    });
    assert!(state.rows.is_empty(), "no row must be invented");
}
