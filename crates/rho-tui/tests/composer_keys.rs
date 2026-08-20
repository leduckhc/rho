//! Composer key wiring tests. See `SPEC-tui-inline-and-composer` section 6.3 to 6.6.
//!
//! The composer is a real editor, but no key reached it. A live run typed `ctrl-j`
//! and got one row. These tests pin the wiring: every newline key, the history, the
//! reverse search, and the external editor now answer a key. See section 8's S3 table.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rho_tui::{
    KeyAction, Panel, Row, TuiState, edit_draft, editor_argv, editor_command, filter_history,
    help_rows,
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(code: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(code), KeyModifiers::CONTROL)
}

fn alt(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::ALT)
}

fn shift(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::SHIFT)
}

fn typed(state: &mut TuiState, text: &str) {
    for ch in text.chars() {
        state.handle_key(key(KeyCode::Char(ch)));
    }
}

/// A paste large enough to collapse into one chip.
fn big_paste() -> String {
    "x".repeat(1200)
}

// ---- 6.3 The newline keys. ------------------------------------------------

#[test]
fn ctrl_j_inserts_a_newline() {
    let mut state = TuiState::default();
    typed(&mut state, "line one");
    state.handle_key(ctrl('j'));
    typed(&mut state, "line two");
    assert_eq!(
        state.draft.display_lines(80).len(),
        2,
        "ctrl-j must start a second display row"
    );
}

#[test]
fn alt_enter_inserts_a_newline() {
    let mut state = TuiState::default();
    typed(&mut state, "line one");
    state.handle_key(alt(KeyCode::Enter));
    typed(&mut state, "line two");
    assert_eq!(
        state.draft.display_lines(80).len(),
        2,
        "alt+enter must start a second display row"
    );
}

/// `shift+enter` is the newline key the owner named, and the help screen has advertised
/// it as built from the first stage. It reached no code, because `handle_chord` is only
/// entered when the modifiers hold CONTROL or ALT, and SHIFT is in neither. So the key
/// fell through to the plain `Enter` arm and **sent the draft**. That is worse than a key
/// that does nothing: it destroys the draft the user was still writing.
#[test]
fn shift_enter_inserts_a_newline() {
    let mut state = TuiState::default();
    typed(&mut state, "line one");
    let action = state.handle_key(shift(KeyCode::Enter));
    typed(&mut state, "line two");
    assert!(
        matches!(action, KeyAction::None),
        "shift+enter must not send the draft, and it returned {action:?}"
    );
    assert_eq!(
        state.draft.display_lines(80).len(),
        2,
        "shift+enter must start a second display row"
    );
}

#[test]
fn enter_sends_the_whole_draft() {
    let mut state = TuiState::default();
    let big = big_paste();
    state.draft.paste(&big);
    typed(&mut state, "a");
    state.handle_key(ctrl('j'));
    typed(&mut state, "b");
    let action = state.handle_key(key(KeyCode::Enter));
    match action {
        KeyAction::Submit(text) => {
            assert!(text.contains(&big), "the held paste must reach the model");
            assert!(
                text.contains("a\nb"),
                "both rows must reach the model: {text:?}"
            );
        }
        other => panic!("expected a submit, got {other:?}"),
    }
    assert!(state.draft_is_empty(), "the draft is empty after a send");
}

// ---- 6.4 The history. -----------------------------------------------------

#[test]
fn history_recalls_the_previous_prompt() {
    let mut state = TuiState::default();
    typed(&mut state, "hello");
    state.handle_key(key(KeyCode::Enter));
    // The draft is empty. Up on a one-row draft recalls the last submit.
    state.handle_key(key(KeyCode::Up));
    assert_eq!(state.draft_text(), "hello");
}

#[test]
fn history_recalls_forward_to_the_live_draft() {
    let mut state = TuiState::default();
    typed(&mut state, "hello");
    state.handle_key(key(KeyCode::Enter));
    typed(&mut state, "wip");
    state.handle_key(key(KeyCode::Up));
    assert_eq!(state.draft_text(), "hello", "up recalls the last submit");
    state.handle_key(key(KeyCode::Down));
    assert_eq!(
        state.draft.model_text(),
        "wip",
        "down past the newest restores the draft"
    );
}

#[test]
fn history_does_not_duplicate_a_repeat() {
    let mut state = TuiState::default();
    typed(&mut state, "hi");
    state.handle_key(key(KeyCode::Enter));
    typed(&mut state, "hi");
    state.handle_key(key(KeyCode::Enter));
    assert_eq!(
        state.history.len(),
        1,
        "two identical submits give one entry"
    );
}

#[test]
fn up_moves_the_cursor_in_a_tall_draft() {
    let mut state = TuiState::default();
    typed(&mut state, "ab");
    state.handle_key(ctrl('j'));
    typed(&mut state, "cd");
    let before = state.draft.cursor();
    state.handle_key(key(KeyCode::Up));
    assert_ne!(
        state.draft.cursor(),
        before,
        "the cursor moved, not the history"
    );
    assert_eq!(state.draft_text(), "ab\ncd", "the draft text is unchanged");
}

#[test]
fn esc_esc_clears_the_draft_into_the_history() {
    let mut state = TuiState::default();
    typed(&mut state, "hello");
    state.handle_key(key(KeyCode::Esc));
    state.handle_key(key(KeyCode::Esc));
    assert!(state.draft_is_empty(), "two esc clear the draft");
    state.handle_key(key(KeyCode::Up));
    assert_eq!(state.draft_text(), "hello", "up restores the cleared draft");
}

#[test]
fn one_esc_does_not_clear_the_draft() {
    let mut state = TuiState::default();
    typed(&mut state, "hello");
    state.handle_key(key(KeyCode::Esc));
    assert_eq!(state.draft_text(), "hello", "one esc keeps the draft");
}

// ---- 6.5 Reverse search. --------------------------------------------------

#[test]
fn reverse_search_filters_and_accepts() {
    let mut state = TuiState::default();
    typed(&mut state, "alpha");
    state.handle_key(key(KeyCode::Enter));
    typed(&mut state, "beta");
    state.handle_key(key(KeyCode::Enter));
    state.handle_key(ctrl('r'));
    typed(&mut state, "be");
    state.handle_key(key(KeyCode::Enter));
    assert_eq!(
        state.draft.model_text(),
        "beta",
        "enter puts the matched entry in the draft"
    );
}

#[test]
fn reverse_search_esc_keeps_the_draft() {
    let mut state = TuiState::default();
    typed(&mut state, "wip");
    state.handle_key(ctrl('r'));
    typed(&mut state, "xy");
    state.handle_key(key(KeyCode::Esc));
    assert_eq!(
        state.draft.model_text(),
        "wip",
        "esc keeps the draft typed before ctrl-r"
    );
    assert_eq!(state.panel, Panel::None, "esc closes the search panel");
}

#[test]
fn reverse_search_matches_without_case() {
    let mut state = TuiState::default();
    typed(&mut state, "test");
    state.handle_key(key(KeyCode::Enter));
    state.handle_key(ctrl('r'));
    typed(&mut state, "TEST");
    state.handle_key(key(KeyCode::Enter));
    assert_eq!(state.draft_text(), "test", "TEST matches test");
}

#[test]
fn the_panel_owns_the_keyboard() {
    let mut state = TuiState::default();
    state.handle_key(ctrl('r'));
    typed(&mut state, "z");
    assert!(
        state.draft_is_empty(),
        "a character typed into the search never reaches the draft"
    );
}

#[test]
fn filter_history_is_case_insensitive_and_newest_first() {
    let history = vec!["test one".to_string(), "TEST two".to_string()];
    let hits = filter_history(&history, "test");
    assert_eq!(
        hits,
        vec![1, 0],
        "the match is case-insensitive and newest first"
    );
    assert_eq!(
        filter_history(&history, ""),
        vec![1, 0],
        "an empty query matches every entry"
    );
}

// ---- 6.6 The external editor. ---------------------------------------------

#[test]
fn ctrl_x_ctrl_e_returns_edit_draft() {
    let mut state = TuiState::default();
    typed(&mut state, "hi");
    state.handle_key(ctrl('x'));
    let action = state.handle_key(ctrl('e'));
    assert_eq!(action, KeyAction::EditDraft("hi".to_string()));
}

#[test]
fn ctrl_g_returns_edit_draft() {
    let mut state = TuiState::default();
    typed(&mut state, "hi");
    let action = state.handle_key(ctrl('g'));
    assert_eq!(action, KeyAction::EditDraft("hi".to_string()));
}

#[test]
fn the_editor_command_prefers_visual() {
    assert_eq!(editor_command(Some("code -w"), Some("nano")), "code -w");
}

#[test]
fn the_editor_command_falls_back_to_vi() {
    assert_eq!(editor_command(None, None), "vi");
}

#[test]
fn the_editor_argv_never_reaches_a_shell() {
    let argv = editor_argv("vi; rm -rf ~");
    assert_eq!(argv.len(), 4, "the program plus three arguments");
    assert_eq!(
        &argv[1..],
        &["rm", "-rf", "~"],
        "the tail are three inert arguments"
    );
    assert!(
        !argv.iter().any(|token| token == "-c"),
        "the argv never passes -c to a shell"
    );
    assert!(
        !argv.iter().any(|token| token == "sh" || token == "bash"),
        "the argv never names a shell"
    );
}

#[test]
fn a_failed_editor_keeps_the_draft() {
    let mut state = TuiState::default();
    state.draft.set_text("keep me");
    // `false` exits non-zero, so the run fails and the draft must survive.
    edit_draft(&mut state, &["false".to_string()], "keep me");
    assert_eq!(
        state.draft.model_text(),
        "keep me",
        "a failed editor keeps the draft"
    );
    let errors = state
        .rows
        .iter()
        .filter(|row| matches!(row, Row::Error { .. }))
        .count();
    assert_eq!(errors, 1, "a failed editor pushes one error row");
}

// ---- The help guard. ------------------------------------------------------

#[test]
fn the_help_lists_every_new_key() {
    let rendered = help_rows().join("\n");
    for keys in [
        "enter",
        "ctrl-j",
        "alt+enter",
        "shift+enter",
        "ctrl-r",
        "ctrl-x ctrl-e",
        "ctrl-g",
        "↑ ↓",
        "esc",
    ] {
        assert!(rendered.contains(keys), "the help screen dropped {keys:?}");
    }
}

// ---- The search panel must show what it found. ---------------------------------
//
// The panel drew the query row and nothing else, while its own doc comment claimed it
// drew "the matches, newest first" and marked the selected one. A search that shows no
// result cannot be used, and a doc that disagrees with the code is worse than no doc.

#[test]
fn reverse_search_lists_the_matches() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut state = TuiState::default();
    state.history = vec![
        "run the gate and report".to_string(),
        "say the word pineapple".to_string(),
        "count the rows".to_string(),
    ];
    state.handle_key(ctrl('r'));
    typed(&mut state, "pine");

    let mut terminal = Terminal::new(TestBackend::new(90, 20)).expect("a test terminal");
    terminal
        .draw(|frame| rho_tui::render(&state, frame))
        .expect("the frame draws");

    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();

    assert!(
        text.contains("say the word pineapple"),
        "the panel must list the matched draft"
    );
    assert!(
        !text.contains("count the rows"),
        "the panel must not list a draft that does not match"
    );
}

/// Routing a shifted Enter must not route a shifted letter. A capital letter arrives as
/// `shift+a`, and a chord drops its text, so a wider mask would silently stop the user
/// typing capitals. This pins the boundary.
#[test]
fn shift_still_types_a_capital_letter() {
    let mut state = TuiState::default();
    for ch in "Hello World".chars() {
        state.handle_key(KeyEvent::new(
            KeyCode::Char(ch),
            if ch.is_uppercase() {
                KeyModifiers::SHIFT
            } else {
                KeyModifiers::NONE
            },
        ));
    }
    assert_eq!(
        state.draft.model_text(),
        "Hello World",
        "a shifted letter must reach the draft as text"
    );
}
