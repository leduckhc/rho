//! Sanitiser tests. Tool output is untrusted, so a control character or an
//! escape sequence must not corrupt the display. See `SPEC-tui` Task A.

use rho_tui::{fit_to_width, sanitize_line};
use unicode_width::UnicodeWidthStr;

#[test]
fn sanitize_strips_a_real_escape_sequence() {
    // A real ANSI colour escape sequence.
    let dirty = "before\u{1b}[31mred\u{1b}[0mafter";
    let clean = sanitize_line(dirty);
    assert!(
        !clean.contains('\u{1b}'),
        "an escape byte survived: {clean:?}"
    );
    assert!(clean.contains("before"));
    assert!(clean.contains("red"));
    assert!(clean.contains("after"));
}

#[test]
fn sanitize_replaces_control_characters() {
    let dirty = "a\u{7}b\u{0}c"; // bell and null
    let clean = sanitize_line(dirty);
    assert!(!clean.contains('\u{7}'));
    assert!(!clean.contains('\u{0}'));
    assert!(clean.contains('a'));
    assert!(clean.contains('c'));
}

#[test]
fn sanitize_keeps_normal_and_wide_text() {
    let clean = sanitize_line("hello 世界");
    assert_eq!(clean, "hello 世界");
}

#[test]
fn fit_to_width_keeps_a_short_line() {
    assert_eq!(fit_to_width("hello", 10), "hello");
}

#[test]
fn fit_to_width_truncates_a_long_line_at_the_boundary() {
    let out = fit_to_width("abcdefghij", 5);
    assert!(
        out.width() <= 5,
        "line too wide: {out:?} width {}",
        out.width()
    );
    // The marker shows the cut.
    assert!(out.ends_with('…'));
}

#[test]
fn fit_to_width_counts_wide_glyphs() {
    // Each CJK glyph is two columns. Four glyphs is eight columns.
    let out = fit_to_width("世界世界", 5);
    assert!(
        out.width() <= 5,
        "line too wide: {out:?} width {}",
        out.width()
    );
}

#[test]
fn fit_to_width_zero_is_empty() {
    assert_eq!(fit_to_width("anything", 0), "");
}
