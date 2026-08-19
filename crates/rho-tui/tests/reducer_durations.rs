//! Reducer metadata tests: the durations and the turn clock. These are pure reducer
//! behaviour, independent of any screen layout. They moved here when the freeze machinery
//! went away with the inline band. See `SPEC-tui-alternate-screen` section 6b.

use rho_core::{AgentEvent, AgentStopReason, StreamEvent, ToolKind, ToolOutput};
use rho_tui::TuiState;

fn running() -> TuiState {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart, 1_000);
    state
}

fn tool_start(id: &str) -> AgentEvent {
    AgentEvent::ToolStart {
        id: id.to_string(),
        name: "bash".to_string(),
        kind: ToolKind::Execute,
    }
}

fn tool_end(id: &str, failed: bool) -> AgentEvent {
    AgentEvent::ToolEnd {
        id: id.to_string(),
        output: ToolOutput {
            content: vec![rho_core::ContentBlock::Text {
                text: "done".to_string(),
            }],
            is_error: failed,
        },
    }
}

fn thinking_start() -> AgentEvent {
    AgentEvent::Stream(StreamEvent::ThinkingStart { index: 0 })
}

#[test]
fn the_reducer_writes_a_tool_duration() {
    let mut state = running();
    state.apply(&tool_start("call-1"), 1_100);
    state.apply(&tool_end("call-1", false), 1_600);
    assert_eq!(state.row_durations.first().copied().flatten(), Some(500));
}

#[test]
fn the_reducer_writes_a_thinking_duration() {
    let mut state = running();
    state.apply(&thinking_start(), 1_000);
    state.apply(
        &AgentEvent::Stream(StreamEvent::ThinkingEnd {
            index: 0,
            signature: None,
        }),
        3_500,
    );
    assert_eq!(state.row_durations.first().copied().flatten(), Some(2_500));
}

#[test]
fn the_reducer_writes_the_turn_clock() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart, 1_000);
    state.apply(
        &AgentEvent::AgentEnd {
            stop_reason: AgentStopReason::EndTurn,
        },
        4_200,
    );
    assert_eq!(
        state.turn_millis,
        Some(3_200),
        "the footer clock needs the finished turn"
    );
}

#[test]
fn a_backward_clock_reports_no_duration() {
    // A clock can step back, so `end < start` is reachable. The ladder renders an empty
    // slot for that, and it must never render a negative span.
    let mut state = running();
    state.apply(&tool_start("call-1"), 5_000);
    state.apply(&tool_end("call-1", false), 4_000);
    assert_eq!(state.row_durations.first().copied().flatten(), Some(-1_000));
    assert_eq!(rho_tui::format_duration(Some(-1_000)), None);
}
