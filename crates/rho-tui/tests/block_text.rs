//! Block text keeps its shape. A coding agent answers with lists and code.
//!
//! rho drew every assistant answer as one flowed paragraph. `sanitize_line` folded each
//! newline into a space, and `wrap` then re-split on whitespace, so a markdown list, a
//! paragraph break, and a fenced code block all collapsed into prose. Measured before the
//! fix, with a real model answer on screen:
//!
//! ```text
//! model sent:  Intro paragraph.\n\n- alpha: first\n- beta: second\n\n```rust\nfn main() {}\n```
//! rho drew:    Intro paragraph. - alpha: first - beta: second ```rust fn main() {} ``` Done.
//! ```
//!
//! Mangling a code block is the serious half, because rho is a coding agent.
//!
//! **The escape filter stays.** Only the line folding goes. Model output and tool output
//! are untrusted, and an escape sequence in either can move the cursor, clear the screen,
//! or set the clipboard through OSC 52. `sanitize_text` already keeps `\n` and drops every
//! escape, so the fix is to stop using the single-line wrapper on block text.
//!
//! See `D-block-text-keeps-its-shape`.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rho_tui::{Row, TuiState, render};

/// The drawn rows, trailing spaces removed, blanks kept.
fn drawn(text: &str, width: u16) -> Vec<String> {
    let mut state = TuiState::default();
    state.rows.push(Row::Assistant {
        text: text.to_string(),
    });
    let rows = 30u16;
    let backend = TestBackend::new(width, rows);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render(&state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    (0..rows)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

// ---- The shape survives. --------------------------------------------------------

#[test]
fn an_assistant_answer_keeps_its_line_breaks() {
    // The expected text changed with `SPEC-tui-markdown` phase 1, on purpose: a list marker
    // now draws as a bullet glyph and takes the bullet colour. The intent of this test is
    // unchanged, and the intent is what matters: each item takes its own row instead of being
    // flowed into its neighbour. Only the glyph moved.
    let rows = drawn("- alpha: first\n- beta: second\n- gamma: third", 70);
    assert!(
        rows.iter().any(|r| r.trim() == "• alpha: first"),
        "each list item takes its own row:\n{rows:#?}"
    );
    assert!(
        rows.iter().any(|r| r.trim() == "• beta: second"),
        "the second item is not flowed into the first:\n{rows:#?}"
    );
    assert!(
        rows.iter().any(|r| r.trim() == "• gamma: third"),
        "and the third:\n{rows:#?}"
    );
}

#[test]
fn a_code_fence_keeps_its_own_lines_and_its_indent() {
    // Indentation carries meaning in code. `wrap` used `split_whitespace`, which dropped
    // every leading space, so a nested block came out flat even once newlines survived.
    let source = "```rust\nfn main() {\n    let x = 1;\n}\n```";
    let rows = drawn(source, 70);
    for want in ["```rust", "fn main() {", "    let x = 1;", "}", "```"] {
        assert!(
            rows.iter().any(|r| r.trim_end() == want),
            "the code line {want:?} must draw on its own row, indent intact:\n{rows:#?}"
        );
    }
}

#[test]
fn a_blank_line_between_paragraphs_survives() {
    let rows = drawn("First paragraph.\n\nSecond paragraph.", 70);
    let first = rows
        .iter()
        .position(|r| r.trim() == "First paragraph.")
        .expect("the first paragraph draws");
    let second = rows
        .iter()
        .position(|r| r.trim() == "Second paragraph.")
        .expect("the second paragraph draws");
    assert_eq!(
        second - first,
        2,
        "exactly one blank row separates the paragraphs:\n{rows:#?}"
    );
}

#[test]
fn a_long_line_inside_a_block_still_wraps() {
    // Keeping newlines must not stop wrapping.
    //
    // This test claimed to cover "one very long line" and did not: sixty spaced four-letter words
    // never produce a word wider than the row, so it passed while a long **word** lost its tail.
    // A correctness review found the defect this test was supposed to hold. The long-word case now
    // lives in `long_words.rs`, and this one keeps the many-words case it actually tests.
    let long = "word ".repeat(60);
    let rows = drawn(&long, 40);
    let filled: Vec<&String> = rows.iter().filter(|r| !r.trim().is_empty()).collect();
    assert!(
        filled.len() > 3,
        "a long line still wraps to several rows: {filled:#?}"
    );
    for row in &filled {
        assert!(
            row.chars().count() <= 40,
            "no row may exceed the width: {row:?}"
        );
    }
}

// ---- The escape filter stays. This is the security half. ------------------------

#[test]
fn an_escape_sequence_is_still_dropped_from_block_text() {
    // Model output is untrusted. An escape here could clear the screen, move the cursor to
    // fake an approval prompt, or set the clipboard with OSC 52. Removing the filter to fix
    // the newline bug would have opened exactly that hole.
    let hostile = "before\n\u{1b}[2J\u{1b}[31mred\n\u{1b}]52;c;aGk=\u{7}after";
    let rows = drawn(hostile, 70);
    let all = rows.join("\n");
    assert!(
        !all.contains('\u{1b}'),
        "no escape character may reach the screen:\n{all:?}"
    );
    assert!(
        !all.contains("[2J") && !all.contains("[31m"),
        "a dropped sequence takes its parameters with it:\n{all:?}"
    );
    assert!(
        !all.contains("52;c;"),
        "an OSC 52 clipboard write must not survive:\n{all:?}"
    );
    // The real text is still readable.
    assert!(all.contains("before"), "text before the escape survives");
    assert!(all.contains("after"), "text after the escape survives");
}

#[test]
fn a_stray_control_character_is_still_replaced() {
    let rows = drawn("alpha\u{7}\u{1}beta", 70);
    let all = rows.join("\n");
    assert!(!all.contains('\u{7}'), "a bell must not reach the terminal");
    assert!(!all.contains('\u{1}'), "nor a stray control byte");
}

#[test]
fn a_tab_becomes_spaces_so_the_grid_holds() {
    // A tab has no defined width in a terminal cell, so it would break the column maths.
    // It expands to spaces, which keeps the indent a code line needs.
    let rows = drawn("fn main() {\n\tlet x = 1;\n}", 70);
    let all = rows.join("\n");
    assert!(!all.contains('\t'), "no tab reaches a cell:\n{all:?}");
    assert!(
        rows.iter()
            .any(|r| r.trim_end().ends_with("let x = 1;") && r.starts_with("    ")),
        "the tab became leading spaces, so the indent survives:\n{rows:#?}"
    );
}
