//! A word longer than the row must break, not lose its tail.
//!
//! Found by a harsh correctness review, then reproduced by driving `render`. `wrap` never broke a
//! word wider than the row: it emitted the word whole, and `put` then clipped it to the frame and
//! dropped the rest with no marker. Measured before the fix:
//!
//! | input | width | sent | drawn |
//! | --- | --- | --- | --- |
//! | 30 `x` characters | 10 | 30 | **10** |
//! | the same 30 in a code span | 10 | 30 | 30 |
//! | a 60 glyph CJK paragraph | 80 | 60 | **40** |
//! | a 72 character URL | 40 | 72 | **40** |
//!
//! The two paths disagreed, which is what made it visible: `wrap_runs` broke a long word and
//! `wrap` did not, so the same text survived inside backticks and was cut without them.
//!
//! **This matters for a coding agent specifically.** A URL, a path, a hash, a base64 blob, and a
//! stack-trace line are all one long word. So is a whole CJK or Thai paragraph, because those
//! scripts do not put spaces between words.
//!
//! The test that should have caught it used `"word ".repeat(60)`, sixty short words with spaces, so
//! it never had a word wider than the row. See `D-a-long-word-breaks`.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rho_tui::{Row, TuiState, render};
use unicode_width::UnicodeWidthStr;

/// The visible text of every drawn row, with the continuation cell of a wide glyph removed.
fn visible(text: &str, width: u16) -> Vec<String> {
    let mut state = TuiState::default();
    state.rows.push(Row::Assistant {
        text: text.to_string(),
    });
    let backend = TestBackend::new(width, 24);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render(&state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    (0..24)
        .map(|y| {
            let mut row = String::new();
            let mut skip = false;
            for x in 0..width {
                let symbol = buffer[(x, y)].symbol();
                if skip {
                    skip = false;
                    continue;
                }
                row.push_str(symbol);
                if symbol.width() == 2 {
                    skip = true;
                }
            }
            row.trim_end().to_string()
        })
        .collect()
}

/// Every character of `text` that the frame drew, in order, ignoring row breaks.
fn drawn_chars(text: &str, width: u16, marker: char) -> String {
    visible(text, width)
        .into_iter()
        .filter(|row| row.chars().any(|ch| ch == marker))
        .collect::<Vec<_>>()
        .join("")
}

#[test]
fn a_word_longer_than_the_row_keeps_every_character() {
    let word = "x".repeat(30);
    let drawn = drawn_chars(&word, 10, 'x');
    assert_eq!(
        drawn.chars().filter(|ch| *ch == 'x').count(),
        30,
        "every character survives a break: {drawn:?}"
    );
}

#[test]
fn a_long_url_keeps_its_tail() {
    // The tail is the part that matters: a fragment, a query, or a line number.
    let url = "https://example.com/a/very/long/path/that/keeps/going/and/going#fragment";
    let rows = visible(url, 40);
    let joined = rows.join("");
    assert!(
        joined.contains("#fragment"),
        "the fragment must survive:\n{rows:#?}"
    );
}

#[test]
fn a_cjk_paragraph_keeps_every_glyph() {
    // CJK puts no spaces between words, so a whole paragraph is one word. A third of this was
    // being dropped.
    let text = "\u{8aac}\u{660e}".repeat(30);
    let rows = visible(&text, 80);
    let drawn: usize = rows
        .iter()
        .map(|row| {
            row.chars()
                .filter(|ch| *ch == '\u{8aac}' || *ch == '\u{660e}')
                .count()
        })
        .sum();
    assert_eq!(drawn, 60, "every glyph survives:\n{rows:#?}");
}

#[test]
fn the_markup_free_path_agrees_with_the_run_path() {
    // The two paths disagreed, and that is what exposed the defect. The same long word must draw
    // the same number of characters with and without a code span around it.
    let word = "y".repeat(30);
    let plain = drawn_chars(&word, 12, 'y');
    let spanned = drawn_chars(&format!("`{word}`"), 12, 'y');
    assert_eq!(
        plain.chars().filter(|ch| *ch == 'y').count(),
        spanned.chars().filter(|ch| *ch == 'y').count(),
        "the fast path and the run path must not disagree"
    );
}

#[test]
fn no_row_exceeds_the_frame_in_display_columns() {
    // Measured in display columns, not characters. A review found `wrap_runs` appending the
    // character that tipped a word over the width, so a row could be one or two columns too wide
    // and a wide glyph at the edge was then dropped by the renderer.
    let cases: [(&str, u16); 5] = [
        ("z".repeat(50).leak(), 20),
        ("\u{672c}".repeat(20).leak(), 11),
        ("a b c ".repeat(30).leak(), 15),
        ("**bold**".repeat(20).leak(), 17),
        ("`code`".repeat(20).leak(), 13),
    ];
    for (text, width) in cases {
        for row in visible(text, width) {
            assert!(
                row.width() <= width as usize,
                "a row of {} columns exceeds the {width} column frame: {row:?}",
                row.width()
            );
        }
    }
}
