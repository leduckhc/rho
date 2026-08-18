//! Input tests. The key handler is pure, so these tests need no terminal.
//! See `SPEC-tui` section 7.

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

// ---- Wiring the promised keys. --------------------------------------------
//
// The panels, the help screen, and the slash list were built, rendered, and unit
// tested, and no key ever reached them. A user pressed `/`, got a literal slash in
// the draft, and reported the feature as missing. These tests pin the wiring, so
// the interface cannot promise a key it does not answer.

fn ctrl(code: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(code), KeyModifiers::CONTROL)
}

fn typed(state: &mut TuiState, text: &str) {
    for ch in text.chars() {
        state.handle_key(key(KeyCode::Char(ch)));
    }
}

fn panel_of(state: &TuiState) -> &rho_tui::Panel {
    &state.panel
}

#[test]
fn ctrl_d_on_an_empty_draft_exits() {
    let mut state = TuiState::default();
    assert_eq!(state.handle_key(ctrl('d')), KeyAction::Exit);
}

#[test]
fn ctrl_d_with_a_draft_keeps_the_draft_and_inserts_nothing() {
    let mut state = TuiState::default();
    typed(&mut state, "hi");
    assert_eq!(state.handle_key(ctrl('d')), KeyAction::None);
    // The old handler pushed the letter of any chord into the draft.
    assert_eq!(state.input, "hi");
}

#[test]
fn a_control_chord_never_becomes_a_letter() {
    let mut state = TuiState::default();
    for chord in ['o', 'e', 'd', 'x'] {
        state.handle_key(ctrl(chord));
    }
    assert_eq!(state.input, "", "a chord must never type its letter");
}

#[test]
fn slash_on_an_empty_draft_opens_the_command_list() {
    let mut state = TuiState::default();
    state.handle_key(key(KeyCode::Char('/')));
    match panel_of(&state) {
        rho_tui::Panel::SlashList(list) => {
            assert_eq!(list.query, "/");
            assert_eq!(list.selected, 0);
        }
        other => panic!("expected the slash list, got {other:?}"),
    }
}

#[test]
fn the_slash_list_filters_as_the_user_types() {
    let mut state = TuiState::default();
    typed(&mut state, "/qu");
    match panel_of(&state) {
        rho_tui::Panel::SlashList(list) => assert_eq!(list.query, "/qu"),
        other => panic!("expected the slash list, got {other:?}"),
    }
    assert_eq!(rho_tui::filter_slash_commands("/qu").len(), 1);
}

#[test]
fn the_arrows_move_the_slash_selection_and_stop_at_the_ends() {
    let mut state = TuiState::default();
    state.handle_key(key(KeyCode::Char('/')));
    let count = rho_tui::filter_slash_commands("/").len();
    assert!(count >= 2, "this test needs at least two commands");

    state.handle_key(key(KeyCode::Down));
    match panel_of(&state) {
        rho_tui::Panel::SlashList(list) => assert_eq!(list.selected, 1),
        other => panic!("expected the slash list, got {other:?}"),
    }
    // Up from the first row stays on the first row.
    state.handle_key(key(KeyCode::Up));
    state.handle_key(key(KeyCode::Up));
    match panel_of(&state) {
        rho_tui::Panel::SlashList(list) => assert_eq!(list.selected, 0),
        other => panic!("expected the slash list, got {other:?}"),
    }
    // Down past the last row stays on the last row.
    for _ in 0..count + 3 {
        state.handle_key(key(KeyCode::Down));
    }
    match panel_of(&state) {
        rho_tui::Panel::SlashList(list) => assert_eq!(list.selected, count - 1),
        other => panic!("expected the slash list, got {other:?}"),
    }
}

#[test]
fn enter_runs_the_selected_command_and_quit_exits() {
    let mut state = TuiState::default();
    typed(&mut state, "/quit");
    assert_eq!(state.handle_key(key(KeyCode::Enter)), KeyAction::Exit);
}

#[test]
fn enter_never_submits_the_slash_text_as_a_prompt() {
    let mut state = TuiState::default();
    typed(&mut state, "/help");
    let action = state.handle_key(key(KeyCode::Enter));
    assert_eq!(
        action,
        KeyAction::None,
        "a command must not reach the model"
    );
    assert_eq!(*panel_of(&state), rho_tui::Panel::Help);
    assert!(state.input.is_empty(), "running a command clears the draft");
}

#[test]
fn question_mark_on_an_empty_draft_opens_help() {
    let mut state = TuiState::default();
    state.handle_key(key(KeyCode::Char('?')));
    assert_eq!(*panel_of(&state), rho_tui::Panel::Help);
    assert!(
        state.input.is_empty(),
        "the key must not type a question mark"
    );
}

#[test]
fn question_mark_inside_a_draft_is_just_text() {
    let mut state = TuiState::default();
    typed(&mut state, "why?");
    assert_eq!(state.input, "why?");
    assert_eq!(*panel_of(&state), rho_tui::Panel::None);
}

#[test]
fn esc_closes_a_panel() {
    let mut state = TuiState::default();
    state.handle_key(key(KeyCode::Char('?')));
    assert_eq!(*panel_of(&state), rho_tui::Panel::Help);
    state.handle_key(key(KeyCode::Esc));
    assert_eq!(*panel_of(&state), rho_tui::Panel::None);
}

#[test]
fn backspace_past_the_slash_closes_the_list() {
    let mut state = TuiState::default();
    typed(&mut state, "/q");
    state.handle_key(key(KeyCode::Backspace));
    state.handle_key(key(KeyCode::Backspace));
    assert_eq!(*panel_of(&state), rho_tui::Panel::None);
    assert!(state.input.is_empty());
}

#[test]
fn a_command_that_is_not_built_reports_instead_of_doing_nothing() {
    let mut state = TuiState::default();
    typed(&mut state, "/model");
    state.handle_key(key(KeyCode::Enter));
    let reported = state.rows.iter().any(|row| match row {
        rho_tui::Row::Error { message, .. } => message.contains("/model"),
        _ => false,
    });
    assert!(
        reported,
        "an unbuilt command must say so on screen: {:?}",
        state.rows
    );
}

#[test]
fn an_unknown_command_reports_on_screen() {
    let mut state = TuiState::default();
    typed(&mut state, "/nope");
    state.handle_key(key(KeyCode::Enter));
    let reported = state.rows.iter().any(|row| match row {
        rho_tui::Row::Error { message, .. } => message.contains("/nope"),
        _ => false,
    });
    assert!(reported, "an unknown command must report: {:?}", state.rows);
}

#[test]
fn esc_keeps_the_draft_so_a_path_is_not_lost() {
    // A user typing a path opens the list by accident, because a path starts with a
    // slash. Esc must close the panel and keep every character, because throwing away
    // a draft to close a panel is worse than the panel.
    let mut state = TuiState::default();
    typed(&mut state, "/usr/bin/foo");
    state.handle_key(key(KeyCode::Esc));
    assert_eq!(*panel_of(&state), rho_tui::Panel::None);
    assert_eq!(state.input, "/usr/bin/foo");
    // From here it is ordinary text, and Enter sends it.
    assert_eq!(
        state.handle_key(key(KeyCode::Enter)),
        KeyAction::Submit("/usr/bin/foo".to_string())
    );
}

#[test]
fn an_unknown_command_keeps_the_draft_it_reported_on() {
    let mut state = TuiState::default();
    typed(&mut state, "/nope");
    state.handle_key(key(KeyCode::Enter));
    assert_eq!(state.input, "/nope", "reporting must not eat the draft");
    assert_eq!(*panel_of(&state), rho_tui::Panel::None);
}

#[test]
fn tab_completes_the_selected_command_without_running_it() {
    // The user asked for the list to be selectable by mouse, enter, or tab. Tab
    // completes the draft, the way a shell completes a path, and it runs nothing.
    let mut state = TuiState::default();
    typed(&mut state, "/mo");
    state.handle_key(key(KeyCode::Tab));
    assert_eq!(state.input, "/model");
    match panel_of(&state) {
        rho_tui::Panel::SlashList(list) => assert_eq!(list.query, "/model"),
        other => panic!("tab must keep the list open, got {other:?}"),
    }
    assert!(
        state.rows.is_empty(),
        "tab must not run the command: {:?}",
        state.rows
    );
}

#[test]
fn tab_on_an_empty_filter_changes_nothing() {
    let mut state = TuiState::default();
    typed(&mut state, "/zz");
    let before = state.input.clone();
    state.handle_key(key(KeyCode::Tab));
    assert_eq!(state.input, before);
}

// ---- A run that ends with no AgentEnd. -------------------------------------
//
// `rho_core::Driver::run` returns on `TurnOutcome::Failed` and `Closed` without emitting
// `AgentEnd` (crates/rho-core/src/agent.rs:344). The TUI cleared only its stream handles,
// so `activity` stayed `Running` for the rest of the session. A review found it, and the
// consequence is worse than a stale word: Ctrl-C then routes to a cancel on a token that
// is gone, so Ctrl-C can never quit again.

#[test]
fn a_run_that_ends_with_no_stop_event_returns_to_idle() {
    let mut state = TuiState::default();
    state.apply(&rho_core::AgentEvent::TurnStart);
    state.handle_key(ctrl_c());
    assert!(state.canceling, "a cancel while running sets the flag");

    state.end_run(true);
    assert_eq!(state.activity, ActivityState::Idle);
    assert!(!state.canceling, "the flag must not outlive the run");
    assert!(state.last_error, "a failed run is a failed run");
}

#[test]
fn ctrl_c_can_still_quit_after_a_run_that_never_ended() {
    let mut state = TuiState::default();
    state.apply(&rho_core::AgentEvent::TurnStart);
    state.handle_key(ctrl_c());
    state.end_run(true);

    // The two-press gate must work again, because the run is over.
    assert_eq!(state.handle_key(ctrl_c()), KeyAction::None);
    assert_eq!(state.handle_key(ctrl_c()), KeyAction::Exit);
}

#[test]
fn a_cancel_that_closes_the_stream_reads_as_canceled() {
    let mut state = TuiState::default();
    state.apply(&rho_core::AgentEvent::TurnStart);
    state.handle_key(ctrl_c());
    state.end_run(false);
    assert_eq!(state.activity, ActivityState::Idle);
    assert!(!state.last_error, "a cancel is not an error");
}

#[test]
fn the_canceling_flag_clears_when_the_turn_ends() {
    let mut state = TuiState::default();
    state.apply(&rho_core::AgentEvent::TurnStart);
    state.handle_key(ctrl_c());
    state.apply(&rho_core::AgentEvent::AgentEnd {
        stop_reason: rho_core::AgentStopReason::Canceled,
    });
    assert!(!state.canceling, "AgentEnd must clear the cancel flag");
}

#[test]
fn a_click_runs_the_command_on_that_row() {
    // The row must not be index 0. A first version of this test clicked `/model`, which
    // is the first row, so it passed even when the click always ran row 0.
    let mut state = TuiState::default();
    state.handle_key(key(KeyCode::Char('/')));
    let commands = rho_tui::filter_slash_commands("/");
    let index = commands
        .iter()
        .position(|command| command.name == "/guide")
        .expect("/guide is in the list");
    assert!(index > 0, "this test needs a row below the first one");

    state.click_slash_row(index);
    let messages: Vec<&String> = state
        .rows
        .iter()
        .filter_map(|row| match row {
            rho_tui::Row::Error { message, .. } => Some(message),
            _ => None,
        })
        .collect();
    assert!(
        messages.iter().any(|message| message.contains("/guide")),
        "the click must run the clicked row: {messages:?}"
    );
    assert!(
        !messages.iter().any(|message| message.contains("/model")),
        "the click ran the first row instead of the clicked one: {messages:?}"
    );
}

#[test]
fn a_click_disarms_the_exit_gate() {
    // The footer promises that any key keeps the session. A click is a key press to a
    // user, so it must not leave the gate armed behind their back.
    // Arm the gate after the panel is open. A first version armed it first, and the
    // `/` key press disarmed it, so the test proved nothing about the click.
    let mut state = TuiState::default();
    state.handle_key(key(KeyCode::Char('/')));
    state.handle_key(ctrl_c());
    assert!(state.exit_armed, "Ctrl-C while idle arms the gate");
    state.click_slash_row(0);
    assert!(!state.exit_armed, "a click must disarm the gate");
}

#[test]
fn an_error_row_is_sanitised() {
    let mut state = TuiState::default();
    state.push_error("a\u{1b}[31mred\u{7} message");
    match state.rows.first() {
        Some(rho_tui::Row::Error { message, .. }) => {
            assert!(!message.contains('\u{1b}'), "escape survived: {message:?}");
            assert!(!message.contains('\u{7}'), "bell survived: {message:?}");
        }
        other => panic!("expected an error row, got {other:?}"),
    }
}
