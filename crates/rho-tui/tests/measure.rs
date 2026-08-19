//! Text fills the terminal, instead of stopping at a reading measure.
//!
//! The transcript wrapped at `min(80, width - 10)`, so a 156 column terminal used half its width
//! and the owner asked for the full width with no margin.
//!
//! **One column is reserved, and only one.** The scroll rail draws at `width - 1`, so text that
//! used the whole width would have its last character overwritten every time the transcript
//! overflowed. Reserving the column only when the rail shows does not work either: the measure
//! decides how many rows the text wraps to, the row count decides whether it overflows, and the
//! overflow would decide the measure. That is circular, so the column is reserved always.
//!
//! See `D-text-fills-the-width`.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rho_tui::{Row, TuiState, render};

fn drawn(rows_in: Vec<Row>, width: u16, height: u16) -> Vec<String> {
    let mut state = TuiState::default();
    state.rows = rows_in;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render(&state, frame))
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
fn text_wraps_to_the_terminal_width_not_to_eighty() {
    // The old cap was 80 columns with a 10 column margin, so a 156 column terminal wrapped at 80
    // and wasted half the screen.
    // Only the transcript rows count. A composer rule and the footer are chrome and fill the
    // frame on purpose, and an earlier version of this test measured those and reported 156.
    let text = "word ".repeat(200);
    let rows = drawn(vec![Row::Assistant { text }], 156, 30);
    let widest = rows
        .iter()
        .filter(|row| row.contains("word"))
        .map(|row| row.trim_end().chars().count())
        .max()
        .expect("the text drew");
    assert!(
        widest > 120,
        "text must fill the terminal, widest row was {widest} columns of 156"
    );
    assert!(
        widest <= 155,
        "and it must leave the rail column, widest row was {widest}"
    );
}

#[test]
fn the_rail_never_overwrites_a_character_of_text() {
    // This is why one column is reserved. With the full width used, a scrolling transcript ate the
    // last character of every wrapped line.
    // Again, only the transcript rows. A composer rule is a full-width divider by design, and an
    // earlier version of this test failed on one.
    let text = "abcdefgh ".repeat(400);
    let rows = drawn(vec![Row::Assistant { text }], 60, 12);
    let text_rows: Vec<&String> = rows.iter().filter(|row| row.contains("abcdefgh")).collect();
    assert!(!text_rows.is_empty(), "the text drew");
    for row in text_rows {
        let last = row.chars().last().expect("a last column");
        assert!(
            last == ' ' || last == '\u{2502}',
            "the last column of a text row must be blank or the rail, found {last:?}: {row:?}"
        );
    }
}

#[test]
fn a_horizontal_rule_spans_the_whole_width() {
    // A divider is a divider. It was capped at 80 columns, which left it short of the frame.
    let rows = drawn(
        vec![Row::Assistant {
            text: "before\n\n---\n\nafter".to_string(),
        }],
        140,
        20,
    );
    let rule = rows
        .iter()
        .find(|row| row.trim_start().starts_with('\u{2500}'))
        .expect("the rule draws");
    assert_eq!(
        rule.chars().filter(|ch| *ch == '\u{2500}').count(),
        140,
        "the rule fills every column: {rule:?}"
    );
}

#[test]
fn a_narrow_terminal_still_leaves_room_for_text() {
    // The reserved column must not eat a narrow screen.
    for width in [20u16, 30, 40] {
        let rows = drawn(
            vec![Row::Assistant {
                text: "alpha beta gamma delta epsilon".to_string(),
            }],
            width,
            20,
        );
        let joined: String = rows.join("");
        for word in ["alpha", "beta", "gamma", "delta", "epsilon"] {
            assert!(
                joined.contains(word),
                "at width {width} the word {word:?} must draw"
            );
        }
    }
}
