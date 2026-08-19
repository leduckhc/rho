//! A table is drawn as a table, because a model emits them constantly.
//!
//! rho drew a markdown table as its raw pipes, which is what the owner sees most often in a
//! model's answer. Both pi and jcode render one: pi with box borders, jcode with aligned
//! columns, a bold header, and a divider. rho follows jcode, which is lighter and matches the
//! rest of the interface.
//!
//! A table is the one construct here that spans several lines, so it is recognised in
//! `scan_markdown`, which already reads a whole message. See `SPEC-tui-markdown` section 3b.
//!
//! **A malformed table stays verbatim.** Half a table drawn is worse than none, and the
//! contract review said so before any of this existed.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;
use rho_tui::{MarkdownKind, Row, TuiState, render, scan_markdown};

fn kinds(text: &str) -> Vec<MarkdownKind> {
    scan_markdown(text)
        .into_iter()
        .map(|line| line.kind)
        .collect()
}

fn texts(text: &str) -> Vec<String> {
    scan_markdown(text)
        .into_iter()
        .map(|line| line.text)
        .collect()
}

/// The display column of the first `needle` in `line`, counting terminal columns not bytes.
fn column_of_char(line: &str, needle: char) -> Option<usize> {
    let mut column = 0usize;
    for ch in line.chars() {
        if ch == needle {
            return Some(column);
        }
        column += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    None
}

const TABLE: &str =
    "| crate | role |\n|-------|------|\n| rho-tui | the interface |\n| rho-core | the loop |";

// ---- Recognition. ---------------------------------------------------------------

#[test]
fn a_table_is_recognised_as_a_table() {
    assert_eq!(
        kinds(TABLE),
        vec![
            MarkdownKind::TableHead,
            MarkdownKind::TableRule,
            MarkdownKind::TableRow,
            MarkdownKind::TableRow,
        ]
    );
}

#[test]
fn the_pipes_are_replaced_by_a_drawn_column() {
    let rows = texts(TABLE);
    assert!(
        !rows[0].contains('|'),
        "the header draws no raw pipe: {:?}",
        rows[0]
    );
    assert!(
        rows[0].contains('\u{2502}'),
        "it draws a column divider instead: {:?}",
        rows[0]
    );
    assert!(
        rows[1]
            .chars()
            .all(|ch| ch == '\u{2500}' || ch == '\u{253c}' || ch == ' '),
        "the rule row is only rule glyphs: {:?}",
        rows[1]
    );
}

#[test]
fn the_columns_align_across_every_row() {
    let rows = texts(TABLE);
    // The divider sits at the same DISPLAY column on every row of the table.
    //
    // This first used `str::find`, which returns a byte offset. A rule glyph is three bytes, so
    // the rule row reported 27 where the header reported 9 and the test failed against correct
    // code. Alignment is a display property, so it is measured in display columns.
    let column_of = |line: &str| {
        let mut column = 0usize;
        for ch in line.chars() {
            if ch == '\u{2502}' || ch == '\u{253c}' {
                return Some(column);
            }
            column += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        }
        None
    };
    let first = column_of(&rows[0]).expect("the header has a divider");
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(
            column_of(row),
            Some(first),
            "row {index} must align: {row:?}"
        );
    }
}

#[test]
fn a_cell_keeps_its_text() {
    let joined = texts(TABLE).join("\n");
    for cell in [
        "crate",
        "role",
        "rho-tui",
        "the interface",
        "rho-core",
        "the loop",
    ] {
        assert!(joined.contains(cell), "cell {cell:?} survives:\n{joined}");
    }
}

#[test]
fn a_table_without_a_rule_row_stays_verbatim() {
    // Two pipe lines and no alignment row is not a table. Drawing half of one is worse than
    // drawing none, so it stays exactly as the model wrote it.
    let not_a_table = "| a | b |\n| c | d |";
    assert_eq!(
        kinds(not_a_table),
        vec![MarkdownKind::Text, MarkdownKind::Text]
    );
    assert_eq!(
        texts(not_a_table),
        not_a_table.split('\n').collect::<Vec<_>>()
    );
}

#[test]
fn a_table_inside_a_fence_stays_code() {
    let fenced = "```\n| a | b |\n|---|---|\n| c | d |\n```";
    assert_eq!(
        kinds(fenced),
        vec![
            MarkdownKind::Fence,
            MarkdownKind::CodeBlock,
            MarkdownKind::CodeBlock,
            MarkdownKind::CodeBlock,
            MarkdownKind::Fence,
        ]
    );
}

#[test]
fn a_lone_pipe_line_is_not_a_table() {
    assert_eq!(kinds("| just one line |"), vec![MarkdownKind::Text]);
    assert_eq!(kinds("a | b in prose"), vec![MarkdownKind::Text]);
}

#[test]
fn a_table_with_no_outer_pipes_still_works() {
    // A model often omits the leading and trailing pipe.
    let bare = "crate | role\n------|-----\nrho-tui | interface";
    assert_eq!(
        kinds(bare),
        vec![
            MarkdownKind::TableHead,
            MarkdownKind::TableRule,
            MarkdownKind::TableRow,
        ]
    );
}

#[test]
fn a_ragged_row_is_padded_not_dropped() {
    // A row with fewer cells than the header must still draw, or data disappears.
    let ragged = "| a | b | c |\n|---|---|---|\n| 1 | 2 |";
    let rows = texts(ragged);
    assert_eq!(rows.len(), 3, "every row draws");
    assert!(
        rows[2].contains('1') && rows[2].contains('2'),
        "cells survive"
    );
}

#[test]
fn an_alignment_marker_places_the_text() {
    // `---:` is right aligned and `:---:` is centred. A number column reads wrong left aligned.
    //
    // This test first asserted only that nothing followed the number, which `trim_end` makes
    // true for a left aligned cell too, so it passed against a build that ignored alignment
    // entirely. It now compares display columns: a right aligned `7` must sit well past the
    // divider, where a left aligned one would sit right after it.
    let aligned = "| name | count |\n|:-----|------:|\n| a | 7 |";
    let rows = texts(aligned);
    let last = rows.last().expect("the body row");
    let divider = column_of_char(last, '\u{2502}').expect("the divider draws");
    let number = column_of_char(last, '7').expect("the number draws");
    assert!(
        number > divider + 2,
        "a right aligned cell sits at the far edge of its column: divider at {divider}, \
         number at {number}, row {last:?}"
    );

    // And a left aligned column puts its text right after the divider.
    let left = "| name | count |\n|:-----|:------|\n| a | 7 |";
    let left_rows = texts(left);
    let left_last = left_rows.last().expect("the body row");
    let left_divider = column_of_char(left_last, '\u{2502}').expect("divider");
    let left_number = column_of_char(left_last, '7').expect("number");
    assert_eq!(
        left_number,
        left_divider + 2,
        "a left aligned cell starts one pad after the divider: {left_last:?}"
    );
}

#[test]
fn a_cell_with_inline_markup_stays_aligned() {
    // A model writes `**bold**` and `` `code` `` inside a table cell constantly. The markers
    // must come off, and the columns must still line up. Measuring a cell with its markers still
    // in it would shift every column to the right of it.
    //
    // The cost is stated: emphasis inside a cell is dropped rather than styled, because a row
    // here is one string and cannot carry runs.
    let table =
        "| field | note |\n|-------|------|\n| `id` | the **primary** key |\n| name | plain |";
    let rows = texts(&table.replace("\\n", "\n"));
    let joined = rows.join("\n");
    assert!(
        !joined.contains('*'),
        "no emphasis marker survives:\n{joined}"
    );
    assert!(!joined.contains('`'), "no backtick survives:\n{joined}");
    assert!(
        joined.contains("primary"),
        "the cell text survives:\n{joined}"
    );
    assert!(joined.contains("id"), "and the code cell:\n{joined}");

    let columns: Vec<Option<usize>> = rows
        .iter()
        .map(|row| column_of_char(row, '\u{2502}').or_else(|| column_of_char(row, '\u{253c}')))
        .collect();
    let first = columns[0].expect("the header has a divider");
    for (index, column) in columns.iter().enumerate() {
        assert_eq!(
            *column,
            Some(first),
            "row {index} must align once the markers are gone: {:?}",
            rows[index]
        );
    }
}

#[test]
fn a_table_wider_than_the_screen_is_cut_not_wrapped() {
    // A wide table must not wrap into an unreadable stack. Each row is cut to the measure.
    let wide = format!(
        "| {} | {} |\n|---|---|\n| {} | {} |",
        "a".repeat(60),
        "b".repeat(60),
        "c".repeat(60),
        "d".repeat(60)
    );
    let mut state = TuiState::default();
    state.rows.push(Row::Assistant { text: wide });
    let backend = TestBackend::new(60, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render(&state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    for y in 0..20u16 {
        let line: String = (0..60).map(|x| buffer[(x, y)].symbol()).collect();
        assert!(
            line.chars().count() <= 60,
            "row {y} stays inside the screen"
        );
    }
}

// ---- What the user sees. --------------------------------------------------------

#[test]
fn a_header_draws_bold_and_the_rule_draws_muted() {
    let mut state = TuiState::default();
    state.rows.push(Row::Assistant {
        text: TABLE.to_string(),
    });
    let backend = TestBackend::new(70, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render(&state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    let mut header = None;
    let mut rule = None;
    let mut body = None;
    for y in 0..20u16 {
        let line: String = (0..70).map(|x| buffer[(x, y)].symbol()).collect();
        let first = (0..70).find(|x| buffer[(*x, y)].symbol() != " ");
        let Some(x) = first else { continue };
        if line.contains("crate") {
            header = Some(buffer[(x, y)].style());
        } else if line.contains('\u{253c}') {
            rule = Some(buffer[(x, y)].style());
        } else if line.contains("rho-tui") {
            body = Some(buffer[(x, y)].style());
        }
    }
    let header = header.expect("the header draws");
    let rule = rule.expect("the rule draws");
    let body = body.expect("a body row draws");
    assert!(
        header.add_modifier.contains(Modifier::BOLD),
        "the header is bold"
    );
    assert!(
        !body.add_modifier.contains(Modifier::BOLD),
        "a body row is not"
    );
    assert_ne!(rule.fg, body.fg, "the rule is quieter than the data");
}

// ---- Findings from a second-opinion review. -------------------------------------

#[test]
fn a_row_with_more_cells_than_the_header_stays_verbatim() {
    // Found by a second-opinion review. `| a | b |` with a body row of three cells drew as a two
    // column table and **silently dropped the third cell**. GitHub's markdown drops it too, and this
    // project's own rule is the stronger one: a ragged row is padded, never dropped, or data
    // disappears. So an over-wide row means the block is not an unambiguous table, and it stays
    // verbatim, which loses nothing.
    let ragged = "| a | b |\n|---|---|\n| one | two | LOST |";
    let scanned = scan_markdown(ragged);
    for line in &scanned {
        assert_eq!(
            line.kind,
            MarkdownKind::Text,
            "an over-wide row makes the block verbatim: {line:?}"
        );
    }
    let joined: String = scanned
        .iter()
        .map(|line| line.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("LOST"), "no cell is lost:\n{joined}");
}

#[test]
fn a_row_with_fewer_cells_is_still_padded_and_drawn() {
    // The other direction is unchanged: a short row is padded, because nothing is lost by padding.
    let short = "| a | b | c |\n|---|---|---|\n| 1 | 2 |";
    let kinds: Vec<MarkdownKind> = scan_markdown(short).into_iter().map(|l| l.kind).collect();
    assert_eq!(
        kinds,
        vec![
            MarkdownKind::TableHead,
            MarkdownKind::TableRule,
            MarkdownKind::TableRow
        ],
        "a short row still draws"
    );
}
