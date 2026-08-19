//! The Ledger band rules, from `D-ledger-wins-the-band` and `docs/design/tui-mock.html`.
//!
//! The band holds fourteen rows, and the regions want twenty. So they yield in a stated
//! rank: the footer keeps its row, a panel takes its rows next, the composer scrolls
//! inside what remains and never drops below three rows, and the live rows yield first.
//!
//! Every test here pins a rule that a real defect broke.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rho_tui::{Approval, BAND_ROWS, Panel, TuiState, bindings, render};

const WIDTH: u16 = 100;

/// The rendered band, one string per row.
fn band(state: &TuiState) -> Vec<String> {
    let backend = TestBackend::new(WIDTH, BAND_ROWS);
    let mut terminal = Terminal::new(backend).expect("build test terminal");
    terminal
        .draw(|frame| render(state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    (0..BAND_ROWS)
        .map(|y| {
            (0..WIDTH)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

fn with_draft(text: &str) -> TuiState {
    let mut state = TuiState::default();
    state.draft.set_text(text);
    state
}

// ---- The help screen. -----------------------------------------------------

/// The help panel built one row per binding, which is more rows than the band holds. So
/// `plan_band` granted the panel nothing, and the help screen drew **no rows at all**.
/// A frame fixture pinned that blank output as correct for a whole stage.
#[test]
fn the_help_screen_draws_its_keys() {
    let mut state = TuiState::default();
    state.panel = Panel::Help;
    let rows = band(&state);
    let joined = rows.join("\n");
    assert!(
        joined.contains("ctrl-c"),
        "the help screen drew no binding row; the band was:\n{joined}"
    );
}

/// The window states where it is, because a list that scrolls without a position lies
/// about its own length. The count comes from the table, never from a literal, so adding
/// or removing a binding cannot make the header wrong.
#[test]
fn the_help_window_counts_the_whole_table() {
    let mut state = TuiState::default();
    state.panel = Panel::Help;
    let joined = band(&state).join("\n");
    let total = bindings().len();
    assert!(
        joined.contains(&format!("of {total}")),
        "the help header must count all {total} bindings; the band was:\n{joined}"
    );
}

/// A window that hides rows must say so, or the user reads the visible rows as the whole
/// table. `D-a-panel-nobody-can-open` forbids a silent promise, and silence about a
/// hidden row is the same defect.
#[test]
fn the_help_window_names_the_hidden_rows() {
    let mut state = TuiState::default();
    state.panel = Panel::Help;
    let joined = band(&state).join("\n");
    assert!(
        joined.contains("more below"),
        "the help window hid rows and did not say so; the band was:\n{joined}"
    );
}

// ---- The rank. ------------------------------------------------------------

/// The footer keeps its row first. It carries the activity word, so a band without it
/// cannot say whether rho is working or waiting.
#[test]
fn the_footer_keeps_its_row_under_pressure() {
    let mut state = with_draft("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten");
    state.panel = Panel::Help;
    let rows = band(&state);
    let last = rows.last().expect("a band row").clone();
    assert!(
        last.contains("help") || last.contains("ready") || last.contains("esc"),
        "the last row must be the footer, and it was {last:?}"
    );
}

/// The composer never drops below three rows: one draft row between two rules. The draft
/// is the one thing the user owns, so it is never erased to fit a panel.
#[test]
fn the_composer_keeps_three_rows_under_pressure() {
    let mut state = with_draft("the draft must survive");
    state.panel = Panel::Help;
    let rows = band(&state);
    let joined = rows.join("\n");
    let rules = rows.iter().filter(|r| r.starts_with('─')).count();
    assert!(
        rules >= 2,
        "the composer must keep both rules; the band was:\n{joined}"
    );
    assert!(
        joined.contains("the draft must survive"),
        "the composer must keep its draft row; the band was:\n{joined}"
    );
}

// ---- The approval never yields. -------------------------------------------

/// An approval states a destructive command, so it states all of it. An early mock cut
/// the session root row to save one row, and the panel then named `rm -rf target` while
/// it hid the directory the command ran in. The composer yields the row instead.
#[test]
fn an_approval_never_yields_its_session_root() {
    let mut state = with_draft("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten");
    state.panel = Panel::Approval(Approval {
        title: "bash asks to run".to_string(),
        command: "rm -rf target && cargo test --workspace".to_string(),
        root: "~/Work/Vibe/rho".to_string(),
        millis: Some(4_000),
    });
    let joined = band(&state).join("\n");
    assert!(
        joined.contains("rm -rf target"),
        "the approval must state the command; the band was:\n{joined}"
    );
    assert!(
        joined.contains("~/Work/Vibe/rho"),
        "the approval must state the session root, even under a tall draft; the band was:\n{joined}"
    );
}

// ---- The tool row grammar. ------------------------------------------------

/// Ledger's tool row leads with its status glyph and indents two columns, and the duration
/// sits flush right. The status led in both references the design compared, because the eye
/// scans a left column and hunts a right one. The row read `read  path … 0.3s ✓` before.
#[test]
fn a_tool_row_leads_with_its_status_and_indents() {
    use rho_core::ToolKind;
    use rho_tui::{Row, RowFold, ToolRowStatus};
    let mut state = TuiState::default();
    state.rows.push(Row::Tool {
        id: "t1".to_string(),
        name: "read".to_string(),
        kind: ToolKind::Read,
        status: ToolRowStatus::Ok,
        preview: "crates/rho-tui/src/render.rs · 220 lines".to_string(),
    });
    state.row_durations.push(Some(300));
    state.row_folds.push(RowFold::Collapsed);
    state.row_bodies.push(Vec::new());
    let joined = band(&state).join("\n");
    let row = joined
        .lines()
        .find(|l| l.contains("read"))
        .unwrap_or_default()
        .to_string();
    assert!(
        row.starts_with("  ✓ read"),
        "a tool row must read `  ✓ read …`, and it read {row:?}"
    );
    assert!(
        row.trim_end().ends_with("0.3s"),
        "the duration must sit flush right, and the row was {row:?}"
    );
}

// ---- The help window answers its own keys. --------------------------------

/// The window states `↓ 14 more below` and the footer offers `↑ ↓ scroll`. Both are
/// promises, so a key must answer them. The window drew the marker and the hint while
/// `handle_help_key` ignored the arrows, which is `D-a-panel-nobody-can-open` exactly.
#[test]
fn the_down_arrow_scrolls_the_help_window() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut state = TuiState::default();
    state.panel = Panel::Help;
    let first = band(&state).join("\n");
    assert!(
        first.contains("1-8 of"),
        "the window must open at the top; it was:\n{first}"
    );
    state.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let scrolled = band(&state).join("\n");
    assert!(
        !scrolled.contains("1-8 of"),
        "the down arrow must move the window, and it stayed:\n{scrolled}"
    );
    // The marker is `↑ n more above` at the end of the list, and the compact `↑ n · ↓ n`
    // in the middle. Both state that rows are above, so this asserts the fact and not one
    // wording of it.
    assert!(
        scrolled.contains('↑'),
        "a scrolled window must say rows are above it:\n{scrolled}"
    );
}

/// The window stops at the last row. A list that scrolls past its end shows blank rows and
/// reads as a defect.
#[test]
fn the_help_window_stops_at_the_end() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut state = TuiState::default();
    state.panel = Panel::Help;
    for _ in 0..60 {
        state.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    let joined = band(&state).join("\n");
    let total = bindings().len();
    assert!(
        joined.contains(&format!("of {total}")),
        "the header must still count the table:\n{joined}"
    );
    assert!(
        !joined.contains("more below"),
        "at the end nothing is below, and it claimed there was:\n{joined}"
    );
    assert!(
        joined.contains("esc"),
        "the last binding must be reachable:\n{joined}"
    );
}

/// Scrolling past the end must not bank presses. The offset was clamped when the window
/// drew and never when it moved, so `help_offset` climbed past the last row. One press of
/// the up arrow then moved nothing, and the window only answered after the user paid back
/// every press they had made. A user found this by hand in a spike, and
/// `the_help_window_stops_at_the_end` missed it, because it asserted the drawn rows and
/// never pressed a key afterwards.
#[test]
fn the_help_window_answers_the_first_press_back() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut state = TuiState::default();
    state.panel = Panel::Help;
    let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
    let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
    // Push far past the end, the way a wheel or a held key does.
    for _ in 0..40 {
        state.handle_key(down);
    }
    let at_end = band(&state).join("\n");
    // One press back must move the window at once.
    state.handle_key(up);
    let after = band(&state).join("\n");
    assert_ne!(
        at_end, after,
        "one press back must move the window, and it banked the presses instead"
    );
}
