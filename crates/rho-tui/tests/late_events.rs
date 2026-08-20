//! A late event reaches an old row. See `SPEC-tui-alternate-screen` section 6b.
//!
//! The inline band pushed a finished row into the terminal's scrollback with
//! `Terminal::insert_before`, and rho could never repaint that row again. So the reducer
//! dropped an event that named an already-frozen row. That drop was correct then.
//!
//! In the alternate screen rho owns every row and can repaint any of them, so the drop
//! became a defect: it discarded output that rho was able to show. This file pins the
//! repair, and it is the reason section 6b calls the change a removal and not a cleanup.

use rho_core::{AgentEvent, AgentId, AgentOutcome, AgentReport, Usage};
use rho_tui::TuiState;

/// Push enough rows that the first ones are far behind the newest.
fn state_with_history() -> TuiState {
    let mut state = TuiState::default();
    for turn in 0..60 {
        state.apply(
            &AgentEvent::AgentSpawned {
                id: AgentId(turn),
                agent: format!("reviewer {turn}"),
                depth: 1,
            },
            turn as i64 * 1000,
        );
    }
    state
}

/// An event naming the oldest agent row still updates that row, long after newer rows
/// arrived. Under the old rule the row was final, so the event was dropped and the output
/// was lost.
#[test]
fn a_late_event_reaches_an_old_row() {
    let mut state = state_with_history();
    let rows_before = state.rows.len();

    state.apply(
        &AgentEvent::AgentFinished {
            id: AgentId(0),
            report: AgentReport {
                agent: "reviewer 0".to_string(),
                outcome: AgentOutcome::Done,
                summary: "read the diff".to_string(),
                usage: Usage {
                    input_tokens: 1400,
                    output_tokens: 42,
                    ..Default::default()
                },
                turns: 3,
                transcript: None,
            },
        },
        999_000,
    );

    assert_eq!(
        state.rows.len(),
        rows_before,
        "a late event must update a row, and never append one"
    );
    let oldest = state.rows.first().expect("the oldest row");
    let finished = matches!(oldest, rho_tui::Row::Agent { finished: true, .. });
    assert!(
        finished,
        "the oldest agent row is still running, so the late event was dropped: {oldest:?}"
    );
}
