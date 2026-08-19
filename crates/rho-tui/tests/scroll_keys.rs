//! The scroll keys, the help, and the wheel. See `SPEC-tui-alternate-screen` sections 5
//! and 9, and `D-scroll-keys-yield-to-an-empty-draft`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rho_tui::{KeyAction, TuiState, bindings, help_rows};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(code: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(code), KeyModifiers::CONTROL)
}

/// A state whose transcript overflows its window, with an empty draft and no panel.
fn overflow_state() -> TuiState {
    let mut state = TuiState::default();
    state.transcript_total = 200;
    state.transcript_visible = 20;
    state
}

/// The keys in section 5 that scroll toward the oldest row.
const UP_KEYS: [KeyCode; 2] = [KeyCode::PageUp, KeyCode::Home];
/// The keys in section 5 that scroll toward the newest row.
const DOWN_KEYS: [KeyCode; 2] = [KeyCode::PageDown, KeyCode::End];

#[test]
fn every_scroll_key_has_a_binding_row() {
    let table = bindings();
    for token in [
        "pageup", "pagedown", "ctrl-u", "ctrl-d", "home", "end", "↑ ↓",
    ] {
        assert!(
            table.iter().any(|b| b.keys.contains(token)),
            "no binding row names {token}"
        );
    }
}

#[test]
fn the_help_states_every_scroll_key() {
    let rows = help_rows();
    for token in [
        "pageup", "pagedown", "ctrl-u", "ctrl-d", "home", "end", "↑ ↓",
    ] {
        assert!(
            rows.iter().any(|row| row.contains(token)),
            "the help does not state {token}"
        );
    }
}

#[test]
fn every_scroll_key_moves_the_view() {
    // A key in the table that moves nothing is the `help_panel` defect again: the table
    // promised the key and the code answered nothing.
    //
    // The set holds only keys with no other meaning. `ctrl-u`, `ctrl-d`, and the arrows keep
    // their shell habits and never scroll, which
    // `ctrl_d_still_quits_while_the_transcript_overflows` pins.
    for code in UP_KEYS {
        let mut state = overflow_state();
        let before = state.scroll_first_visible();
        state.handle_key(key(code));
        assert!(
            state.scroll_first_visible() < before,
            "{code:?} did not move the view toward the oldest row"
        );
    }
    for code in DOWN_KEYS {
        let mut state = overflow_state();
        state.scroll_up(1000); // jump to the oldest row and release the pin
        let before = state.scroll_first_visible();
        state.handle_key(key(code));
        assert!(
            state.scroll_first_visible() > before,
            "{code:?} did not move the view toward the newest row"
        );
    }
}

#[test]
fn a_scroll_key_keeps_its_old_meaning_without_overflow() {
    // With no overflow the transcript cannot scroll, so ctrl-d still exits an empty draft.
    let mut state = TuiState::default();
    assert_eq!(
        state.handle_key(ctrl('d')),
        rho_tui::KeyAction::Exit,
        "ctrl-d must still exit when the transcript does not overflow"
    );
}

#[test]
fn a_scroll_key_keeps_its_old_meaning_with_a_draft() {
    // A non-empty draft blocks the scroll path, so the readline keys keep working.
    let mut state = overflow_state();
    state.handle_key(key(KeyCode::Char('h')));
    let before = state.scroll_first_visible();
    state.handle_key(key(KeyCode::Up));
    assert_eq!(
        state.scroll_first_visible(),
        before,
        "a scroll key must not move the view while the draft holds text"
    );
}

// ---- A habit key never becomes a scroll key. -------------------------------

/// `ctrl-d` quits on an empty draft. That is a shell habit, and the binding table promises
/// it without a condition.
///
/// An earlier rule let every scroll key win on an empty draft, so `ctrl-d` scrolled instead
/// of quitting whenever the transcript overflowed. That is the normal state of a session, so
/// the promise was broken almost always. See `D-scroll-keys-yield-to-an-empty-draft`.
#[test]
fn ctrl_d_still_quits_while_the_transcript_overflows() {
    let mut state = TuiState::default();
    state.transcript_total = 500;
    state.transcript_visible = 20;
    assert!(
        state.transcript_total > state.transcript_visible,
        "this test needs an overflowing transcript to be meaningful"
    );
    let action = state.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    assert!(
        matches!(action, KeyAction::Exit),
        "ctrl-d must quit on an empty draft, and it returned {action:?}"
    );
}

/// `ctrl-u` cuts to the line start. It never scrolls.
#[test]
fn ctrl_u_still_cuts_while_the_transcript_overflows() {
    let mut state = TuiState::default();
    state.transcript_total = 500;
    state.transcript_visible = 20;
    for ch in "hello".chars() {
        state.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    state.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(
        state.draft.model_text(),
        "",
        "ctrl-u must cut the draft, not scroll"
    );
}

/// `↑` recalls the history on an empty draft. It never scrolls.
#[test]
fn the_up_arrow_still_recalls_the_history() {
    let mut state = TuiState::default();
    state.transcript_total = 500;
    state.transcript_visible = 20;
    state.history.push("the earlier draft".to_string());
    state.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(
        state.draft.model_text(),
        "the earlier draft",
        "the up arrow must recall the history, not scroll"
    );
}
