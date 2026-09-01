//! The two minute tour. See `SPEC-tui-guide`.
//!
//! rho promised a tour in two places and built none. The slash list carried
//! `/guide`, and the first frame advertised it as a starter hint, so the first screen
//! a new user saw invited them to run a command that failed.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rho_tui::{
    Guide, MAX_GUIDE_PAGE_ROWS, Panel, TuiState, bindings, guide_footer_hint, guide_pages, render,
    slash_commands,
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(code: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(code), KeyModifiers::CONTROL)
}

/// Type a command into the draft, character by character, as a user does.
fn typed(state: &mut TuiState, text: &str) {
    for ch in text.chars() {
        state.handle_key(key(KeyCode::Char(ch)));
    }
}

/// A state carrying a model and a provider, so page one has something to name.
fn state_with_context() -> TuiState {
    let mut state = TuiState::default();
    state.model = "anthropic/claude-haiku-4.5".to_string();
    state.provider = "openrouter".to_string();
    state
}

/// Open the guide the way a user does, through the command.
fn opened() -> TuiState {
    let mut state = state_with_context();
    typed(&mut state, "/guide");
    state.handle_key(key(KeyCode::Enter));
    state
}

/// The page index, or a failure when the guide is not open.
fn page_of(state: &TuiState) -> usize {
    match state.panel {
        Panel::Guide(Guide { page }) => page,
        ref other => panic!("the guide is not open: {other:?}"),
    }
}

/// Every drawn row of a frame, as plain text.
fn frame_rows(state: &TuiState, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
    terminal
        .draw(|frame| render(state, frame))
        .expect("the frame draws");
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// The frame rows joined, for a substring assertion.
fn frame_text(state: &TuiState, width: u16, height: u16) -> String {
    frame_rows(state, width, height).join("\n")
}

// ---- Opening, paging, and closing. ----------------------------------------

#[test]
fn the_guide_command_opens_the_first_page() {
    let state = opened();
    assert_eq!(page_of(&state), 0, "the guide opens at page one");
}

#[test]
fn the_guide_has_at_least_three_pages() {
    // The product promises a tour, not a screen. See D-the-guide-is-a-paged-panel.
    let pages = guide_pages("m", "p");
    assert!(pages.len() >= 3, "a tour has pages: {}", pages.len());
}

#[test]
fn right_moves_to_the_next_page() {
    let mut state = opened();
    state.handle_key(key(KeyCode::Right));
    assert_eq!(page_of(&state), 1);
}

#[test]
fn space_moves_to_the_next_page() {
    // A reader holds one key, so space advances too.
    let mut state = opened();
    state.handle_key(key(KeyCode::Char(' ')));
    assert_eq!(page_of(&state), 1);
}

#[test]
fn left_moves_to_the_previous_page() {
    let mut state = opened();
    state.handle_key(key(KeyCode::Right));
    state.handle_key(key(KeyCode::Left));
    assert_eq!(page_of(&state), 0);
}

#[test]
fn right_on_the_last_page_does_not_advance() {
    let mut state = opened();
    let last = guide_pages(&state.model, &state.provider).len() - 1;
    for _ in 0..last + 3 {
        state.handle_key(key(KeyCode::Right));
    }
    assert_eq!(page_of(&state), last, "the last page holds");
    assert!(
        matches!(state.panel, Panel::Guide(_)),
        "a next press never closes the panel"
    );
}

#[test]
fn space_on_the_last_page_does_not_advance() {
    // Both next keys are pinned, so one cannot regress behind the other.
    let mut state = opened();
    let last = guide_pages(&state.model, &state.provider).len() - 1;
    for _ in 0..last + 3 {
        state.handle_key(key(KeyCode::Char(' ')));
    }
    assert_eq!(page_of(&state), last);
    assert!(matches!(state.panel, Panel::Guide(_)));
}

#[test]
fn the_first_page_does_not_go_back() {
    let mut state = opened();
    state.handle_key(key(KeyCode::Left));
    state.handle_key(key(KeyCode::Left));
    assert_eq!(page_of(&state), 0, "no underflow");
}

#[test]
fn escape_closes_the_guide_and_keeps_the_draft() {
    let mut state = state_with_context();
    typed(&mut state, "keep me");
    state.open_guide();
    assert!(
        matches!(state.panel, Panel::Guide(_)),
        "the guide must be open before Esc proves anything"
    );
    state.handle_key(key(KeyCode::Esc));
    assert_eq!(state.panel, Panel::None);
    assert_eq!(state.draft_text(), "keep me", "a draft belongs to the user");
}

#[test]
fn reopening_the_guide_starts_at_the_first_page() {
    let mut state = opened();
    state.handle_key(key(KeyCode::Right));
    state.handle_key(key(KeyCode::Esc));
    typed(&mut state, "/guide");
    state.handle_key(key(KeyCode::Enter));
    assert_eq!(page_of(&state), 0, "a second run is not a resume");
}

// ---- The keyboard belongs to the panel. -----------------------------------

#[test]
fn a_chord_does_not_leak_through_the_guide() {
    // `handle_chord` runs before the panel match, and it guarded only the history
    // search. So ctrl-r swapped the panel and ctrl-u edited the draft behind the
    // open tour. A review found it before any code existed.
    let mut state = state_with_context();
    typed(&mut state, "keep me");
    state.open_guide();

    state.handle_key(ctrl('r'));
    assert!(
        matches!(state.panel, Panel::Guide(_)),
        "ctrl-r must not swap the panel"
    );

    state.handle_key(ctrl('u'));
    assert_eq!(
        state.draft_text(),
        "keep me",
        "a chord must not edit the draft behind the guide"
    );
}

#[test]
fn a_printable_key_does_not_reach_the_draft() {
    let mut state = state_with_context();
    state.open_guide();
    state.handle_key(key(KeyCode::Char('x')));
    assert!(state.draft_is_empty(), "a letter is not draft text here");
}

#[test]
fn an_unknown_key_leaves_the_guide_unchanged() {
    let mut state = opened();
    state.handle_key(key(KeyCode::Right));
    assert!(
        matches!(state.panel, Panel::Guide(_)),
        "the guide must be open for this to mean anything"
    );
    let before = state.clone();
    state.handle_key(key(KeyCode::Tab));
    state.handle_key(key(KeyCode::Backspace));
    assert_eq!(state.panel, before.panel);
    assert_eq!(state.draft_text(), before.draft_text());
}

#[test]
fn ctrl_c_still_cancels_with_the_guide_open() {
    // Ctrl-C is answered before any panel routing, so it keeps its meaning.
    let mut state = state_with_context();
    state.apply(&rho_core::AgentEvent::TurnStart, 0);
    state.open_guide();
    let action = state.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert_eq!(action, rho_tui::KeyAction::Cancel);
    assert!(
        matches!(state.panel, Panel::Guide(_)),
        "a cancel does not close the tour"
    );
}

// ---- The panel really draws. ---------------------------------------------

#[test]
fn the_panel_draws_the_page_title_first() {
    let state = opened();
    let pages = guide_pages(&state.model, &state.provider);
    let title = pages[0].title;
    assert!(!title.is_empty(), "a page carries a real title");
    let rows = frame_rows(&state, 100, 30);
    // The title must lead the panel, so the first row that holds it comes before every
    // body row of that page.
    let title_at = rows
        .iter()
        .position(|row| row.contains(title))
        .unwrap_or_else(|| panic!("the title row draws: {rows:?}"));
    let first_body = pages[0]
        .rows
        .iter()
        .find(|row| !row.trim().is_empty())
        .expect("a page has a body row");
    let body_at = rows
        .iter()
        .position(|row| row.contains(first_body.trim()))
        .unwrap_or_else(|| panic!("a body row draws: {rows:?}"));
    assert!(title_at < body_at, "the title leads the page");
}

#[test]
fn the_first_page_draws_the_model_and_the_provider() {
    // Asserted on the page, never on the whole frame: the banner already names both, so
    // a frame-wide search would pass with no page at all.
    let state = opened();
    let page = &guide_pages(&state.model, &state.provider)[0];
    let body = page.rows.join("\n");
    assert!(
        body.contains("anthropic/claude-haiku-4.5"),
        "page one names the live model: {body}"
    );
    assert!(
        body.contains("openrouter"),
        "page one names the live provider: {body}"
    );
    // And it really reaches the screen.
    let drawn = frame_text(&state, 100, 30);
    assert!(
        drawn.contains(page.title),
        "the page reaches the frame: {drawn}"
    );
}

#[test]
fn the_second_page_names_the_read_only_and_sandbox_switches() {
    let mut state = opened();
    state.handle_key(key(KeyCode::Right));
    let text = frame_text(&state, 100, 30);
    assert!(text.contains("--read-only"), "the safety page: {text}");
    assert!(text.contains("--sandbox"));
}

#[test]
fn the_third_page_draws_a_key_and_its_summary() {
    let mut state = opened();
    state.handle_key(key(KeyCode::Right));
    state.handle_key(key(KeyCode::Right));
    let pages = guide_pages(&state.model, &state.provider);
    let last = pages.last().expect("a last page");
    let row = last
        .rows
        .iter()
        .find(|row| !row.trim().is_empty())
        .expect("the keys page has rows");
    // A whole row, not a bare key: `enter` alone already appears in the footer hint, so a
    // key-only search would pass with no page drawn.
    let text = frame_text(&state, 100, 30);
    assert!(
        text.contains(row.trim()),
        "the keys page row draws: {row:?} not in {text}"
    );
}

#[test]
fn a_narrow_terminal_does_not_split_a_row() {
    let state = opened();
    let rows = frame_rows(&state, 40, 20);
    let title = guide_pages(&state.model, &state.provider)[0].title;
    assert!(
        rows.iter()
            .any(|row| row.contains(title) || !row.is_empty()),
        "the frame draws something to measure"
    );
    for row in rows {
        assert!(
            row.chars().count() <= 40,
            "a row must fit the width: {row:?}"
        );
    }
}

#[test]
fn no_page_is_taller_than_the_page_cap() {
    for page in guide_pages("m", "p") {
        assert!(
            page.rows.len() <= MAX_GUIDE_PAGE_ROWS,
            "page {:?} holds {} rows, over the cap",
            page.title,
            page.rows.len()
        );
    }
}

// ---- Nothing drifts. -----------------------------------------------------

#[test]
fn the_keys_page_matches_the_binding_table() {
    // Existence alone was the weak test a review rejected: a generated page can only
    // ever hold real keys. So the row must carry the binding's own summary too.
    // Row by row, never by substring: `enter` appears inside another binding's summary,
    // so a whole-body search reports a collision rather than a defect.
    let pages = guide_pages("m", "p");
    let keys_page = pages.last().expect("a last page");
    let mut matched = 0;
    for row in &keys_page.rows {
        let trimmed = row.trim_start();
        let found = bindings()
            .iter()
            .filter(|binding| trimmed.starts_with(binding.keys))
            // The longest key wins, so `ctrl-x ctrl-e` is not read as `ctrl-x`.
            .max_by_key(|binding| binding.keys.len())
            .unwrap_or_else(|| panic!("the row {row:?} names no key from the table"));
        assert!(
            row.contains(found.summary),
            "the row for {} must carry the table's own summary, not a copy",
            found.keys
        );
        matched += 1;
    }
    assert!(matched >= 3, "the keys page names real bindings: {matched}");
}

#[test]
fn the_commands_the_guide_names_are_built() {
    // A tour must never send a reader to a command that answers "not built yet".
    let pages = guide_pages("m", "p");
    let body: String = pages
        .iter()
        .flat_map(|page| page.rows.clone())
        .collect::<Vec<String>>()
        .join("\n");
    for command in slash_commands() {
        if body.contains(command.name) {
            assert!(
                command.built,
                "the guide names {}, which is not built",
                command.name
            );
        }
    }
}

#[test]
fn the_footer_numbers_pages_from_one() {
    let hint = guide_footer_hint(0, 3);
    assert!(hint.contains("page 1 of 3"), "one-based display: {hint}");
}

#[test]
fn the_footer_names_the_page_and_the_close_key() {
    let hint = guide_footer_hint(1, 3);
    assert!(hint.contains("page 2 of 3"), "{hint}");
    assert!(hint.contains("esc"), "the exit is always named: {hint}");
}

#[test]
fn the_guide_never_sends_a_prompt() {
    let mut state = opened();
    for code in [
        KeyCode::Right,
        KeyCode::Left,
        KeyCode::Char(' '),
        KeyCode::Enter,
        KeyCode::Tab,
    ] {
        let action = state.handle_key(key(code));
        assert!(
            !matches!(action, rho_tui::KeyAction::Submit(_)),
            "no page reaches the model"
        );
    }
    assert!(
        state
            .rows
            .iter()
            .all(|row| !matches!(row, rho_tui::Row::User { .. })),
        "the transcript gains no user row"
    );
}

// ---- The slash list tells the truth about a command. ----------------------

#[test]
fn the_slash_list_marks_an_unbuilt_command() {
    // The help screen already marks an unwired key. The command list said nothing, so a
    // user spent a keystroke to learn that a command does nothing. `/sessions` is the
    // current unbuilt example.
    let mut state = state_with_context();
    state.handle_key(key(KeyCode::Char('/')));
    let text = frame_text(&state, 100, 30);
    assert!(
        text.contains("/sessions"),
        "the list draws the command: {text}"
    );
    let marked: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("/sessions") && line.contains("not built yet"))
        .collect();
    assert!(
        !marked.is_empty(),
        "an unbuilt command says so in the list: {text}"
    );
}
