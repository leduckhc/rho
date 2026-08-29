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
use rho_tui::{DURATION_SLOT_COLUMNS, Row, ToolRowStatus, TuiState, render, sanitize_line};
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
    // The row is justified to the measure, so the rail's column follows the slot. See
    // `a_task_row_keeps_its_whole_duration_beside_the_scroll_rail`.
    assert!(
        row.trim_end().ends_with("1m 12s"),
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
    // The arithmetic gives the boundary, and the boundary is the rule. `task ` and ` running`
    // cost 13 columns. The rail, the duration slot, and the gap beside it cost 9. The
    // separator and the smallest useful progress cost 7, and the command keeps 8. So the
    // progress leaves the row below 37 columns, and it comes back at 37.
    for width in [20u16, 24, 28, 32, 36] {
        let row = task_row(&state, width);
        assert!(
            !row.contains('\u{b7}'),
            "no separator survives at {width}: {row:?}"
        );
        assert!(
            !row.contains("42%"),
            "the progress is dropped whole at {width}: {row:?}"
        );
        // The row never grows an empty gap. Where the command goes, the space that carried it
        // goes with it.
        assert!(
            !row.trim_end().contains("  "),
            "no double space anywhere at {width}: {row:?}"
        );
        if width >= 24 {
            // The state word draws whole, and the command keeps whatever is left.
            assert!(
                row.contains("running"),
                "the state word survives at {width}: {row:?}"
            );
            assert!(
                row.starts_with("task c"),
                "the command head draws at {width}: {row:?}"
            );
        } else {
            // At twenty columns the rail and the slot take nine, so even the state is cut.
            // Its head still draws, because the status is what the reader needs first.
            assert_eq!(
                row.trim_end(),
                "task runni\u{2026}",
                "the state head alone at {width}: {row:?}"
            );
        }
    }
    for width in [37u16, 40, 80] {
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
    // The exact kept width, not an upper bound. A review showed that `<= 64` let the constant
    // shrink to seven columns with the whole suite still green.
    assert_eq!(
        stored.width(),
        64,
        "the bound keeps exactly its columns: {stored:?}"
    );
    assert!(
        stored.starts_with("7% 1/9"),
        "the bound keeps the head, which carries the numbers: {:?}",
        &stored[..stored.len().min(40)]
    );

    // A column bound is not a character bound: a combining mark is zero columns wide, so a
    // hostile task could store many characters inside 64 columns. The character bound comes
    // from `rho_redact::MAX_LINE_CHARS`, which is 4096, and this pins it so a change there
    // cannot make a row hold a megabyte in silence.
    let mut wide = started("build");
    progressed(
        &mut wide,
        TaskProgress {
            percent: None,
            message: Some("a\u{301}".repeat(100_000)),
            done: None,
            total: None,
        },
    );
    let dense = wide
        .rows
        .iter()
        .find_map(|row| match row {
            Row::Task { progress, .. } => Some(progress.clone()),
            _ => None,
        })
        .expect("a task row");
    assert!(
        dense.chars().count() <= 4_100,
        "the stored summary keeps a character bound too, and it held {} characters",
        dense.chars().count()
    );
}

// ---- The guard for the class. ---------------------------------------------

#[test]
fn the_task_row_pattern_names_every_field() {
    // The compiler catches a **new** field, because the pattern has no `..`. It cannot
    // catch a contributor who puts `..` back, because `..` compiles. So this test reads
    // the renderer and states the rule where a contributor meets it. See
    // `D-a-row-pattern-names-every-field`.
    //
    // **The field list is read from the enum, not written here.** A review found the hard
    // coded list worthless against the case that matters: a field added to `Row::Task`
    // tomorrow would not be in a list written today, so the test would pass while the row
    // ignored it. The compiler forces the new field to be **named**; this test forces it to
    // be named in a pattern with no `..`; only a reviewer can decide it should be drawn.
    let render_source =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/render.rs"))
            .expect("read the renderer source");
    let state_source =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/state.rs"))
            .expect("read the state source");
    // A comment is not code. The pattern is read with every comment line removed, so a `..`
    // inside a note about `..` neither trips the guard nor satisfies it.
    let code: String = render_source
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

    // The fields of the `Task` variant, taken from its declaration.
    let declaration = state_source
        .split("    Task {")
        .nth(1)
        .expect("the Task variant is declared");
    let body = declaration
        .split("\n    },")
        .next()
        .expect("the variant body ends");
    let fields: Vec<String> = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("//") && line.ends_with(','))
        .filter_map(|line| line.split(':').next().map(str::to_string))
        .filter(|name| !name.is_empty())
        .collect();
    assert!(
        fields.len() >= 6,
        "the field list must come from the enum, and it read {fields:?}"
    );
    for field in fields {
        assert!(
            pattern.contains(&field),
            "the renderer pattern must name {field}, and it reads {pattern:?}"
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

// ---- What the review found. ------------------------------------------------

/// The column the scroll rail takes, from `RAIL_COLUMN` in the renderer. The transcript
/// measure reserves it always, because reserving it only while the rail draws would make the
/// measure depend on the overflow it decides.
const RAIL: usize = 1;

/// The first column of the duration slot, at a given frame width.
fn slot_start(width: u16) -> usize {
    usize::from(width) - RAIL - DURATION_SLOT_COLUMNS
}

/// The last column any text reached on the first task row.
fn last_text_column(state: &TuiState, width: u16) -> usize {
    let drawn = cells(state, width);
    let line = drawn
        .iter()
        .find(|line| line.concat().contains("task "))
        .expect("the task row draws");
    line.iter().rposition(|cell| cell != " ").unwrap_or(0)
}

#[test]
fn a_task_row_keeps_its_whole_duration_beside_the_scroll_rail() {
    // Found by review, then measured. The row justified its duration to the frame width, and
    // the rail draws over the last column of the transcript whenever it overflows. So a
    // settled duration read `1m 12│`. The transcript measure exists for exactly this, and
    // the rail is why it reserves a column always. See `RAIL_COLUMN`.
    let mut state = TuiState::default();
    for index in 0..40 {
        state.rows.push(Row::Task {
            id: format!("t{index}"),
            command: "build".to_string(),
            state: "done".to_string(),
            finished: true,
            failed: false,
            progress: String::new(),
        });
        state.row_durations.push(Some(72_000));
    }
    let rows = screen(&state, 60);
    let drawn: Vec<&String> = rows.iter().filter(|row| row.contains("task ")).collect();
    assert!(drawn.len() > 1, "the transcript must overflow: {rows:?}");
    for row in drawn {
        assert!(
            row.contains("1m 12s"),
            "the rail must not take the last column of the duration: {row:?}"
        );
    }
}

#[test]
fn a_tool_row_keeps_its_whole_duration_beside_the_scroll_rail() {
    // The sibling row had the same defect, in the same one word, and it was live rather than
    // latent: a tool row settles a real duration today. Both rows now justify to the measure,
    // so both keep the guard. The design fixtures `100-idle.txt` and `100-tool-run.txt` moved
    // one column with the fix.
    let mut state = TuiState::default();
    for index in 0..40 {
        state.rows.push(Row::Tool {
            id: format!("x{index}"),
            name: "read".to_string(),
            kind: rho_core::ToolKind::Read,
            status: ToolRowStatus::Ok,
            preview: "src/render.rs".to_string(),
        });
        state.row_durations.push(Some(72_000));
    }
    let rows = screen(&state, 60);
    // `ready` in the footer holds `read`, so the tool row is matched by its status glyph.
    let drawn: Vec<&String> = rows
        .iter()
        .filter(|row| row.contains("\u{2713} read"))
        .collect();
    assert!(drawn.len() > 1, "the transcript must overflow: {rows:?}");
    for row in drawn {
        assert!(
            row.contains("1m 12s"),
            "the rail must not take the last column of the duration: {row:?}"
        );
    }
}

#[test]
fn a_hostile_state_word_cannot_reach_the_duration_slot() {
    // Found by review. `Row` is public, so the state word is untrusted too, and the row
    // counted its width without ever cutting it. A long state took the reserved slot, which
    // is the one thing this row exists to protect.
    for state_word in [
        "x".repeat(500),
        "killed (SIGKILL)".to_string(),
        "timed out".to_string(),
    ] {
        for width in [24u16, 27, 40, 80] {
            let mut state = TuiState::default();
            state.rows = vec![Row::Task {
                id: "t1".to_string(),
                command: "build".to_string(),
                state: state_word.clone(),
                finished: false,
                failed: false,
                progress: "42%".to_string(),
            }];
            let last = last_text_column(&state, width);
            assert!(
                last < slot_start(width),
                "the state word reached column {last} of {width}, and the slot starts at {}",
                slot_start(width)
            );
        }
    }
}

#[test]
fn a_task_row_filters_a_command_a_frontend_built() {
    // Found by review. Every earlier test took its command through the reducer, which
    // sanitises, so the row's own filter on the command was never exercised. `Row` is public,
    // and a model writes the command.
    let hostile = "build\u{1b}[2J\u{1b}]0;pwned\u{7} --release";
    let mut raw = TuiState::default();
    raw.rows = vec![Row::Task {
        id: "t1".to_string(),
        command: hostile.to_string(),
        state: "running".to_string(),
        finished: false,
        failed: false,
        progress: String::new(),
    }];
    let mut oracle = TuiState::default();
    oracle.rows = vec![Row::Task {
        id: "t1".to_string(),
        command: sanitize_line(hostile),
        state: "running".to_string(),
        finished: false,
        failed: false,
        progress: String::new(),
    }];
    for width in [40u16, 80] {
        assert_eq!(
            cells(&raw, width),
            cells(&oracle, width),
            "the row filters its own command at {width}"
        );
    }
}

#[test]
fn a_task_row_drops_a_command_it_can_only_draw_as_an_ellipsis() {
    // Found by review. A one-column command draws as a lone `…`, which is the bare ellipsis
    // the decision refuses for the progress. The same rule holds for the command.
    let mut state = TuiState::default();
    state.rows = vec![Row::Task {
        id: "t1".to_string(),
        command: "cargo build --release".to_string(),
        state: "running".to_string(),
        finished: false,
        failed: false,
        progress: String::new(),
    }];
    for width in [21u16, 22, 23] {
        let row = task_row(&state, width);
        assert_eq!(
            row.trim_end(),
            "task running",
            "a command with no room goes whole at {width}: {row:?}"
        );
    }
}

#[test]
fn a_command_that_filters_to_nothing_leaves_no_gap() {
    // A command of escape bytes alone filters to an empty string, so the row draws no command
    // at all. The space that carried it must go with it. Found by a surviving mutation: the
    // `cut.is_empty()` branch had no test, because every other case has a visible command.
    let mut state = TuiState::default();
    state.rows = vec![Row::Task {
        id: "t1".to_string(),
        command: "\u{1b}[2J\u{1b}[H".to_string(),
        state: "running".to_string(),
        finished: false,
        failed: false,
        progress: "42%".to_string(),
    }];
    for width in [40u16, 80] {
        let row = task_row(&state, width);
        assert_eq!(
            row.trim_end(),
            "task running \u{b7} 42%",
            "no gap where the command was at {width}: {row:?}"
        );
    }
}

#[test]
fn the_command_takes_at_most_half_the_row_beside_a_progress() {
    // Found by review. Nothing pinned the half share, so the divisor was free: a change from
    // two to three passed the whole suite while every row changed.
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
    for width in [80u16, 100, 200] {
        let row = task_row(&state, width);
        // The text columns are the frame less the rail, the slot, and the gap beside it.
        let text_columns = usize::from(width) - RAIL - DURATION_SLOT_COLUMNS - 1;
        let share = text_columns / 2;
        // Measured in columns, never in bytes: the cut marker is one column and three bytes.
        let head = row
            .split(" running")
            .next()
            .expect("the state word draws after the command");
        let command_columns = head.width() - "task ".len();
        assert_eq!(
            command_columns, share,
            "the command takes half the text columns at {width}: {row:?}"
        );
    }
}

#[test]
fn a_random_task_row_never_leaves_its_bounds() {
    // The invariant over generated input, not over nine strings somebody thought of. The
    // generator is a small xorshift, seeded fixed, so a failure is reproducible and the test
    // needs no new dependency.
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    // The bytes a hostile field is built from: escapes, controls, bidi marks, wide glyphs,
    // and ordinary text.
    let alphabet: Vec<char> = "aZ9 %/\u{1b}[]0;\u{7}\u{8}\u{202e}\u{200b}\u{65e5}\u{fffd}\t\n"
        .chars()
        .collect();
    let mut pick = move |bound: usize| (next() % bound as u64) as usize;
    for _ in 0..2_000 {
        let field = |length: usize, pick: &mut dyn FnMut(usize) -> usize| -> String {
            (0..length)
                .map(|_| alphabet[pick(alphabet.len())])
                .collect()
        };
        let command = field(pick(120), &mut pick);
        let state_word = field(pick(40), &mut pick);
        let progress = field(pick(200), &mut pick);
        let width = (20 + pick(180)) as u16;
        let mut state = TuiState::default();
        state.rows = vec![Row::Task {
            id: "t1".to_string(),
            command: command.clone(),
            state: state_word.clone(),
            finished: false,
            failed: false,
            progress: progress.clone(),
        }];
        let drawn = cells(&state, width);
        let line = drawn
            .iter()
            .find(|line| line.concat().contains("task "))
            .expect("the task row draws");
        let last = line.iter().rposition(|cell| cell != " ").unwrap_or(0);
        assert!(
            last < slot_start(width),
            "text reached column {last} of {width}, and the slot starts at {}. \
             command {command:?} state {state_word:?} progress {progress:?}",
            slot_start(width)
        );
        // The row filters every field itself, whatever a frontend handed it.
        let mut oracle = TuiState::default();
        oracle.rows = vec![Row::Task {
            id: "t1".to_string(),
            command: sanitize_line(&command),
            state: sanitize_line(&state_word),
            finished: false,
            failed: false,
            progress: sanitize_line(&progress),
        }];
        assert_eq!(
            drawn,
            cells(&oracle, width),
            "the row filters every field at {width}. command {command:?} \
             state {state_word:?} progress {progress:?}"
        );
    }
}

#[test]
fn a_row_drops_a_duration_it_cannot_draw_whole() {
    // A second review pass found this. At a pathological width the text and the slot together
    // exceed the row, and `pad` cuts the tail, so `1m 12s` drew as `1m 12`. That reads as a
    // different span, which is worse than no span at all. A duration is drawn whole or it is
    // dropped, which is the rule the progress cell already follows.
    let mut task = TuiState::default();
    task.rows = vec![Row::Task {
        id: "t1".to_string(),
        command: "build".to_string(),
        state: "running".to_string(),
        finished: false,
        failed: false,
        progress: String::new(),
    }];
    task.row_durations.push(Some(72_000));

    let mut tool = TuiState::default();
    tool.rows = vec![Row::Tool {
        id: "x1".to_string(),
        name: "read".to_string(),
        kind: rho_core::ToolKind::Read,
        status: ToolRowStatus::Ok,
        preview: "x".to_string(),
    }];
    tool.row_durations.push(Some(72_000));

    for width in 10u16..=30 {
        for (label, state) in [("task", &task), ("tool", &tool)] {
            let rows = screen(state, width);
            let row = rows
                .iter()
                .find(|row| row.contains("task ") || row.contains('\u{2713}'))
                .cloned()
                .unwrap_or_default();
            // Either the whole duration draws, or none of it does. A fragment is a wrong span.
            let whole = row.contains("1m 12s");
            let fragment = row.contains("1m 1") && !whole;
            assert!(
                !fragment,
                "{label} row at {width} drew a cut duration: {row:?}"
            );
        }
    }
    // At a comfortable width the duration is there, so the rule above is not vacuous.
    assert!(
        task_row(&task, 40).contains("1m 12s"),
        "the duration draws at 40 columns"
    );
}
