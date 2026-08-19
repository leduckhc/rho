//! Freeze tests: which rows may leave the band, and what a late event does.
//!
//! A frozen row lives in the terminal's scrollback, and rho can never repaint it. So these
//! tests are the guard for the one unrepairable defect in the design. See
//! `SPEC-tui-inline-and-composer` sections 3.1, 3.4, and 3.5.

use rho_core::{
    AgentEvent, AgentId, AgentOutcome, AgentReport, AgentStopReason, StreamEvent, ToolKind,
    ToolOutput, Usage,
};
use rho_tui::{Row, ToolRowStatus, TuiState, row_is_final};

/// Start and finish one tool call, so a test has a final tool row.
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

fn text_start() -> AgentEvent {
    AgentEvent::Stream(StreamEvent::TextStart { index: 0 })
}

fn thinking_start() -> AgentEvent {
    AgentEvent::Stream(StreamEvent::ThinkingStart { index: 0 })
}

fn running() -> TuiState {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart, 1_000);
    state
}

#[test]
fn a_user_row_is_final_at_once() {
    let mut state = running();
    state.rows.push(Row::User {
        text: "hello".to_string(),
    });
    assert!(row_is_final(&state, 0), "a user row can never change again");
}

#[test]
fn an_error_row_is_final_at_once() {
    let mut state = running();
    state.push_error("run error: boom");
    assert!(row_is_final(&state, 0), "nothing updates an error row");
}

#[test]
fn a_running_tool_row_is_never_final() {
    let mut state = running();
    state.apply(&tool_start("call-1"), 1_100);
    assert_eq!(
        state.rows.len(),
        1,
        "the tool start pushes the row this test measures"
    );
    assert!(
        !row_is_final(&state, 0),
        "a running tool row still changes, so it must not freeze"
    );
}

#[test]
fn a_finished_tool_row_is_final() {
    for failed in [false, true] {
        let mut state = running();
        state.apply(&tool_start("call-1"), 1_100);
        state.apply(&tool_end("call-1", failed), 1_600);
        assert!(
            row_is_final(&state, 0),
            "a tool row with a settled status is final, failed = {failed}"
        );
    }
}

#[test]
fn the_newest_assistant_row_is_not_final_mid_turn() {
    let mut state = running();
    state.apply(&text_start(), 1_100);
    assert!(
        !row_is_final(&state, 0),
        "the newest assistant row still receives deltas"
    );
}

#[test]
fn an_older_assistant_row_is_final() {
    let mut state = running();
    state.apply(&text_start(), 1_100);
    state.apply(&text_start(), 1_200);
    assert!(
        row_is_final(&state, 0),
        "a delta reaches only the newest assistant row"
    );
    assert!(!row_is_final(&state, 1), "the newest row still grows");
}

#[test]
fn an_older_thinking_row_is_final() {
    let mut state = running();
    state.apply(&thinking_start(), 1_100);
    state.apply(&thinking_start(), 1_200);
    assert!(row_is_final(&state, 0));
    assert!(!row_is_final(&state, 1));
}

#[test]
fn a_thinking_row_stays_live_when_an_assistant_row_follows() {
    // Interleaved text and thinking is a real provider shape. A newer assistant row says
    // nothing about a thinking row, because the two delta paths are separate.
    let mut state = running();
    state.apply(&thinking_start(), 1_100);
    state.apply(&text_start(), 1_200);
    assert!(
        !row_is_final(&state, 0),
        "a thinking delta can still reach the newest thinking row"
    );
}

#[test]
fn every_row_is_final_when_the_turn_ends() {
    let mut state = running();
    state.apply(&text_start(), 1_100);
    state.apply(&thinking_start(), 1_200);
    state.apply(&tool_start("call-1"), 1_300);
    state.apply(&tool_end("call-1", false), 1_400);
    state.apply(
        &AgentEvent::AgentEnd {
            stop_reason: AgentStopReason::EndTurn,
        },
        2_000,
    );
    for index in 0..state.rows.len() {
        assert!(
            row_is_final(&state, index),
            "row {index} must be final once the turn ended"
        );
    }
}

#[test]
fn an_unfinished_agent_row_is_never_final() {
    let mut state = running();
    state.apply(
        &AgentEvent::AgentSpawned {
            id: AgentId(1),
            agent: "scout".to_string(),
            depth: 1,
        },
        1_100,
    );
    assert!(!row_is_final(&state, 0), "a running child still reports");

    state.apply(
        &AgentEvent::AgentFinished {
            id: AgentId(1),
            report: AgentReport {
                agent: "scout".to_string(),
                outcome: AgentOutcome::Done,
                summary: "did the thing".to_string(),
                usage: Usage::default(),
                turns: 2,
                transcript: None,
            },
        },
        1_500,
    );
    assert!(row_is_final(&state, 0), "a finished child is final");
}

#[test]
fn an_unfinished_task_row_is_never_final() {
    let mut state = running();
    state.apply(
        &AgentEvent::TaskStart {
            id: rho_core::TaskId("t1".to_string()),
            command: "sleep 1".to_string(),
            reason: rho_core::BackgroundReason::KnownLongRunning,
        },
        1_100,
    );
    assert!(!row_is_final(&state, 0), "a running task still reports");
}

// ---- Section 3.4: a late event must never reach a frozen row. -------------------

#[test]
fn a_late_tool_event_for_a_frozen_row_changes_nothing() {
    let mut state = running();
    state.apply(&tool_start("call-1"), 1_100);
    state.apply(&tool_end("call-1", false), 1_600);
    state.mark_frozen(1);

    let before = state.rows.clone();
    state.apply(&tool_end("call-1", true), 1_700);
    state.apply(
        &AgentEvent::ToolUpdate {
            id: "call-1".to_string(),
            output: "late chatter".to_string(),
        },
        1_800,
    );

    assert_eq!(
        state.rows, before,
        "a frozen row is in the scrollback, so the reducer must not write it"
    );
    assert_eq!(state.late_events, 2, "each dropped event is counted");
}

#[test]
fn a_late_tool_start_pushes_no_duplicate_row() {
    let mut state = running();
    state.apply(&tool_start("call-1"), 1_100);
    state.apply(&tool_end("call-1", false), 1_600);
    state.mark_frozen(1);

    state.apply(&tool_start("call-1"), 1_700);

    assert_eq!(
        state.rows.len(),
        1,
        "a late start for a frozen id must not push a second row"
    );
    assert_eq!(state.late_events, 1);
}

#[test]
fn a_late_agent_progress_after_finish_changes_nothing() {
    let mut state = running();
    state.apply(
        &AgentEvent::AgentSpawned {
            id: AgentId(1),
            agent: "scout".to_string(),
            depth: 1,
        },
        1_100,
    );
    state.apply(
        &AgentEvent::AgentFinished {
            id: AgentId(1),
            report: AgentReport {
                agent: "scout".to_string(),
                outcome: AgentOutcome::Done,
                summary: "did the thing".to_string(),
                usage: Usage::default(),
                turns: 2,
                transcript: None,
            },
        },
        1_500,
    );
    state.mark_frozen(1);

    let before = state.rows.clone();
    state.apply(
        &AgentEvent::AgentProgressed {
            id: AgentId(1),
            turns: 9,
            usage: Usage::default(),
        },
        1_600,
    );

    assert_eq!(state.rows, before);
    assert_eq!(state.late_events, 1);
}

#[test]
fn a_live_row_still_takes_its_events() {
    // The drop rule must not break the normal path.
    let mut state = running();
    state.apply(&tool_start("call-1"), 1_100);
    state.apply(
        &AgentEvent::ToolUpdate {
            id: "call-1".to_string(),
            output: "progress".to_string(),
        },
        1_200,
    );
    let Row::Tool { preview, .. } = &state.rows[0] else {
        panic!("expected a tool row");
    };
    assert_eq!(preview, "progress");
    assert_eq!(state.late_events, 0);
}

// ---- Section 3.5: the reducer owns the row metadata. ---------------------------

#[test]
fn the_reducer_writes_a_tool_duration() {
    let mut state = running();
    state.apply(&tool_start("call-1"), 1_100);
    state.apply(&tool_end("call-1", false), 1_600);

    assert_eq!(
        state.row_durations.first().copied().flatten(),
        Some(500),
        "the row must carry its span before it can freeze"
    );
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

// ---- The band accounting. ------------------------------------------------------

#[test]
fn a_frozen_row_leaves_the_band() {
    let mut state = running();
    state.rows.push(Row::User {
        text: "first".to_string(),
    });
    state.rows.push(Row::User {
        text: "second".to_string(),
    });

    state.mark_frozen(1);

    assert_eq!(state.live_rows().len(), 1);
    assert_eq!(
        state.live_rows()[0],
        Row::User {
            text: "second".to_string()
        }
    );
}

#[test]
fn mark_frozen_saturates() {
    let mut state = running();
    state.rows.push(Row::User {
        text: "only".to_string(),
    });

    state.mark_frozen(9);

    assert_eq!(state.frozen_rows, 1, "a count past the end saturates");
    assert!(state.live_rows().is_empty());
}

#[test]
fn a_status_change_on_a_live_row_after_a_freeze_still_lands() {
    // Two tool calls, and only the first freezes. The second must still finish.
    let mut state = running();
    state.apply(&tool_start("call-1"), 1_100);
    state.apply(&tool_end("call-1", false), 1_200);
    state.apply(&tool_start("call-2"), 1_300);
    state.mark_frozen(1);
    state.apply(&tool_end("call-2", true), 1_900);

    let Row::Tool { status, .. } = &state.rows[1] else {
        panic!("expected the second tool row");
    };
    assert_eq!(*status, ToolRowStatus::Failed);
    assert_eq!(state.late_events, 0);
}

// ---- A row that outlives its turn is not final when the turn ends. --------------
//
// `Row::Task` and `Row::Agent` both say so in their own documentation: a task and a child
// outlive the turn that started them. So an idle turn must not freeze them. A frozen task
// row would read `running` for ever, and the events that would correct it get dropped.

#[test]
fn a_running_task_is_not_final_when_the_turn_ends() {
    let mut state = running();
    state.apply(
        &AgentEvent::TaskStart {
            id: rho_core::TaskId("t1".to_string()),
            command: "cargo test".to_string(),
            reason: rho_core::BackgroundReason::KnownLongRunning,
        },
        1_100,
    );
    state.apply(
        &AgentEvent::AgentEnd {
            stop_reason: AgentStopReason::EndTurn,
        },
        2_000,
    );

    assert!(
        !row_is_final(&state, 0),
        "a task outlives the turn, so the turn ending proves nothing"
    );
}

#[test]
fn a_running_child_is_not_final_when_the_turn_ends() {
    let mut state = running();
    state.apply(
        &AgentEvent::AgentSpawned {
            id: AgentId(1),
            agent: "scout".to_string(),
            depth: 1,
        },
        1_100,
    );
    state.apply(
        &AgentEvent::AgentEnd {
            stop_reason: AgentStopReason::EndTurn,
        },
        2_000,
    );

    assert!(!row_is_final(&state, 0), "a child outlives the turn");
}

#[test]
fn a_running_tool_is_final_when_the_turn_ends() {
    // A tool call cannot outlive its turn, so the turn ending does settle it.
    let mut state = running();
    state.apply(&tool_start("call-1"), 1_100);
    state.apply(
        &AgentEvent::AgentEnd {
            stop_reason: AgentStopReason::EndTurn,
        },
        2_000,
    );

    assert!(row_is_final(&state, 0));
}
