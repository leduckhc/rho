//! Render tests. Every test renders into a `ratatui` `TestBackend`. No test
//! opens a real terminal. See `SPEC-tui` section 7.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rho_core::{AgentEvent, ModelDescriptor, ReasoningEffort, StreamEvent};
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
fn listing_failure_shows_one_error_line() {
    // A listing failure must render as exactly one error line in the picker panel,
    // and the current model must still draw. The panel budget keeps the error visible.
    let mut state = TuiState::default();
    state.model = "seed".to_string();
    state.provider = "openrouter".to_string();
    state.reasoning_effort = Some(ReasoningEffort::Medium);
    state.set_starred_models(vec!["starred-a".to_string()]);
    state.open_model_picker();
    state.set_picker_error("provider timed out");
    let lines = render_to_lines(&state, 60, 12);
    let error_lines: Vec<&String> = lines
        .iter()
        .filter(|line| line.trim().starts_with('⚠'))
        .collect();
    assert_eq!(
        error_lines.len(),
        1,
        "a listing failure draws exactly one error line: {error_lines:?}"
    );
    assert!(
        error_lines[0].contains("provider timed out"),
        "the error line names the failure: {}",
        error_lines[0]
    );
    assert!(
        lines.iter().any(|line| line.contains("seed")),
        "the current model still draws after a listing failure"
    );
}

#[test]
fn stale_picker_rows_render_with_a_stale_tag() {
    // A stale cached row must show a `(stale)` tag, and the current model must not.
    let mut state = TuiState::default();
    state.model = "seed".to_string();
    state.provider = "openrouter".to_string();
    state.reasoning_effort = Some(ReasoningEffort::Medium);
    state.set_starred_models(vec!["starred-a".to_string()]);
    state.open_model_picker();
    state.seed_picker_with_cached_models(&[ModelDescriptor {
        id: "cached-a".to_string(),
        display_name: None,
    }]);
    // Height 16, not 12: the grouped picker now draws a `starred` and a `models` header,
    // which reserve two model-row lines. At height 12 the last model row (`cached-a`) is
    // cut, so the fixture needs the extra rows. The stale-tag assertion is unchanged. See
    // `SPEC-the-model-picker-groups-and-labels-rows` section 3.
    let lines = render_to_lines(&state, 60, 16);
    let stale_lines: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains("(stale)"))
        .collect();
    assert_eq!(
        stale_lines.len(),
        1,
        "exactly one row draws the stale tag: {stale_lines:?}"
    );
    assert!(
        stale_lines[0].contains("cached-a"),
        "the stale row names the cached id: {}",
        stale_lines[0]
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("seed") && line.contains("(stale)")),
        "the current model is not marked stale"
    );
}

#[test]
fn model_picker_draws_provider_header_and_scroll_progress() {
    let mut state = TuiState::default();
    state.model = "seed".to_string();
    state.provider = "openrouter".to_string();
    let models: Vec<ModelDescriptor> = (0..12)
        .map(|i| ModelDescriptor {
            id: format!("model-{i:02}"),
            display_name: None,
        })
        .collect();
    state.set_starred_models(vec![]);
    state.open_model_picker();
    state.append_catalog_models(&models);
    // The panel has a header, a query row, ten visible rows, and a scroll-progress footer.
    let lines = render_to_lines(&state, 60, 30);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("provider: openrouter")),
        "the provider header is dimmed in the picker: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.trim() == "1-10 / 13"),
        "the footer shows the visible range and total: {lines:?}"
    );
    assert!(
        lines.iter().filter(|line| line.contains("model-")).count() <= 10,
        "the picker never draws more than ten model rows: {lines:?}"
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
    state.end_run(true, 100);
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

// ---- The grouped and labelled picker rows. --------------------------------
//
// See `SPEC-the-model-picker-groups-and-labels-rows` and
// `D-the-picker-draws-a-starred-section`, `D-a-picker-row-labels-its-vendor`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn picker_key(state: &mut TuiState, code: KeyCode) {
    state.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// A picker with a current model, one starred row, and two catalog rows.
fn grouped_picker_state() -> TuiState {
    let mut state = TuiState::default();
    state.model = "openai/seed".to_string();
    state.provider = "openrouter".to_string();
    state.reasoning_effort = Some(ReasoningEffort::Medium);
    state.set_starred_models(vec!["anthropic/star-a".to_string()]);
    state.set_suggested_models(vec![
        "anthropic/cat-x".to_string(),
        "openai/cat-y".to_string(),
    ]);
    state.open_model_picker();
    state
}

/// The trimmed text of every rendered row that carries a reversed cell.
fn reversed_rows(state: &TuiState, width: u16, height: u16) -> Vec<String> {
    let mut out = Vec::new();
    for row in 0..height {
        let cells = row_cells(state, width, height, row);
        if cells.iter().any(|(_, _, m)| m.contains(Modifier::REVERSED)) {
            let text: String = cells.iter().map(|(s, _, _)| s.as_str()).collect();
            out.push(text.trim().to_string());
        }
    }
    out
}

#[test]
fn a_header_is_never_the_highlighted_row() {
    let mut state = grouped_picker_state();
    // Drive the selection through every model row and back up.
    let presses = [
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Down,
        KeyCode::Up,
        KeyCode::Up,
    ];
    for code in presses {
        picker_key(&mut state, code);
        for text in reversed_rows(&state, 60, 20) {
            assert_ne!(
                text, "starred",
                "the starred header is highlighted: {text:?}"
            );
            assert_ne!(text, "models", "the models header is highlighted: {text:?}");
        }
    }
}

#[test]
fn the_drawn_rows_match_filtered_indices_for_a_divergent_query() {
    let mut state = TuiState::default();
    state.model = "x/seed".to_string();
    state.provider = "openrouter".to_string();
    state.set_starred_models(vec![]);
    state.set_suggested_models(vec![
        "anthropic/claude".to_string(),
        "openai/gpt-4o".to_string(),
    ]);
    state.open_model_picker();
    // `c/c` is a subsequence of the full id `anthropic/claude`, but not of the model name
    // `claude` nor the vendor `anthropic`. The new filter drops it.
    for ch in "c/c".chars() {
        picker_key(&mut state, KeyCode::Char(ch));
    }
    let filtered_len = match &state.panel {
        rho_tui::Panel::ModelPicker(picker) => picker.filtered_indices(&state.starred_models).len(),
        other => panic!("picker not open: {other:?}"),
    };
    assert_eq!(filtered_len, 0, "the divergent query matches no row");
    let lines = render_to_lines(&state, 60, 20);
    let model_rows = lines
        .iter()
        .filter(|l| l.contains('★') || l.contains('☆'))
        .count();
    assert_eq!(
        model_rows, filtered_len,
        "the renderer draws exactly the filtered rows: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("claude")),
        "no full-id-only row draws: {lines:?}"
    );
}

#[test]
fn a_starred_header_draws_above_the_starred_rows() {
    let state = grouped_picker_state();
    let lines = render_to_lines(&state, 60, 20);
    let header_idx = lines
        .iter()
        .position(|l| l.trim() == "starred")
        .expect("a starred header draws");
    let star_idx = lines
        .iter()
        .position(|l| l.contains("star-a"))
        .expect("a starred row draws");
    assert!(header_idx < star_idx, "the header is above the starred row");
    assert_eq!(
        star_idx,
        header_idx + 1,
        "the header sits directly above the first starred row: {lines:?}"
    );
}

#[test]
fn a_models_header_draws_above_the_catalog_rows() {
    let state = grouped_picker_state();
    let lines = render_to_lines(&state, 60, 20);
    let header_idx = lines
        .iter()
        .position(|l| l.trim() == "models")
        .expect("a models header draws");
    let cat_idx = lines
        .iter()
        .position(|l| l.contains("cat-x"))
        .expect("a catalog row draws");
    assert!(header_idx < cat_idx, "the header is above the catalog row");
    assert_eq!(
        cat_idx,
        header_idx + 1,
        "the header sits directly above the first catalog row: {lines:?}"
    );
}

#[test]
fn a_row_draws_the_name_then_a_dim_vendor() {
    let state = grouped_picker_state();
    let row = find_row(&state, 60, 20, "star-a");
    let cells = row_cells(&state, 60, 20, row);
    let line: String = cells.iter().map(|(s, _, _)| s.as_str()).collect();
    let name_at = line.find("star-a").expect("the model name draws");
    let vendor_at = line.find("anthropic").expect("the vendor draws");
    assert!(
        name_at < vendor_at,
        "the model name draws before the vendor: {line:?}"
    );
    for (symbol, fg, _) in cells.iter().skip(vendor_at).take("anthropic".len()) {
        assert_eq!(*fg, MUTED, "vendor cell {symbol:?} is not dim");
    }
}

#[test]
fn a_row_draws_the_effort_suffix_and_none_omits_it() {
    let state = grouped_picker_state();
    let lines = render_to_lines(&state, 60, 20);
    let current = lines
        .iter()
        .find(|l| l.contains("(current)"))
        .expect("the current row draws");
    assert!(
        current.contains("[effort=medium]"),
        "the current row draws its preview effort: {current:?}"
    );
    let starred = lines
        .iter()
        .find(|l| l.contains("star-a"))
        .expect("the starred row draws");
    assert!(
        !starred.contains("[effort="),
        "a row with no preview effort draws no suffix: {starred:?}"
    );
}

#[test]
fn an_empty_vendor_draws_no_separator() {
    let mut state = TuiState::default();
    state.model = "openai/seed".to_string();
    state.provider = "openrouter".to_string();
    state.set_starred_models(vec![]);
    state.set_suggested_models(vec!["plain-model".to_string()]);
    state.open_model_picker();
    let lines = render_to_lines(&state, 60, 20);
    let row = lines
        .iter()
        .find(|l| l.contains("plain-model"))
        .expect("the vendorless row draws");
    assert_eq!(
        row.trim(),
        "☆ plain-model",
        "a vendorless id draws no trailing separator: {row:?}"
    );
}

#[test]
fn a_narrow_row_drops_the_vendor_first() {
    let mut state = TuiState::default();
    state.model = "openai/seed".to_string();
    state.provider = "openrouter".to_string();
    state.set_starred_models(vec![]);
    state.set_suggested_models(vec!["anthropic/claude-sonnet".to_string()]);
    state.open_model_picker();
    let lines = render_to_lines(&state, 22, 20);
    assert!(
        lines.iter().any(|l| l.contains("claude-sonnet")),
        "the model name stays at a narrow width: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("anthropic")),
        "the vendor yields first at a narrow width: {lines:?}"
    );
}

/// The width the row *measures* for the separator must equal the width it *draws*.
///
/// `picker_row_line` decides `include_vendor` from `2 + vendor.width()`, and then draws a
/// literal two-space separator. Those are two constants that must agree. A test that pins
/// only the drawn gap leaves the measurement free, and a mutation from `2` to `3` survived
/// the whole suite. This test pins the boundary, so the measurement cannot drift either way.
///
/// The arithmetic for `anthropic/claude`, with no effort and no tag:
///   `  ☆ claude` is 10 columns, the separator is 2, and `anthropic` is 9. So 21 columns fit
///   the vendor and 20 do not.
#[test]
fn the_vendor_separator_width_is_measured_as_it_is_drawn() {
    let row_at = |width: u16| -> String {
        let mut state = TuiState::default();
        state.model = "openai/seed".to_string();
        state.provider = "openrouter".to_string();
        state.set_starred_models(vec![]);
        state.set_suggested_models(vec!["anthropic/claude".to_string()]);
        state.open_model_picker();
        render_to_lines(&state, width, 20)
            .iter()
            .find(|l| l.contains("claude"))
            .cloned()
            .expect("the model row draws at every width under test")
    };

    // One column short: the vendor cannot fit, so it yields and the name stays.
    //
    // Assert the exact kept text, not the absence of the vendor word. A measurement of
    // `1 + vendor.width()` includes the vendor here, overruns the frame by one column, and
    // the backend clips the last character. `!contains("anthropic")` still held against
    // that bug, because the clipped row reads `anthropi`. The exact assertion catches it.
    let narrow = row_at(20);
    assert_eq!(
        narrow.trim(),
        "☆ claude",
        "at 20 columns the vendor does not fit and must yield whole: {narrow:?}"
    );

    // Exactly enough: the vendor fits with a two-space separator, so it must draw.
    // A measurement of `3 + vendor.width()` would drop it here.
    let exact = row_at(21);
    assert!(
        exact.contains("anthropic"),
        "at 21 columns the vendor fits with a two-space separator: {exact:?}"
    );
    assert!(
        exact.contains("claude  anthropic"),
        "the drawn separator is exactly two spaces at the boundary: {exact:?}"
    );
}

#[test]
fn the_vendor_is_sanitized_before_it_draws() {
    let mut state = TuiState::default();
    state.model = "openai/seed".to_string();
    state.provider = "openrouter".to_string();
    state.set_starred_models(vec![]);
    state.set_suggested_models(vec!["anthropic\u{1b}[31m/claude".to_string()]);
    state.open_model_picker();
    let text = render_to_string(&state, 60, 20);
    assert!(
        !text.contains('\u{1b}'),
        "an escape sequence reached the buffer: {text:?}"
    );
    // `TestBackend` drops a bare ESC on its own, so the assertion above holds even with no
    // filter at all. The body is what reaches a real terminal, so assert the body is gone.
    assert!(
        !text.contains("[31m"),
        "the escape body reached the buffer: {text:?}"
    );
    assert!(
        text.contains("anthropic"),
        "the safe vendor draws: {text:?}"
    );
    assert!(
        text.contains("claude"),
        "the safe model name draws: {text:?}"
    );
}

#[test]
fn the_model_name_is_sanitized_before_it_draws() {
    let mut state = TuiState::default();
    state.model = "openai/seed".to_string();
    state.provider = "openrouter".to_string();
    state.set_starred_models(vec![]);
    state.set_suggested_models(vec![
        "\u{1b}[31mgpt-4o".to_string(),
        "foo/\u{1b}[31mbar".to_string(),
    ]);
    state.open_model_picker();
    let text = render_to_string(&state, 60, 20);
    assert!(
        !text.contains('\u{1b}'),
        "an escape sequence reached the buffer: {text:?}"
    );
    // `TestBackend` drops a bare ESC on its own, so the assertion above holds even with no
    // filter at all. The body is what reaches a real terminal, so assert the body is gone.
    // A mutation that deleted `sanitize_line` from this column survived without this line.
    assert!(
        !text.contains("[31m"),
        "the escape body reached the buffer: {text:?}"
    );
    assert!(
        text.contains("gpt-4o"),
        "the safe vendorless name draws: {text:?}"
    );
    assert!(
        text.contains("bar"),
        "the safe name after a separator draws: {text:?}"
    );
}

#[test]
fn the_headers_reduce_the_model_row_budget() {
    // A constrained height, so the panel budget binds. More headers leave fewer model
    // rows. See `SPEC-the-model-picker-groups-and-labels-rows` section 3.
    let mut state = TuiState::default();
    state.model = "seed".to_string();
    state.provider = "openrouter".to_string();
    state.set_starred_models(vec!["anthropic/star-a".to_string()]);
    let cats: Vec<String> = (0..10).map(|i| format!("cat-{i:02}")).collect();
    state.set_suggested_models(cats);
    state.open_model_picker();

    let vr_two_headers =
        rho_tui::picker_viewport_rows(&state, 60, 14).expect("a viewport row count");
    // The draw reserves the same header rows. At this constrained height the model rows
    // drawn equal the viewport count, and the scroll footer is not truncated. A draw that
    // forgets to reserve the headers over-draws and truncates the footer.
    let lines = render_to_lines(&state, 60, 14);
    let drawn = lines
        .iter()
        .filter(|l| l.contains('★') || l.contains('☆'))
        .count();
    assert_eq!(
        drawn, vr_two_headers,
        "the draw reserves the header rows too: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.trim().ends_with("/ 12")),
        "the scroll footer is not truncated at a constrained height: {lines:?}"
    );
    // Filter to the catalog only, which drops the starred header.
    for ch in "cat".chars() {
        picker_key(&mut state, KeyCode::Char(ch));
    }
    let vr_one_header =
        rho_tui::picker_viewport_rows(&state, 60, 14).expect("a viewport row count");
    assert!(
        vr_two_headers < vr_one_header,
        "two headers leave fewer model rows than one: {vr_two_headers} vs {vr_one_header}"
    );
}

#[test]
fn two_headers_and_a_full_catalog_fit_the_panel_budget() {
    let mut state = TuiState::default();
    state.model = "openai/seed".to_string();
    state.provider = "openrouter".to_string();
    state.set_starred_models(vec!["anthropic/star-a".to_string()]);
    let cats: Vec<String> = (0..10).map(|i| format!("cat-{i:02}")).collect();
    state.set_suggested_models(cats);
    state.open_model_picker();

    let width = 60;
    let height = 26;
    let viewport =
        rho_tui::picker_viewport_rows(&state, width, height).expect("a viewport row count");
    let lines = render_to_lines(&state, width, height);
    assert!(
        lines.iter().any(|l| l.trim() == "starred"),
        "the starred header draws: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.trim() == "models"),
        "the models header draws: {lines:?}"
    );
    let model_rows = lines
        .iter()
        .filter(|l| l.contains('★') || l.contains('☆'))
        .count();
    assert_eq!(
        model_rows, viewport,
        "the drawn model rows equal the viewport budget: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.trim() == "1-10 / 12"),
        "the scroll footer is not truncated: {lines:?}"
    );
}

#[test]
fn the_picker_keeps_a_header_and_fills_its_grant_at_every_scroll_offset() {
    // Defect 1. A long two-section picker, scrolled past the starred section. The old draw
    // reserved two header rows but drew neither once the section's first row scrolled off
    // the top. So the composer jumped up, blank rows appeared, and catalog rows that would
    // fit were hidden. A reviewer reproduced it at width 60, height 14, after 15 Down
    // presses. This test asserts the invariant at every offset, not one hand-picked offset.
    // See `SPEC-the-model-picker-groups-and-labels-rows` section 3.
    let width = 60;
    let height = 14;
    let mut state = TuiState::default();
    state.model = "openai/seed".to_string();
    state.provider = "openrouter".to_string();
    state.set_starred_models(vec!["anthropic/star-a".to_string()]);
    let cats: Vec<String> = (0..20).map(|i| format!("anthropic/cat-{i:02}")).collect();
    state.set_suggested_models(cats);
    state.open_model_picker();

    let starred = state.starred_models.clone();
    // Mimic the app loop: measure the viewport, then keep the selection visible.
    let measure = |state: &mut TuiState| {
        let rows = rho_tui::picker_viewport_rows(state, width, height);
        if let rho_tui::Panel::ModelPicker(picker) = &mut state.panel {
            picker.viewport_rows = rows;
            picker.scroll_to_selection(&starred);
        }
    };
    measure(&mut state);

    let mut saw_catalog_top = false;
    let mut footer_row: Option<usize> = None;
    for _ in 0..=15 {
        let promise =
            rho_tui::picker_viewport_rows(&state, width, height).expect("a viewport row count");
        let lines = render_to_lines(&state, width, height);
        let drawn = lines
            .iter()
            .filter(|l| l.contains('★') || l.contains('☆'))
            .count();
        assert_eq!(
            drawn, promise,
            "the panel draws exactly its promised rows at this offset: {lines:?}"
        );

        // The footer row must not move. A panel that returns fewer lines than its grant
        // lets the composer and footer jump up. See section 3.
        let this_footer = lines
            .iter()
            .position(|l| l.contains("/ 22"))
            .expect("the scroll footer draws");
        match footer_row {
            None => footer_row = Some(this_footer),
            Some(row) => assert_eq!(
                this_footer, row,
                "the footer row moved, so the panel underfilled its grant: {lines:?}"
            ),
        }

        // A section header must stay at every offset, so section context never vanishes.
        let (offset, starred_end, top_index) = match &state.panel {
            rho_tui::Panel::ModelPicker(picker) => {
                let filtered = picker.filtered_indices(&starred);
                let sec = picker.sections(&starred);
                let off = picker.scroll_offset;
                (off, sec.current + sec.starred, filtered[off])
            }
            other => panic!("picker not open: {other:?}"),
        };
        assert!(
            lines
                .iter()
                .any(|l| l.trim() == "starred" || l.trim() == "models"),
            "a section header draws at offset {offset}: {lines:?}"
        );
        if top_index >= starred_end {
            saw_catalog_top = true;
            assert!(
                lines.iter().any(|l| l.trim() == "models"),
                "the models header stays when scrolled into the catalog at offset {offset}: {lines:?}"
            );
        }

        picker_key(&mut state, KeyCode::Down);
        measure(&mut state);
    }
    assert!(
        saw_catalog_top,
        "the drive must scroll past the starred section to prove the fix"
    );
}

#[test]
fn a_control_sequence_in_a_picker_error_cannot_reach_the_screen() {
    // Defect 2. The picker error comes from a provider failure, so it is untrusted. It must
    // pass through `sanitize_line` like every other column. This mirrors
    // `the_vendor_is_sanitized_before_it_draws`. See section 4.
    let mut state = TuiState::default();
    state.model = "openai/seed".to_string();
    state.provider = "openrouter".to_string();
    state.set_starred_models(vec![]);
    state.open_model_picker();
    state.set_picker_error("boom\u{1b}[31mred");
    let text = render_to_string(&state, 60, 20);
    assert!(
        !text.contains('\u{1b}'),
        "an escape sequence in the picker error reached the buffer: {text:?}"
    );
    // `sanitize_line` drops the whole sequence, so the `[31m` bytes vanish too. The
    // backend drops only the bare escape, so the leftover `[31m` proves a missing
    // sanitize call. See `sanitize.rs` and section 4.
    assert!(
        !text.contains("[31m"),
        "the escape body leaked past the missing sanitize call: {text:?}"
    );
    assert!(
        text.contains("boomred"),
        "the safe error text draws whole: {text:?}"
    );
}

#[test]
fn the_vendor_separator_is_exactly_two_spaces() {
    // Defect 3. The gap between the model name and the dim vendor is two spaces. A `<=`
    // bound would let the constant shrink in silence, so this pins the exact width. See
    // section 4.
    let state = grouped_picker_state();
    let row = find_row(&state, 60, 20, "star-a");
    let line: String = row_cells(&state, 60, 20, row)
        .iter()
        .map(|(s, _, _)| s.as_str())
        .collect();
    let name_at = line.find("star-a").expect("the model name draws");
    let vendor_at = line.find("anthropic").expect("the vendor draws");
    let gap = &line[name_at + "star-a".len()..vendor_at];
    assert_eq!(
        gap, "  ",
        "the vendor separator is exactly two spaces: {line:?}"
    );
}

#[test]
fn a_region_only_id_never_draws_an_empty_name() {
    // Defect 4. The region-prefix rule strips `us.anthropic` down to an empty model name,
    // so the row would draw with no name. A row must never draw an empty name. It falls
    // back to the full id, because the full id is what `Enter` applies. See section 4.
    let mut state = TuiState::default();
    state.model = "openai/seed".to_string();
    state.provider = "openrouter".to_string();
    state.set_starred_models(vec![]);
    state.set_suggested_models(vec!["us.anthropic".to_string()]);
    state.open_model_picker();
    let lines = render_to_lines(&state, 60, 20);
    let row = lines
        .iter()
        .find(|l| l.contains('☆') && l.contains("anthropic"))
        .expect("the region-only row draws");
    // Strip the indent and the star, then assert a name remains.
    let after_star = row
        .split_once('☆')
        .map(|(_, rest)| rest.trim())
        .unwrap_or_default();
    assert!(
        !after_star.is_empty(),
        "the row draws a non-empty name: {row:?}"
    );
    assert!(
        after_star.contains("us.anthropic"),
        "the row falls back to the full id for its name: {row:?}"
    );
}
