//! Input tests. The key handler is pure, so these tests need no terminal.
//! See `SPEC-05` section 7.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rho_core::AgentEvent;
use rho_tui::{ActivityState, KeyAction, TuiState};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl_c() -> KeyEvent {
    KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
}

#[test]
fn input_key_appends_while_running() {
    let mut state = TuiState::default();
    // Drive the state into a running turn.
    state.apply(&AgentEvent::TurnStart);
    assert_eq!(state.activity, ActivityState::Running);

    // A key press appends even while the model streams. Input never blocks on
    // model work.
    state.handle_key(key(KeyCode::Char('h')));
    state.handle_key(key(KeyCode::Char('i')));

    assert_eq!(state.input, "hi");
    assert_eq!(state.activity, ActivityState::Running);
}

#[test]
fn input_backspace_removes_last_char() {
    let mut state = TuiState::default();
    state.handle_key(key(KeyCode::Char('a')));
    state.handle_key(key(KeyCode::Char('b')));
    state.handle_key(key(KeyCode::Backspace));
    assert_eq!(state.input, "a");
}

#[test]
fn input_enter_submits_and_clears() {
    let mut state = TuiState::default();
    state.handle_key(key(KeyCode::Char('h')));
    state.handle_key(key(KeyCode::Char('i')));
    let action = state.handle_key(key(KeyCode::Enter));

    assert_eq!(action, KeyAction::Submit("hi".to_string()));
    assert_eq!(state.input, "");
    assert_eq!(state.rows.len(), 1);
}

#[test]
fn ctrl_c_while_running_cancels_not_exits() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart);

    let action = state.handle_key(ctrl_c());
    assert_eq!(action, KeyAction::Cancel);
    // A cancel must not arm the idle exit gate.
    assert!(!state.exit_armed);
}

#[test]
fn ctrl_c_twice_while_idle_exits() {
    let mut state = TuiState::default();
    // The state starts idle.
    let first = state.handle_key(ctrl_c());
    assert_eq!(first, KeyAction::None);
    assert!(state.exit_armed);

    let second = state.handle_key(ctrl_c());
    assert_eq!(second, KeyAction::Exit);
}

#[test]
fn ctrl_c_once_while_idle_does_not_exit() {
    let mut state = TuiState::default();
    let action = state.handle_key(ctrl_c());
    assert_eq!(action, KeyAction::None);
}

#[test]
fn key_after_first_ctrl_c_disarms_exit() {
    let mut state = TuiState::default();
    state.handle_key(ctrl_c());
    assert!(state.exit_armed);

    // A normal key clears the exit arm.
    state.handle_key(key(KeyCode::Char('a')));
    assert!(!state.exit_armed);

    // A following Ctrl-C only re-arms; it does not exit.
    let action = state.handle_key(ctrl_c());
    assert_eq!(action, KeyAction::None);
}
