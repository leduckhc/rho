//! The design frames are fixtures, and a fixture that does not line up is a lie.
//!
//! `docs/design/tui-frames/` holds the frames that `docs/tui-design.md` treats as the
//! layout acceptance criteria. This file guards the fixtures themselves: the exact
//! display width, the exact row count, and the absence of a character that would break
//! a monospace grid.
//!
//! The frames now describe the full screen of `SPEC-tui-alternate-screen`. rho owns the
//! whole terminal, so a frame is `FRAME_ROWS` rows tall: the banner at the top, the
//! transcript, the composer, and the footer. The banner is also kept as a separate one-row
//! fixture, because `banner_line` still renders it on its own.
//!
//! **Why a display width and not a length.** A box character such as `─` is three bytes
//! and one column. The controller's first check of these frames used a byte length and
//! reported every line as wrong. So width here means what the terminal draws, measured
//! the same way `crates/rho-tui/src/sanitize.rs` measures it.

use std::path::{Path, PathBuf};

use unicode_width::UnicodeWidthStr;

/// The rows a full-screen frame holds. rho owns the whole terminal now.
const FRAME_ROWS: usize = 24;

fn frames_dir() -> PathBuf {
    // The tests run with the crate root as the working directory.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root")
        .join("docs/design/tui-frames")
}

/// The frame body, without the opening and closing fence.
fn frame_lines(name: &str) -> Vec<String> {
    let path = frames_dir().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    text.lines()
        .filter(|line| !line.starts_with("```"))
        .map(str::to_string)
        .collect()
}

/// Assert one frame is exactly `columns` wide on every row, and `FRAME_ROWS` tall.
fn assert_frame(name: &str, columns: usize) {
    let lines = frame_lines(name);
    assert_eq!(
        lines.len(),
        FRAME_ROWS,
        "{name} must hold {FRAME_ROWS} rows, it holds {}",
        lines.len()
    );
    for (number, line) in lines.iter().enumerate() {
        let width = UnicodeWidthStr::width(line.as_str());
        assert_eq!(
            width,
            columns,
            "{name} row {} is {width} columns, it must be {columns}: {line:?}",
            number + 1
        );
    }
}

/// A frame may hold no character that breaks the grid.
///
/// A wide character occupies two cells, and a combining character occupies none. Either
/// one makes a hand-drawn frame disagree with the terminal, and the disagreement is
/// invisible in a diff.
fn assert_grid_safe(name: &str) {
    for (number, line) in frame_lines(name).iter().enumerate() {
        for character in line.chars() {
            let width = UnicodeWidthStr::width(character.to_string().as_str());
            assert_eq!(
                width,
                1,
                "{name} row {} holds {character:?}, which is {width} columns wide",
                number + 1
            );
        }
    }
}

#[test]
fn frame_fixture_100_idle_is_exact() {
    assert_frame("100-idle.txt", 100);
}

#[test]
fn frame_fixture_100_streaming_is_exact() {
    assert_frame("100-streaming.txt", 100);
}

#[test]
fn frame_fixture_100_tool_run_is_exact() {
    assert_frame("100-tool-run.txt", 100);
}

#[test]
fn frame_fixture_100_approval_is_exact() {
    assert_frame("100-approval.txt", 100);
}

#[test]
fn frame_fixture_100_error_is_exact() {
    assert_frame("100-error.txt", 100);
}

#[test]
fn frame_fixture_100_empty_is_exact() {
    assert_frame("100-empty.txt", 100);
}

#[test]
fn frame_fixture_100_slash_list_is_exact() {
    assert_frame("100-slash-list.txt", 100);
}

#[test]
fn frame_fixture_100_help_is_exact() {
    assert_frame("100-help.txt", 100);
}

#[test]
fn frame_fixture_80_streaming_is_exact() {
    assert_frame("80-streaming.txt", 80);
}

#[test]
fn frame_fixture_40_streaming_is_exact() {
    assert_frame("40-streaming.txt", 40);
}

#[test]
fn frame_fixture_banner_is_exact() {
    // The banner is one row, and it freezes above the band. So it is not `FRAME_ROWS`
    // tall, and it gets its own width check.
    let lines = frame_lines("100-banner.txt");
    assert_eq!(lines.len(), 1, "the banner is one row");
    assert_eq!(
        UnicodeWidthStr::width(lines[0].as_str()),
        100,
        "the banner spans the width"
    );
}

#[test]
fn every_frame_fixture_is_grid_safe() {
    for name in [
        "100-banner.txt",
        "100-idle.txt",
        "100-streaming.txt",
        "100-tool-run.txt",
        "100-approval.txt",
        "100-error.txt",
        "100-empty.txt",
        "100-slash-list.txt",
        "100-help.txt",
        "80-streaming.txt",
        "40-streaming.txt",
    ] {
        assert_grid_safe(name);
    }
}

#[test]
fn the_frame_set_is_complete() {
    // A missing frame would silently reduce the acceptance criteria, so the count is
    // pinned. Adding a frame is a deliberate act that updates this test.
    let mut found: Vec<String> = std::fs::read_dir(frames_dir())
        .expect("the frames directory")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".txt"))
        .collect();
    found.sort();
    assert_eq!(
        found.len(),
        11,
        "the design states ten band frames plus one banner, the directory holds {}: {found:?}",
        found.len()
    );
}

// ---------------------------------------------------------------------------
// Frame render tests. These drive the real renderer through a `TestBackend` and
// compare the rendered text, row by row, against the fixture. A mismatch reports
// the first differing row with both versions, so a failure reads at a glance.
// See `SPEC-tui-experience`, the `## Test cases` section.
// ---------------------------------------------------------------------------

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rho_core::{AgentStopReason, ToolKind};
use rho_tui::{
    ActivityState, Approval, Panel, Row, RowFold, ToolRowStatus, TuiState, banner_line, render,
};

/// The rendered text of a state, one string per row, at the given size.
fn render_rows(state: &TuiState, width: u16) -> Vec<String> {
    let backend = TestBackend::new(width, FRAME_ROWS as u16);
    let mut terminal = Terminal::new(backend).expect("build test terminal");
    terminal
        .draw(|frame| render(state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    let mut rows = Vec::with_capacity(FRAME_ROWS);
    for y in 0..FRAME_ROWS as u16 {
        let mut line = String::new();
        for x in 0..width {
            line.push_str(buffer[(x, y)].symbol());
        }
        rows.push(line);
    }
    rows
}

/// Compare a rendered frame against its fixture, row by row. Report the first
/// differing row with both versions, so a mismatch is a finding, not a puzzle.
fn assert_renders(name: &str, width: u16, state: &TuiState) {
    let actual = render_rows(state, width);
    if std::env::var("RHO_REGEN").is_ok() {
        write_fixture(name, &actual);
        return;
    }
    let expected = frame_lines(name);
    assert_eq!(
        actual.len(),
        expected.len(),
        "{name}: rendered {} rows, fixture has {}",
        actual.len(),
        expected.len()
    );
    for (index, (got, want)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            got.as_str(),
            want.as_str(),
            "\n{name}: row {} differs\n  want: {want:?}\n  got:  {got:?}",
            index + 1
        );
    }
}

/// Write a fenced fixture from rendered rows. It runs only under `RHO_REGEN`, so the
/// design owner regenerates the frames when the band shape changes.
fn write_fixture(name: &str, rows: &[String]) {
    let mut text = String::from("```\n");
    for row in rows {
        text.push_str(row);
        text.push('\n');
    }
    text.push_str("```\n");
    std::fs::write(frames_dir().join(name), text).expect("write the fixture");
}

/// A state seeded with the header fields every 100-column frame shares.
fn base_state() -> TuiState {
    let mut state = TuiState::default();
    state.cwd = "~/Work/Vibe/rho".to_string();
    state.branch = "main".to_string();
    state.model = "sonnet-4.5".to_string();
    state.provider = "openrouter".to_string();
    state.tokens = "48.2k in, 3.1k out".to_string();
    state.session_millis = Some(728_000); // 12m 08s
    state
}

/// Push a row with its parallel duration, fold, and body metadata.
fn push(state: &mut TuiState, row: Row, millis: Option<i64>, fold: RowFold, body: Vec<&str>) {
    state.rows.push(row);
    state.row_durations.push(millis);
    state.row_folds.push(fold);
    state
        .row_bodies
        .push(body.into_iter().map(str::to_string).collect());
}

fn tool(id: &str, name: &str, payload: &str, status: ToolRowStatus) -> Row {
    Row::Tool {
        id: id.to_string(),
        name: name.to_string(),
        kind: ToolKind::Other,
        status,
        preview: payload.to_string(),
    }
}

#[test]
fn frame_100_idle_renders_at_100_columns() {
    let mut state = base_state();
    state.last_stop = Some(AgentStopReason::EndTurn);
    state.turn_millis = Some(41_000);
    push(
        &mut state,
        Row::User {
            text: "add a duration to every tool row, and keep the text beside it still".into(),
        },
        None,
        RowFold::Collapsed,
        vec![],
    );
    push(
        &mut state,
        Row::Thinking {
            text: "reasoning".into(),
        },
        Some(2_400),
        RowFold::Collapsed,
        vec![],
    );
    push(&mut state, Row::Assistant { text: "I will add a seven column duration slot to each tool row, then wire the session clock into the header. The slot is right aligned, so a live tick never moves the glyph beside it.".into() }, None, RowFold::Collapsed, vec![]);
    push(
        &mut state,
        tool(
            "c1",
            "read",
            "crates/rho-tui/src/render.rs · 220 lines",
            ToolRowStatus::Ok,
        ),
        Some(300),
        RowFold::Collapsed,
        vec![],
    );
    push(
        &mut state,
        tool(
            "c2",
            "edit",
            "crates/rho-tui/src/render.rs · +18 -4",
            ToolRowStatus::Ok,
        ),
        Some(200),
        RowFold::Collapsed,
        vec![],
    );
    push(
        &mut state,
        tool("c3", "bash", "cargo test -p rho-tui", ToolRowStatus::Ok),
        Some(41_000),
        RowFold::Collapsed,
        vec![],
    );
    push(
        &mut state,
        Row::Assistant {
            text: "Done. Every tool row now carries a duration, and 214 tests pass.".into(),
        },
        None,
        RowFold::Collapsed,
        vec![],
    );
    assert_renders("100-idle.txt", 100, &state);
}

#[test]
fn frame_100_streaming_renders_at_100_columns() {
    let mut state = base_state();
    state.activity = ActivityState::Running;
    state.turn_millis = Some(12_000);
    state
        .draft
        .set_text("also pin the carry case from the ladder");
    push(
        &mut state,
        Row::User {
            text: "now bind the session clock to the header slot".into(),
        },
        None,
        RowFold::Collapsed,
        vec![],
    );
    push(
        &mut state,
        Row::Thinking {
            text: "reasoning".into(),
        },
        Some(1_800),
        RowFold::Collapsed,
        vec![],
    );
    push(&mut state, Row::Assistant { text: "The header reserves a seven column slot at its right edge. I will drive it from the tick count the state carries, so a test asserts an exact frame with no timer. The renderer stays a pure function of the".into() }, None, RowFold::Collapsed, vec![]);
    assert_renders("100-streaming.txt", 100, &state);
}

#[test]
fn frame_100_tool_run_renders_at_100_columns() {
    let mut state = base_state();
    state.activity = ActivityState::Running;
    state.turn_millis = Some(90_000);
    push(
        &mut state,
        Row::User {
            text: "run the workspace tests and fix what breaks".into(),
        },
        None,
        RowFold::Collapsed,
        vec![],
    );
    push(
        &mut state,
        Row::Assistant {
            text: "Running the whole suite first, so the failures name themselves.".into(),
        },
        None,
        RowFold::Collapsed,
        vec![],
    );
    push(
        &mut state,
        tool(
            "c1",
            "read",
            "crates/rho-tui/src/state.rs · 460 lines",
            ToolRowStatus::Ok,
        ),
        Some(200),
        RowFold::Collapsed,
        vec![],
    );
    push(
        &mut state,
        tool(
            "c2",
            "bash",
            "cargo test --workspace --all-features",
            ToolRowStatus::Running,
        ),
        Some(72_000),
        RowFold::Expanded,
        vec![
            "running 214 tests",
            "test duration_ladder_rounds_once ... ok",
            "test duration_slot_stays_seven_columns ... ok",
            "test reducer_is_pure_same_events_same_state ... ok",
            "test render_lines_fit_width ... ok",
        ],
    );
    assert_renders("100-tool-run.txt", 100, &state);
}

#[test]
fn frame_100_approval_renders_at_100_columns() {
    let mut state = base_state();
    state.turn_millis = Some(41_000);
    state.panel = Panel::Approval(Approval {
        title: "bash asks to run".into(),
        command: "rm -rf target && cargo build --release".into(),
        root: "~/Work/Vibe/rho".into(),
        millis: Some(8_000),
    });
    push(
        &mut state,
        Row::User {
            text: "clean the build tree and rebuild the release binary".into(),
        },
        None,
        RowFold::Collapsed,
        vec![],
    );
    push(
        &mut state,
        Row::Assistant {
            text: "The target directory holds 12 GB. I will remove it, then rebuild.".into(),
        },
        None,
        RowFold::Collapsed,
        vec![],
    );
    assert_renders("100-approval.txt", 100, &state);
}

#[test]
fn frame_100_error_renders_at_100_columns() {
    let mut state = base_state();
    state.last_error = true;
    state.turn_millis = Some(3_100);
    push(
        &mut state,
        Row::User {
            text: "send the report to the tracker".into(),
        },
        None,
        RowFold::Collapsed,
        vec![],
    );
    push(
        &mut state,
        Row::Error {
            message: "openrouter returned 429, too many requests".into(),
            detail: vec![
                "retry-after: 20".into(),
                "the turn stopped after 3.1s, and the transcript is kept".into(),
            ],
        },
        None,
        RowFold::Collapsed,
        vec![],
    );
    assert_renders("100-error.txt", 100, &state);
}

#[test]
fn frame_100_empty_renders_at_100_columns() {
    let mut state = base_state();
    state.session_millis = Some(3_000); // 3s
    assert_renders("100-empty.txt", 100, &state);
}

#[test]
fn frame_banner_renders_at_100_columns() {
    // The banner freezes above the band, so a fixture pins its one row.
    let state = base_state();
    let actual = vec![banner_line(&state, 100)];
    if std::env::var("RHO_REGEN").is_ok() {
        write_fixture("100-banner.txt", &actual);
        return;
    }
    let expected = frame_lines("100-banner.txt");
    assert_eq!(
        expected, actual,
        "the banner fixture must match the renderer"
    );
}

#[test]
fn frame_100_slash_list_renders_at_100_columns() {
    let mut state = base_state();
    // Reach the slash panel through the key handler, so the fixture builds a state a
    // real key can build. A hand-built panel proves nothing. See `D-a-panel-nobody-can-open`.
    state.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    push(
        &mut state,
        Row::User {
            text: "add a duration to every tool row, and keep the text beside it still".into(),
        },
        None,
        RowFold::Collapsed,
        vec![],
    );
    push(
        &mut state,
        Row::Assistant {
            text: "Done. Every tool row now carries a duration, and 214 tests pass.".into(),
        },
        None,
        RowFold::Collapsed,
        vec![],
    );
    assert_renders("100-slash-list.txt", 100, &state);
}

#[test]
fn frame_100_help_renders_at_100_columns() {
    let mut state = base_state();
    state.panel = Panel::Help;
    push(
        &mut state,
        Row::User {
            text: "add a duration to every tool row, and keep the text beside it still".into(),
        },
        None,
        RowFold::Collapsed,
        vec![],
    );
    push(
        &mut state,
        Row::Assistant {
            text: "Done. Every tool row now carries a duration, and 214 tests pass.".into(),
        },
        None,
        RowFold::Collapsed,
        vec![],
    );
    assert_renders("100-help.txt", 100, &state);
}

#[test]
fn frame_80_streaming_renders_at_80_columns() {
    let mut state = base_state();
    state.activity = ActivityState::Running;
    state.turn_millis = Some(12_000);
    state
        .draft
        .set_text("also pin the carry case from the ladder");
    push(
        &mut state,
        Row::User {
            text: "now bind the session clock to the header slot".into(),
        },
        None,
        RowFold::Collapsed,
        vec![],
    );
    push(
        &mut state,
        Row::Thinking {
            text: "reasoning".into(),
        },
        Some(1_800),
        RowFold::Collapsed,
        vec![],
    );
    push(&mut state, Row::Assistant { text: "The header reserves a seven column slot at its right edge. I will drive it from the tick count the state carries, so a test asserts an exact frame with no timer. The renderer stays a pure function of the".into() }, None, RowFold::Collapsed, vec![]);
    assert_renders("80-streaming.txt", 80, &state);
}

#[test]
fn frame_40_streaming_renders_at_40_columns() {
    let mut state = base_state();
    state.activity = ActivityState::Running;
    state.turn_millis = Some(12_000);
    state.draft.set_text("also pin the carry case");
    push(
        &mut state,
        Row::User {
            text: "now bind the session clock to the header slot".into(),
        },
        None,
        RowFold::Collapsed,
        vec![],
    );
    push(
        &mut state,
        Row::Thinking {
            text: "reasoning".into(),
        },
        Some(1_800),
        RowFold::Collapsed,
        vec![],
    );
    push(&mut state, Row::Assistant { text: "The header reserves a seven column slot at its right edge. I will drive it from the tick count the state carries, so a test asserts an exact frame with no timer. The renderer stays a pure function of the".into() }, None, RowFold::Collapsed, vec![]);
    assert_renders("40-streaming.txt", 40, &state);
}

// ---------------------------------------------------------------------------
// Height degradation. The transcript is anchored to the newest row, above the
// composer. As the screen shrinks the transcript loses rows first, then below the
// startup minimum the screen draws what fits in the section 7 order: the composer's
// draft row, then the footer, then the transcript. These tests pin that.
// ---------------------------------------------------------------------------

/// A state with one visible tool row, whose payload is easy to find.
fn one_tool_state() -> TuiState {
    let mut state = base_state();
    push(
        &mut state,
        tool("c1", "bash", "cargo build --release", ToolRowStatus::Ok),
        Some(300),
        RowFold::Collapsed,
        vec![],
    );
    state
}

/// Render `state` at an exact height, one string per row.
fn render_at(state: &TuiState, width: u16, height: u16) -> Vec<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("build test terminal");
    terminal
        .draw(|frame| render(state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn the_transcript_survives_a_six_row_frame() {
    // At 60 by 6 the transcript keeps one row, so the tool row is visible.
    let rows = render_at(&one_tool_state(), 60, 6);
    assert!(
        rows.iter()
            .any(|line| line.contains("cargo build --release")),
        "the tool row was not visible in a six-row frame:\n{}",
        rows.join("\n")
    );
}

#[test]
fn the_transcript_survives_a_three_row_frame() {
    // At 60 by 3 the screen is too small for the rules, and it draws the draft row, the
    // footer, and one transcript row. The transcript still shows content.
    let rows = render_at(&one_tool_state(), 60, 3);
    assert!(
        rows.iter()
            .any(|line| line.contains("cargo build --release")),
        "the transcript was not visible in a three-row frame:\n{}",
        rows.join("\n")
    );
}

#[test]
fn a_one_row_frame_shows_the_composer_draft_row() {
    // At 60 by 1 the section 7 order keeps the composer's draft row first, and nothing
    // else. This replaces the inline test that showed the transcript, because the
    // priority order changed with the full screen. See the report.
    let rows = render_at(&one_tool_state(), 60, 1);
    let line = &rows[0];
    assert!(
        line.contains("Type a prompt"),
        "the one-row frame must show the composer draft row: {line:?}"
    );
    assert!(
        !line.contains('─'),
        "a rule leaked into the one-row frame: {line:?}"
    );
    // The footer carries an activity word and right-aligned hints. Checking for the word
    // "help" cannot work here, because the composer placeholder itself ends with "? for
    // help.". So this looks for the footer's own marks instead.
    assert!(
        !line.contains("ready") && !line.contains("enter send"),
        "the footer leaked into the one-row frame: {line:?}"
    );
}
