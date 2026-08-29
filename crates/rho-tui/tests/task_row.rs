//! The task row draws the progress the reducer already computes.
//!
//! The renderer matched `Row::Task { command, state: task, .. }`, and the `..` was the whole
//! defect. The reducer stored a percent, and the row threw it away, so a four minute build
//! said `running` and nothing else. `bench/check-dead-surface.py` cannot see this shape,
//! because it finds an uncalled function and this is a field a reader never reads.
//!
//! The progress text is untrusted, because a task prints what it likes. So these tests
//! assert a bound and not an example: any progress string, of any length and any bytes,
//! leaves the row inside the width, with no control character, and with the seven-column
//! duration slot intact.
//!
//! See `SPEC-the-task-row-draws-its-progress`,
//! `D-progress-follows-the-state-and-never-moves-it`, and `D-a-row-pattern-names-every-field`.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use rho_core::{AgentEvent, BackgroundReason, TaskId, TaskProgress, TaskState};
use rho_tui::{DURATION_SLOT_COLUMNS, Row, TuiState, render, sanitize_line};
use unicode_width::UnicodeWidthStr;

/// The error role's 256-colour value, from `docs/tui-design.md` section 3.
const ERROR_COLOUR: Color = Color::Indexed(203);

/// A tall frame, so the transcript never overflows and the scroll rail never draws.
const HEIGHT: u16 = 20;

fn task_id(text: &str) -> TaskId {
    TaskId(text.to_string())
}

/// A state holding one running task row.
fn started(command: &str) -> TuiState {
    let mut state = TuiState::default();
    state.apply(
        &AgentEvent::TaskStart {
            id: task_id("t1"),
            command: command.to_string(),
            reason: BackgroundReason::KnownLongRunning,
        },
        0,
    );
    state
}

/// Report progress on task `t1`.
fn progressed(state: &mut TuiState, progress: TaskProgress) {
    state.apply(
        &AgentEvent::TaskProgressed {
            id: task_id("t1"),
            progress,
        },
        0,
    );
}

/// A state whose task row was built directly, with no reducer between.
///
/// `Row` is public, so another frontend builds a task row itself and the reducer's sanitiser
/// never runs. The row filters at its own boundary, and this is how a test can see it. A
/// review found the tool row trusting the reducer in exactly this way.
fn raw_row(progress: &str) -> TuiState {
    let mut state = TuiState::default();
    state.rows = vec![Row::Task {
        id: "t1".to_string(),
        command: "build".to_string(),
        state: "running".to_string(),
        finished: false,
        failed: false,
        progress: progress.to_string(),
    }];
    state
}

/// Every drawn row, as its cell symbols. One entry is one terminal column, so a wide glyph
/// is one entry and its continuation column is another. Counting cells is the only honest
/// measure here: `ratatui` blanks the continuation column, so joining the symbols and
/// measuring the display width counts a wide glyph three columns instead of two.
fn cells(state: &TuiState, width: u16) -> Vec<Vec<String>> {
    let backend = TestBackend::new(width, HEIGHT);
    let mut terminal = Terminal::new(backend).expect("build test terminal");
    terminal
        .draw(|frame| render(state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    (0..HEIGHT)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect()
        })
        .collect()
}

/// Every drawn row, as text.
fn screen(state: &TuiState, width: u16) -> Vec<String> {
    cells(state, width)
        .into_iter()
        .map(|row| row.concat())
        .collect()
}

/// The one drawn row that carries the task. A task row is one line, so two would be a defect.
fn task_row(state: &TuiState, width: u16) -> String {
    let rows = screen(state, width);
    let found: Vec<&String> = rows.iter().filter(|row| row.contains("task ")).collect();
    assert_eq!(
        found.len(),
        1,
        "a task row is exactly one line; the screen was:\n{}",
        rows.join("\n")
    );
    found[0].clone()
}

/// The foreground colour of the first cell of the task row.
fn task_row_colour(state: &TuiState, width: u16) -> Option<Color> {
    let backend = TestBackend::new(width, HEIGHT);
    let mut terminal = Terminal::new(backend).expect("build test terminal");
    terminal
        .draw(|frame| render(state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    for y in 0..HEIGHT {
        let text: String = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
        if text.contains("task ") {
            return buffer[(0, y)].style().fg;
        }
    }
    None
}

// ---- The defect itself. ----------------------------------------------------

#[test]
fn a_task_row_draws_the_progress_the_reducer_stored() {
    let mut state = started("cargo build --release");
    progressed(
        &mut state,
        TaskProgress {
            percent: Some(42),
            message: Some("compiling".to_string()),
            done: Some(6),
            total: Some(10),
        },
    );
    let row = task_row(&state, 100);
    assert!(row.contains("42%"), "the percent must draw: {row:?}");
    assert!(row.contains("6/10"), "the counts must draw: {row:?}");
    assert!(row.contains("compiling"), "the message must draw: {row:?}");
    assert!(
        row.contains('\u{b7}'),
        "a separator marks the progress cell: {row:?}"
    );
    // The order is the decision: the state word first, then the progress. A value that
    // changes must not move a value that does not.
    let state_at = row.find("running").expect("the state word draws");
    let progress_at = row.find("42%").expect("the progress draws");
    assert!(
        state_at < progress_at,
        "the progress follows the state word: {row:?}"
    );
}

#[test]
fn a_task_row_with_no_progress_draws_no_separator() {
    // A task with no progress must not grow an empty gap or a stray separator.
    let state = started("cargo test");
    let row = task_row(&state, 100);
    assert_eq!(
        row.trim_end(),
        "task cargo test running",
        "an empty progress leaves the row exactly as it was: {row:?}"
    );
}

#[test]
fn a_task_row_keeps_the_progress_out_of_the_duration_slot() {
    // The row reserves the seven-column slot `F-duration-slot` defines. A hostile progress
    // string must not be able to take it, or the day a span arrives every text on the row
    // would move.
    let mut state = started("build");
    progressed(
        &mut state,
        TaskProgress {
            percent: Some(99),
            message: Some("x".repeat(4000)),
            done: None,
            total: None,
        },
    );
    let index = state
        .rows
        .iter()
        .position(|row| matches!(row, Row::Task { .. }))
        .expect("a task row");
    state.row_durations[index] = Some(72_000);
    let row = task_row(&state, 80);
    assert!(
        row.ends_with(" 1m 12s"),
        "the duration keeps its slot at the right edge: {row:?}"
    );
    assert!(
        row.contains("99%"),
        "the progress still draws beside it: {row:?}"
    );
}

#[test]
fn any_progress_string_stays_inside_the_task_row() {
    // The invariant, not an example. A task prints what it likes, so the row is asserted
    // over lengths and byte sets, and never against one expected string.
    //
    // Three bounds are asserted for every case:
    //
    //   * the drawn text ends before the seven-column duration slot,
    //   * no control character and no escape reaches a terminal cell,
    //   * every other row on the screen is what it was without the progress.
    //
    // The third one is the reflow rule. A row must never move the text beside it.
    let hostile: Vec<String> = vec![
        "42%".to_string(),
        "\u{1b}[2J\u{1b}[Hcleared the screen".to_string(),
        "\u{1b}]0;retitled the window\u{7}".to_string(),
        "\u{7}\u{8}\u{c}\u{0}".repeat(50),
        "\u{202e}reordered".to_string(),
        "line one\nline two\ttabbed".to_string(),
        "\u{65e5}\u{672c}\u{8a9e}".repeat(500),
        "x".repeat(10_000),
        "\u{1b}".repeat(5_000),
    ];
    for progress in &hostile {
        for width in [24u16, 40, 80, 200] {
            // `task build running` is 18 columns, so the head fits at every width here.
            let quiet = started("build");
            let mut state = started("build");
            progressed(
                &mut state,
                TaskProgress {
                    percent: None,
                    message: Some(progress.clone()),
                    done: None,
                    total: None,
                },
            );
            let drawn = cells(&state, width);
            let row = task_row(&state, width);
            for cell in drawn.iter().flatten() {
                assert!(
                    !cell.chars().any(char::is_control),
                    "no control character reaches a cell at {width}: {row:?}"
                );
                assert!(
                    !cell.contains('\u{1b}'),
                    "no escape reaches a cell at {width}: {row:?}"
                );
            }
            // The drawn text ends before the duration slot. At width 24 the head and the
            // slot already spend the row, and the narrow-row test covers that case.
            if width >= 40 {
                let line = drawn
                    .iter()
                    .find(|line| line.concat().contains("task "))
                    .expect("the task row draws");
                let last_used = line.iter().rposition(|cell| cell != " ").unwrap_or(0);
                assert!(
                    last_used < usize::from(width) - DURATION_SLOT_COLUMNS,
                    "the text stops before the duration slot at {width}, and it reached \
                     column {last_used}: {row:?}"
                );
            }
            // The same bounds hold for a row a frontend built itself, with no reducer to
            // sanitise or bound the text first.
            let raw = raw_row(progress);
            let raw_cells = cells(&raw, width);
            let raw_line = task_row(&raw, width);
            for cell in raw_cells.iter().flatten() {
                assert!(
                    !cell.chars().any(char::is_control),
                    "an unsanitised row must not reach a cell at {width}: {raw_line:?}"
                );
                assert!(
                    !cell.contains('\u{1b}'),
                    "no escape from an unsanitised row at {width}: {raw_line:?}"
                );
            }
            if width >= 40 {
                let line = raw_cells
                    .iter()
                    .find(|line| line.concat().contains("task "))
                    .expect("the task row draws");
                let last_used = line.iter().rposition(|cell| cell != " ").unwrap_or(0);
                assert!(
                    last_used < usize::from(width) - DURATION_SLOT_COLUMNS,
                    "an unsanitised row stops before the duration slot at {width}: {raw_line:?}"
                );
            }
            // The row runs the filter itself. Asserting that no cell holds a control byte
            // does **not** prove that: `ratatui` skips a zero-width grapheme, so an escape
            // byte never reaches a cell even with no filter, while the visible payload of
            // the sequence, `[2J`, does. `put`'s own comment names that mistake. So the
            // oracle is the public filter: the row a frontend built raw must draw exactly
            // like the same row built from filtered text.
            let oracle = raw_row(&sanitize_line(progress));
            assert_eq!(
                raw_cells,
                cells(&oracle, width),
                "the row filters its own progress at {width}: {raw_line:?}"
            );

            // No reflow. Every row that is not the task row is unchanged.
            let quiet_rows = screen(&quiet, width);
            let live_rows = screen(&state, width);
            assert_eq!(
                quiet_rows.len(),
                live_rows.len(),
                "the progress must not add a row at {width}"
            );
            for (before, after) in quiet_rows.iter().zip(live_rows.iter()) {
                if before.contains("task ") {
                    continue;
                }
                assert_eq!(
                    before, after,
                    "the progress moved the text beside it at {width}"
                );
            }
        }
    }
}

#[test]
fn a_narrow_task_row_drops_the_progress_whole() {
    // The rank under pressure is the state word, then the command, then the progress. So a
    // row with no room drops the progress and its separator, and it never shows a bare
    // ellipsis in the progress column. The state word always survives.
    let mut state = started("cargo build --release --workspace");
    progressed(
        &mut state,
        TaskProgress {
            percent: Some(42),
            message: None,
            done: None,
            total: None,
        },
    );
    // `task ` and ` running` cost 13 columns, the slot and its gap cost 8, and the separator
    // plus the smallest useful progress cost 7. So the progress leaves the row below 36
    // columns, and it comes back above that. The boundary is the rule, not an example.
    for width in [20u16, 24, 28, 32, 35] {
        let row = task_row(&state, width);
        assert!(
            !row.contains('\u{b7}'),
            "no separator survives at {width}: {row:?}"
        );
        assert!(
            !row.contains("42%"),
            "the progress is dropped whole at {width}: {row:?}"
        );
        assert!(
            row.contains("running"),
            "the state word survives at {width}: {row:?}"
        );
        // The row never grows an empty gap. At 20 columns the command itself goes, and the
        // space that carried it goes with it.
        assert!(
            !row.trim_end().contains("  "),
            "no double space anywhere at {width}: {row:?}"
        );
        if width >= 24 {
            assert!(
                row.starts_with("task \u{2026}") || row.starts_with("task ca"),
                "the command keeps the row, cut to fit, at {width}: {row:?}"
            );
        } else {
            assert_eq!(
                row.trim_end(),
                "task running",
                "the state word alone at {width}: {row:?}"
            );
        }
    }
    for width in [36u16, 40, 80] {
        let row = task_row(&state, width);
        assert!(
            row.contains("42%"),
            "the progress returns at {width}: {row:?}"
        );
        assert!(
            row.contains("running"),
            "the state word survives at {width}: {row:?}"
        );
    }
}

#[test]
fn a_long_command_never_pushes_the_state_or_the_progress_off_the_row() {
    // Found by driving it for real. A model writes the command, and a real one was three
    // hundred characters long: the row drew the command alone, and the state word and the
    // progress were both cut off the right edge.
    let mut state = started(&format!("bash -c {}", "echo hello; ".repeat(40)));
    progressed(
        &mut state,
        TaskProgress {
            percent: Some(88),
            message: Some("linking".to_string()),
            done: None,
            total: None,
        },
    );
    for width in [40u16, 60, 100, 200] {
        let row = task_row(&state, width);
        assert!(
            row.contains("running"),
            "the state word survives a long command at {width}: {row:?}"
        );
        assert!(
            row.contains("88%"),
            "the progress survives a long command at {width}: {row:?}"
        );
        assert!(
            row.contains('\u{2026}'),
            "the command is cut, and the cut is marked at {width}: {row:?}"
        );
    }
    // The command takes at most half the text columns, so the progress stays readable beside
    // a long one. Found by driving it for real: the progress had four columns and read
    // `100\u{2026}`, which says nothing.
    for width in [100u16, 200] {
        let row = task_row(&state, width);
        assert!(
            row.contains("88% linking"),
            "the whole progress draws beside a long command at {width}: {row:?}"
        );
    }
    // At sixty columns half the row is twenty six, so the progress is cut. The numbers lead
    // the summary, so they are what survives the cut.
    let narrow = task_row(&state, 60);
    assert!(
        narrow.contains("88%"),
        "the percent survives the cut at 60: {narrow:?}"
    );
}

// ---- The bound in the data model. -----------------------------------------

#[test]
fn a_stored_task_progress_summary_is_bounded() {
    // `Row::Task.progress` says it is a short summary, and `Row` is public, so another
    // frontend reads the same field. The bound keeps that promise for every reader.
    let mut state = started("build");
    progressed(
        &mut state,
        TaskProgress {
            percent: Some(7),
            message: Some("y".repeat(100_000)),
            done: Some(1),
            total: Some(9),
        },
    );
    let stored = state
        .rows
        .iter()
        .find_map(|row| match row {
            Row::Task { progress, .. } => Some(progress.clone()),
            _ => None,
        })
        .expect("a task row");
    assert!(
        stored.width() <= 64,
        "the stored summary is bounded, and it was {} columns",
        stored.width()
    );
    assert!(
        stored.starts_with("7% 1/9"),
        "the bound keeps the head, which carries the numbers: {:?}",
        &stored[..stored.len().min(40)]
    );
}

// ---- The guard for the class. ---------------------------------------------

#[test]
fn the_task_row_pattern_names_every_field() {
    // The compiler catches a **new** field, because the pattern has no `..`. It cannot
    // catch a contributor who puts `..` back, because `..` compiles. So this test reads
    // the renderer and states the rule where a contributor meets it. See
    // `D-a-row-pattern-names-every-field`.
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/render.rs"))
        .expect("read the renderer source");
    // A comment is not code. The pattern is read with every comment line removed, so a `..`
    // inside a note about `..` neither trips the guard nor satisfies it.
    let code: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<&str>>()
        .join("\n");
    let at = code
        .find("Row::Task {")
        .expect("the renderer draws a task row");
    let rest = &code[at..];
    let end = rest.find("} =>").expect("the pattern ends");
    let pattern = &rest[..end];
    assert!(
        !pattern.contains(".."),
        "a task row pattern names every field, and this one holds `..`: {pattern:?}"
    );
    for field in ["id", "command", "state", "finished", "failed", "progress"] {
        assert!(
            pattern.contains(field),
            "the pattern must name {field}: {pattern:?}"
        );
    }
}

#[test]
fn a_failed_task_row_draws_in_the_error_role() {
    // `Row::Task.failed` says it drives the colour, and no code read it. A field with a
    // promise in its own doc comment and no reader is the same defect as `progress`.
    let mut ok = started("cargo test");
    ok.apply(
        &AgentEvent::TaskEnd {
            id: task_id("t1"),
            state: TaskState::Exited { code: 0 },
            output_tail: String::new(),
        },
        0,
    );
    let mut bad = started("cargo test");
    bad.apply(
        &AgentEvent::TaskEnd {
            id: task_id("t1"),
            state: TaskState::Exited { code: 1 },
            output_tail: String::new(),
        },
        0,
    );
    assert_eq!(
        task_row_colour(&bad, 80),
        Some(ERROR_COLOUR),
        "a failed task row draws in the error role"
    );
    assert_ne!(
        task_row_colour(&ok, 80),
        Some(ERROR_COLOUR),
        "a task that succeeded keeps the plain text style"
    );
}
