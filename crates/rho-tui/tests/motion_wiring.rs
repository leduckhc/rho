//! The sweep must be reachable, and stoppable. See `SPEC-wire-the-dead-switches`.
//!
//! `state.animate` was read by `apply_sweep` and never assigned anywhere, so the sweep
//! never drew. `motion_enabled`, `MotionInputs`, and `sweep_frame` had no production
//! caller at all. See `D-motion-answers-to-one-switch`, which corrects an earlier draft
//! that claimed the opposite.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rho_core::AgentEvent;
use rho_tui::{ActivityState, MotionInputs, TuiState, motion_enabled, render};

#[test]
fn motion_is_on_by_default_once_it_is_wired() {
    // The default is on, which is `F-working-motion` as specified. It is new on screen.
    assert!(motion_enabled(MotionInputs {
        tui_motion: true,
        stdout_is_terminal: true,
    }));
}

#[test]
fn the_motion_switch_stops_the_sweep() {
    assert!(!motion_enabled(MotionInputs {
        tui_motion: false,
        stdout_is_terminal: true,
    }));
}

#[test]
fn a_non_terminal_stdout_stops_the_sweep() {
    // A redirected stdout has no cursor to animate, and this rule predates the switch.
    assert!(!motion_enabled(MotionInputs {
        tui_motion: true,
        stdout_is_terminal: false,
    }));
}

/// The footer row of a rendered frame, as plain text.
fn footer(state: &TuiState) -> String {
    let mut terminal = Terminal::new(TestBackend::new(80, 10)).expect("a test terminal");
    terminal
        .draw(|frame| render(state, frame))
        .expect("the frame draws");
    let buffer = terminal.backend().buffer().clone();
    (0..80)
        .map(|x| buffer[(x, 9)].symbol().to_string())
        .collect::<String>()
}

/// A state mid-turn, with the tick that puts the sweep at its brightest.
fn running(animate: bool, tick: u64) -> TuiState {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart, 0);
    assert_eq!(state.activity, ActivityState::Running);
    state.animate = animate;
    state.tick = tick;
    state
}

#[test]
fn the_renderer_draws_the_word_either_way() {
    // Motion never carries the state on its own. The word is there with or without it,
    // so a reader who stops the animation loses nothing but the movement.
    for animate in [true, false] {
        let text = footer(&running(animate, 5));
        assert!(
            text.contains("working"),
            "the state is named in words, animate={animate}: {text}"
        );
    }
}

#[test]
fn the_motion_flag_reaches_the_renderer() {
    // The missing call, at the seam that matters. `state.animate` was read here and
    // assigned nowhere, so this comparison held for the wrong reason: both sides were
    // still. A styled cell differs only when the sweep runs.
    let mut on = Terminal::new(TestBackend::new(80, 10)).expect("a terminal");
    let state_on = running(true, 5);
    on.draw(|frame| render(&state_on, frame)).expect("draws");
    let styled_on = on.backend().buffer().clone();

    let mut off = Terminal::new(TestBackend::new(80, 10)).expect("a terminal");
    let state_off = running(false, 5);
    off.draw(|frame| render(&state_off, frame)).expect("draws");
    let styled_off = off.backend().buffer().clone();

    assert_ne!(
        styled_on, styled_off,
        "with motion on, the swept word must render differently from the still one"
    );
}
