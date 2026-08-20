//! The Ledger band rules that still stand, from `D-ledger-wins-the-band`.
//!
//! rho owns the whole screen now, so the fourteen-row budget is gone. The rules that stand
//! are unchanged: the composer keeps two rules around at least one draft row, an approval
//! states its session root and never yields a row, and a tool row leads with its status
//! glyph and indents two columns. See `SPEC-tui-alternate-screen` section 6.
//!
//! The help-window tests are deleted, because the help now draws the whole binding table
//! and no longer windows. See the report.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rho_tui::{Approval, Panel, TuiState, render};

const WIDTH: u16 = 100;

/// The rendered screen at `height` rows, one string per row.
fn screen(state: &TuiState, height: u16) -> Vec<String> {
    let backend = TestBackend::new(WIDTH, height);
    let mut terminal = Terminal::new(backend).expect("build test terminal");
    terminal
        .draw(|frame| render(state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    (0..height)
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

// ---- The help screen draws every key. -------------------------------------

/// The help panel drew one row per binding, which was once more rows than the band held,
/// so the help screen drew no rows at all. The screen has room now, so it draws them all.
#[test]
fn the_help_screen_draws_its_keys() {
    let mut state = TuiState::default();
    state.panel = Panel::Help;
    let joined = screen(&state, 30).join("\n");
    assert!(
        joined.contains("ctrl-c"),
        "the help screen drew no binding row; the screen was:\n{joined}"
    );
    assert!(
        joined.contains("pageup"),
        "the help must state the scroll keys; the screen was:\n{joined}"
    );
}

// ---- The rank under pressure. ---------------------------------------------

/// The footer keeps its row first. It carries the activity word, so a screen without it
/// cannot say whether rho is working or waiting.
#[test]
fn the_footer_keeps_its_row_under_pressure() {
    let mut state = with_draft("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten");
    state.panel = Panel::Help;
    let rows = screen(&state, 8);
    let last = rows.last().expect("a screen row").clone();
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
    let rows = screen(&state, 8);
    let joined = rows.join("\n");
    let rules = rows.iter().filter(|r| r.starts_with('─')).count();
    assert!(
        rules >= 2,
        "the composer must keep both rules; the screen was:\n{joined}"
    );
    assert!(
        joined.contains("the draft must survive"),
        "the composer must keep its draft row; the screen was:\n{joined}"
    );
}

// ---- The approval never yields. -------------------------------------------

/// An approval states a destructive command, so it states all of it, including the session
/// root, even under a tall draft. The composer yields its extra rows instead. This is the
/// retained half of `D-ledger-wins-the-band`, and it must stay passing.
#[test]
fn an_approval_states_its_session_root() {
    let mut state = with_draft("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten");
    state.panel = Panel::Approval(Approval {
        title: "bash asks to run".to_string(),
        command: "rm -rf target && cargo test --workspace".to_string(),
        root: "~/Work/Vibe/rho".to_string(),
        millis: Some(4_000),
    });
    let joined = screen(&state, 14).join("\n");
    assert!(
        joined.contains("rm -rf target"),
        "the approval must state the command; the screen was:\n{joined}"
    );
    assert!(
        joined.contains("~/Work/Vibe/rho"),
        "the approval must state the session root, even under a tall draft; the screen was:\n{joined}"
    );
}

// ---- The tool row grammar. ------------------------------------------------

/// Ledger's tool row leads with its status glyph and indents two columns, and the duration
/// sits flush right. The status leads, because the eye scans a left column and hunts a
/// right one.
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
    let joined = screen(&state, 20).join("\n");
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
