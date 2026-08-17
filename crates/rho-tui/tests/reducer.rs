//! Reducer tests. The reducer is pure, so these tests need no terminal.
//! See `SPEC-05` section 7.

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
