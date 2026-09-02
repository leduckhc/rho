//! Render tests. Every test renders into a `ratatui` `TestBackend`. No test
//! opens a real terminal. See `SPEC-tui` section 7.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rho_core::{AgentEvent, StreamEvent};
use rho_tui::{TuiState, render};
use unicode_width::UnicodeWidthStr;

/// Render the state into a backend of the given size and return the frame text.
fn render_to_lines(state: &TuiState, width: u16, height: u16) -> Vec<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("build test terminal");
    terminal
        .draw(|frame| render(state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    let mut lines = Vec::new();
    for y in 0..height {
        let mut line = String::new();
        for x in 0..width {
            line.push_str(buffer[(x, y)].symbol());
        }
        lines.push(line);
    }
    lines
}

fn render_to_string(state: &TuiState, width: u16, height: u16) -> String {
    render_to_lines(state, width, height).join("\n")
}

#[test]
fn render_shows_transcript_rows() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::Stream(StreamEvent::TextStart { index: 0 }), 0);
    state.apply(
        &AgentEvent::Stream(StreamEvent::TextDelta {
            index: 0,
            delta: "the answer".to_string(),
        }),
        0,
    );

    let text = render_to_string(&state, 40, 10);
    assert!(text.contains("the answer"), "buffer was:\n{text}");
}

#[test]
fn render_shows_status_line() {
    let mut state = TuiState::default();
    state.model = "openai/gpt-4o".to_string();
    state.apply(&AgentEvent::TurnStart, 0);

    let text = render_to_string(&state, 60, 6);
    // The controller ruled that the U4 design supersedes the sprint-1 vocabulary:
    // a running turn reads `working` in the footer, not `running`. See the
    // controller finding on slice U4-D.
    assert!(text.contains("working"), "buffer was:\n{text}");
    // The banner now draws on the top row of the screen, because rho owns the whole screen
    // and nothing sits above it. It was frozen above the inline band before, so this test
    // used to assert the opposite. See `D-alternate-screen-after-all`.
    assert!(
        text.contains("openai/gpt-4o"),
        "the banner must carry the model id on screen:\n{text}"
    );
    assert!(
        rho_tui::banner_line(&state, 60).contains("openai/gpt-4o"),
        "the banner must carry the model id"
    );
}

#[test]
fn render_shows_input_buffer() {
    let mut state = TuiState::default();
    state.draft.set_text("hello there");

    let text = render_to_string(&state, 40, 6);
    assert!(text.contains("hello there"), "buffer was:\n{text}");
}

#[test]
fn render_thinking_row_is_dimmed_and_collapsed() {
    let mut state = TuiState::default();
    state.apply(
        &AgentEvent::Stream(StreamEvent::ThinkingStart { index: 0 }),
        0,
    );
    state.apply(
        &AgentEvent::Stream(StreamEvent::ThinkingDelta {
            index: 0,
            delta: "first line\nsecond line\nthird line".to_string(),
        }),
        0,
    );
    // The controller ruled that the U4 design supersedes the sprint-1 thinking
    // row shape: a thinking block renders the `∴` glyph and a duration, not its
    // text. This test keeps the property it protects: a multi-line block still
    // collapses to one line, and the later lines never render.
    state.row_durations = vec![Some(2_400)];

    let lines = render_to_lines(&state, 60, 8);
    assert!(
        lines.iter().any(|line| line.contains("∴ thought for 2.4s")),
        "the thinking row did not take the ∴ glyph and its duration; lines were:\n{lines:#?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("second line")),
        "the thinking block did not collapse; lines were:\n{lines:#?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("third line")),
        "the thinking block did not collapse; lines were:\n{lines:#?}"
    );
}

#[test]
fn render_lines_fit_width() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::Stream(StreamEvent::TextStart { index: 0 }), 0);
    state.apply(
        &AgentEvent::Stream(StreamEvent::TextDelta {
            index: 0,
            delta: "x".repeat(500),
        }),
        0,
    );
    state.draft.set_text(&"y".repeat(500));

    let width = 40u16;
    let lines = render_to_lines(&state, width, 10);
    for line in &lines {
        assert!(
            line.trim_end().width() <= width as usize,
            "a rendered line was wider than the frame: {line:?}"
        );
    }
}

#[test]
fn render_sanitises_a_tool_preview_with_an_escape_sequence() {
    // Tool output is untrusted. A real escape sequence must not reach the buffer.
    let mut state = TuiState::default();
    state.apply(
        &AgentEvent::ToolStart {
            id: "call-1".to_string(),
            name: "bash".to_string(),
            kind: rho_core::ToolKind::Execute,
        },
        0,
    );
    state.apply(
        &AgentEvent::ToolUpdate {
            id: "call-1".to_string(),
            output: "red\u{1b}[31mtext\u{1b}[0m".to_string(),
        },
        0,
    );

    let text = render_to_string(&state, 60, 6);
    assert!(
        !text.contains('\u{1b}'),
        "an escape byte reached the buffer:\n{text:?}"
    );
    // The visible letters survive.
    assert!(text.contains("red"), "buffer was:\n{text}");
}

// ---- Telling the user what the state is. ----------------------------------
//
// `state.status` was written in five places and drawn in none, so the arming
// message for a second Ctrl-C never reached the screen. A user then reported that
// Ctrl-C does not close the session, when in fact it closes 11 ms after the second
// press. These tests pin the feedback, not the mechanism.

#[test]
fn the_footer_tells_the_user_to_press_ctrl_c_again() {
    let mut state = TuiState::default();
    state.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('c'),
        crossterm::event::KeyModifiers::CONTROL,
    ));
    assert!(state.exit_armed, "the first Ctrl-C arms the gate");
    let frame = render_to_string(&state, 100, 12);
    assert!(
        frame.contains("ctrl-c again"),
        "the armed exit gate must be on screen, got:\n{frame}"
    );
}

#[test]
fn the_footer_says_canceling_while_a_cancel_is_in_flight() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart, 0);
    state.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('c'),
        crossterm::event::KeyModifiers::CONTROL,
    ));
    let frame = render_to_string(&state, 100, 12);
    assert!(
        frame.contains("canceling"),
        "a cancel must show on screen, got:\n{frame}"
    );
}

#[test]
fn a_mouse_row_maps_to_the_command_under_it() {
    let mut state = TuiState::default();
    state.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('/'),
        crossterm::event::KeyModifiers::NONE,
    ));
    let height: u16 = 24;
    let lines = render_to_lines(&state, 100, height);
    let commands = rho_tui::filter_slash_commands("/");

    // Find the drawn row of the last command, then ask the mapper for it.
    let last = commands[commands.len() - 1].name;
    let row = lines
        .iter()
        .position(|line| line.contains(last))
        .expect("the last command must be drawn") as u16;
    assert_eq!(
        rho_tui::slash_row_index(&state, 100, height, row),
        Some(commands.len() - 1),
        "the mapper and the renderer must agree on the row"
    );

    // A click on the footer is not a command row.
    assert_eq!(
        rho_tui::slash_row_index(&state, 100, height, height - 1),
        None
    );
}

#[test]
fn the_model_picker_footer_hits_a_space_between_status_and_hint() {
    // Regression: a long hint string butted into `ready` as `readytype filter` on an
    // 88-column terminal. The left side is `ready` plus margins and separators, and the
    // right side is the hint. There must be at least one blank cell between them.
    let mut state = TuiState::default();
    state.model = "seed".to_string();
    state.provider = "openrouter".to_string();
    state.open_model_picker();
    let lines = render_to_lines(&state, 88, 24);
    let footer = lines.last().expect("a frame has a footer");
    let left_part = footer.find("ready").map(|start| &footer[start..]);
    assert!(
        left_part.is_some(),
        "the footer names the idle state: {footer}"
    );
    let left_text = left_part.unwrap();
    // There must be a space before the hint begins, i.e. before the first word of it.
    assert!(
        left_text.contains("ready "),
        "the footer separates `ready` from the hint with a space: {footer}"
    );
    assert!(
        footer.contains("enter pick") || footer.contains("tab effort"),
        "the picker hint is on the footer: {footer}"
    );
}

#[test]
fn the_footer_stops_saying_canceling_when_the_run_dies_without_an_end_event() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart, 0);
    state.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('c'),
        crossterm::event::KeyModifiers::CONTROL,
    ));
    state.end_run(true);
    let frame = render_to_string(&state, 100, 12);
    assert!(
        !frame.contains("canceling"),
        "the cancel word outlived the run:\n{frame}"
    );
    assert!(
        !frame.contains("working"),
        "the working word outlived the run:\n{frame}"
    );
}

#[test]
fn a_click_below_a_dropped_panel_maps_to_nothing() {
    // At a small height the panel is dropped, so no row belongs to a command. Without
    // this guard a click could run a command the user never saw.
    let mut state = TuiState::default();
    state.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('/'),
        crossterm::event::KeyModifiers::NONE,
    ));
    for row in 0..4u16 {
        assert_eq!(
            rho_tui::slash_row_index(&state, 100, 4, row),
            None,
            "a four-row frame draws no panel, so row {row} is not a command"
        );
    }
}

// ---- Contrast. -------------------------------------------------------------
//
// No test pinned a style, which is how a double-dim shipped. `docs/tui-design.md`
// section 3 gives three columns: a 256-colour value, a 16-colour value, and a
// no-colour modifier set. They are alternatives, one per terminal mode. `style_for`
// applied the 256 colour AND the no-colour modifier together, so every muted row was
// grey 245 and DIM as well. The measured 5.19:1 ratio in the design assumes 245 alone.
// A user reported the footer as almost invisible.

use ratatui::style::{Color, Modifier};

/// Render, and return the cells of one row as (symbol, foreground, modifiers).
fn row_cells(
    state: &TuiState,
    width: u16,
    height: u16,
    row: u16,
) -> Vec<(String, Color, Modifier)> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("build test terminal");
    terminal
        .draw(|frame| render(state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    (0..width)
        .map(|x| {
            let cell = &buffer[(x, row)];
            (cell.symbol().to_string(), cell.fg, cell.modifier)
        })
        .collect()
}

fn find_row(state: &TuiState, width: u16, height: u16, needle: &str) -> u16 {
    render_to_lines(state, width, height)
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no row holds {needle:?}")) as u16
}

const MUTED: Color = Color::Indexed(245);

#[test]
fn a_muted_cell_is_grey_but_never_dim_as_well() {
    // The root cause. Grey 245 plus DIM is two dimmings, and the design measured one.
    let state = TuiState::default();
    for row in 0..12u16 {
        for (symbol, fg, modifier) in row_cells(&state, 100, 12, row) {
            if fg == MUTED {
                assert!(
                    !modifier.contains(Modifier::DIM),
                    "row {row} cell {symbol:?} is grey 245 and DIM as well"
                );
            }
        }
    }
}

#[test]
fn the_placeholder_is_muted() {
    // `docs/tui-design.md` section 8: the placeholder shows in `muted`. It drew in the
    // default foreground, which reads as bright as the answer text.
    let state = TuiState::default();
    let row = find_row(&state, 100, 12, "Type a prompt");
    let cells = row_cells(&state, 100, 12, row);
    let placeholder: Vec<&(String, Color, Modifier)> = cells
        .iter()
        .filter(|(symbol, _, _)| symbol.trim() == "Type".chars().next().unwrap().to_string())
        .collect();
    assert!(!placeholder.is_empty(), "the placeholder row has no T");
    for (symbol, fg, _) in placeholder {
        assert_eq!(*fg, MUTED, "placeholder cell {symbol:?} is not muted");
    }
}

#[test]
fn the_footer_activity_word_reads_as_text_and_the_hints_stay_muted() {
    // The user reported `done · end turn` as almost invisible. The whole footer row was
    // painted muted, including the activity word, which the design gives to `text`.
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart, 0);
    state.apply(
        &AgentEvent::AgentEnd {
            stop_reason: rho_core::AgentStopReason::EndTurn,
        },
        0,
    );
    let row = find_row(&state, 100, 12, "done");
    let cells = row_cells(&state, 100, 12, row);
    let line: String = cells.iter().map(|(symbol, _, _)| symbol.as_str()).collect();
    let word_at = line.find("done").expect("the activity word is on the row");
    let hint_at = line.find("enter send").expect("the hints are on the row");

    assert_ne!(
        cells[word_at].1, MUTED,
        "the activity word must not be muted: {line:?}"
    );
    assert!(
        !cells[word_at].2.contains(Modifier::DIM),
        "the activity word must not be dim: {line:?}"
    );
    assert_eq!(
        cells[hint_at].1, MUTED,
        "the key hints stay muted: {line:?}"
    );
}
