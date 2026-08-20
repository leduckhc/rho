//! Composer cursor and editor tests. See `SPEC-tui-inline-and-composer` section 6.2.
//!
//! The draft is a real editor now. A cursor moves over units, a chip is one unit, and
//! the display wraps by terminal columns. These tests pin section 8's "S3, the composer"
//! table for every item that section 6.2 names.

use rho_tui::{COMPOSER_MAX_TEXT_ROWS, Composer, Unit};

/// A paste large enough to collapse into one chip.
fn big_paste() -> String {
    "x".repeat(1200)
}

#[test]
fn typing_inserts_at_the_cursor() {
    let mut composer = Composer::new();
    composer.insert("ab");
    composer.move_left();
    composer.insert("X");
    assert_eq!(
        composer.model_text(),
        "aXb",
        "the character lands before the last one"
    );
}

#[test]
fn backspace_deletes_at_the_cursor() {
    let mut composer = Composer::new();
    composer.insert("abc");
    composer.move_left();
    assert!(!composer.backspace(), "a text unit is not a chip");
    assert_eq!(
        composer.model_text(),
        "ac",
        "the unit before the cursor goes"
    );
    assert_eq!(composer.cursor(), 1, "the cursor follows the deleted unit");
}

#[test]
fn a_chip_is_one_unit_for_motion() {
    let mut composer = Composer::new();
    composer.paste(&big_paste());
    composer.insert("x");
    assert_eq!(composer.units().len(), 2, "the chip is a single unit");
    assert_eq!(
        composer.cursor(),
        2,
        "the cursor sits after the typed character"
    );
    composer.move_left();
    assert_eq!(
        composer.cursor(),
        1,
        "one left key steps over the character"
    );
    composer.move_left();
    assert_eq!(
        composer.cursor(),
        0,
        "one left key steps over the whole chip"
    );
}

#[test]
fn delete_forward_removes_the_next_unit() {
    let mut composer = Composer::new();
    composer.insert("abc");
    composer.move_line_start();
    composer.delete_forward();
    assert_eq!(
        composer.model_text(),
        "bc",
        "the unit after the cursor goes"
    );
}

#[test]
fn delete_forward_at_the_end_does_nothing() {
    let mut composer = Composer::new();
    composer.insert("ab");
    composer.delete_forward();
    assert_eq!(
        composer.model_text(),
        "ab",
        "the draft is unchanged at the end"
    );
}

#[test]
fn word_motion_crosses_one_word() {
    let mut composer = Composer::new();
    composer.insert("foo bar");
    composer.move_word_left();
    assert_eq!(composer.cursor(), 4, "the cursor stops at the word start");
}

#[test]
fn word_motion_crosses_one_word_forward() {
    let mut composer = Composer::new();
    composer.insert("foo bar");
    composer.move_line_start();
    composer.move_word_right();
    assert_eq!(composer.cursor(), 3, "the cursor stops after the word");
}

#[test]
fn move_right_stops_at_the_end() {
    let mut composer = Composer::new();
    composer.insert("ab");
    composer.move_line_start();
    composer.move_right();
    composer.move_right();
    composer.move_right();
    assert_eq!(
        composer.cursor(),
        2,
        "the cursor does not pass the last unit"
    );
}

#[test]
fn line_start_and_line_end_bound_one_row() {
    let mut composer = Composer::new();
    composer.insert("ab");
    composer.insert_newline();
    composer.insert("cde");
    composer.move_line_start();
    assert_eq!(composer.cursor(), 3, "line start lands after the newline");
    composer.move_line_end();
    assert_eq!(composer.cursor(), 6, "line end lands at the row end");
}

#[test]
fn down_moves_the_cursor_then_gives_up_the_key() {
    let mut composer = Composer::new();
    composer.insert("abc");
    assert!(
        !composer.move_row_down(80),
        "move_row_down returns false on the last row"
    );
}

#[test]
fn up_moves_the_cursor_in_a_tall_draft() {
    let mut composer = Composer::new();
    composer.insert("ab");
    composer.insert_newline();
    composer.insert("cd");
    let before = composer.cursor();
    assert!(
        composer.move_row_up(80),
        "the cursor moves to the row above"
    );
    assert_ne!(
        composer.cursor(),
        before,
        "the cursor moved, not the history"
    );
    assert!(
        !composer.move_row_up(80),
        "move_row_up returns false at the top row"
    );
}

#[test]
fn kill_to_line_end_fills_the_kill_buffer() {
    let mut composer = Composer::new();
    composer.insert("hello world");
    composer.move_line_start();
    composer.move_word_right();
    composer.kill_to_line_end();
    assert_eq!(composer.model_text(), "hello", "the tail is cut");
    composer.yank();
    assert_eq!(
        composer.model_text(),
        "hello world",
        "yank restores the cut text"
    );
}

#[test]
fn kill_to_line_start_keeps_the_tail() {
    let mut composer = Composer::new();
    composer.insert("hello world");
    composer.move_line_start();
    composer.move_word_right();
    composer.kill_to_line_start();
    assert_eq!(
        composer.model_text(),
        " world",
        "the text after the cursor stays"
    );
}

#[test]
fn kill_word_left_cuts_one_word() {
    let mut composer = Composer::new();
    composer.insert("foo bar");
    composer.kill_word_left();
    assert_eq!(composer.model_text(), "foo ", "one word goes");
    composer.yank();
    assert_eq!(
        composer.model_text(),
        "foo bar",
        "the kill buffer holds the word"
    );
}

#[test]
fn the_composer_height_is_capped() {
    let mut composer = Composer::new();
    composer.insert(&"line\n".repeat(20));
    assert_eq!(
        composer.display_lines(80).len(),
        COMPOSER_MAX_TEXT_ROWS,
        "twenty draft rows render at the cap"
    );
}

#[test]
fn the_cursor_cell_follows_the_wrap() {
    let mut composer = Composer::new();
    composer.insert("abcdef");
    let (row, _col) = composer.cursor_cell(4);
    assert_eq!(row, 1, "a wrapped row puts the cursor on the second row");
}

#[test]
fn set_text_replaces_the_held_pastes() {
    let mut composer = Composer::new();
    composer.paste(&big_paste());
    composer.insert("x");
    composer.set_text("new text");
    assert_eq!(composer.chip_count(), 0, "set_text drops every held paste");
    assert_eq!(
        composer.take(),
        "new text",
        "take returns the new text alone"
    );
}

#[test]
fn an_empty_draft_reports_empty() {
    let empty = Composer::new();
    assert!(empty.is_empty(), "a fresh draft holds no unit");
    let mut full = Composer::new();
    full.insert("x");
    assert!(!full.is_empty(), "a draft with a unit is not empty");
}

#[test]
fn the_units_report_a_chip_as_one_unit() {
    let mut composer = Composer::new();
    composer.insert("a");
    composer.paste(&big_paste());
    composer.insert("b");
    let units = composer.units();
    let pastes = units
        .iter()
        .filter(|unit| matches!(unit, Unit::Paste))
        .count();
    assert_eq!(pastes, 1, "units holds one Unit::Paste for a chip");
    assert_eq!(units.len(), 3, "the chip counts as one unit among the text");
}

// ---- The composer scrolls, so the cursor is always on screen. -------------------
//
// `display_lines` caps the height at `COMPOSER_MAX_TEXT_ROWS`. The cap kept the first
// rows, and `cursor_cell` reported a row from the uncapped layout. So a draft taller
// than the cap drew the wrong rows and put the cursor outside the box. These tests pin
// the bound, not one example.

#[test]
fn the_cursor_row_is_always_inside_the_drawn_rows() {
    let mut composer = Composer::new();
    for index in 0..20 {
        composer.insert(&format!("row {index}"));
        composer.insert_newline();
    }
    // Walk the cursor over every unit, and check the bound at each stop.
    composer.move_line_start();
    for stop in 0..composer.units().len() {
        let (row, _) = composer.cursor_cell(40);
        let drawn = composer.display_lines(40).len();
        assert!(
            row < drawn.max(1),
            "cursor row {row} must be inside the {drawn} drawn rows, at stop {stop}"
        );
        composer.move_right();
    }
}

#[test]
fn a_tall_draft_shows_the_rows_around_the_cursor() {
    let mut composer = Composer::new();
    for index in 0..20 {
        composer.insert(&format!("row {index}"));
        composer.insert_newline();
    }
    // The cursor sits after the last newline, so the window must hold the newest rows.
    let lines = composer.display_lines(40);
    assert_eq!(lines.len(), COMPOSER_MAX_TEXT_ROWS);
    assert!(
        lines.iter().any(|line| line.contains("row 19")),
        "the window must hold the row the cursor is on, got {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("row 0")),
        "the window must not still show the first row, got {lines:?}"
    );
}
