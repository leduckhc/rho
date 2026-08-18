//! Terminal-safe text for the interface.
//!
//! The filter lives in `rho-redact`, so there is one implementation and one test suite.
//! This module once carried its own copy, and the three copies in the workspace had
//! already drifted. This one replaced each unsafe character, which was safe but left
//! visible rubbish: `red\x1b[31mtext` rendered as `red\u{fffd}[31mtext`. The shared
//! filter drops the whole sequence, so it renders as `redtext`. Decision D-one-redaction-home records
//! the consolidation.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Sanitise one line of untrusted text.
///
/// Tool output, a file's contents, and a task's progress message are all untrusted. An
/// escape sequence in any of them can move the cursor or clear the screen.
pub fn sanitize_line(input: &str) -> String {
    rho_redact::sanitize_line(input)
}

/// The glyph that marks a cut line.
const TRUNCATION_MARKER: char = '\u{2026}';

/// Cut text to `max_width` terminal columns, and mark the cut.
///
/// A byte or character count is wrong for a terminal, because a wide character such as
/// a Japanese glyph occupies two columns. Cutting by character would overflow the row
/// and break the layout.
///
/// A cut line ends with an ellipsis, so the reader can tell that text was removed. The
/// marker takes one column out of the budget.
pub fn fit_to_width(input: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if input.width() <= max_width {
        return input.to_string();
    }
    // Reserve one column for the marker.
    let budget = max_width - 1;
    let mut out = String::new();
    let mut used = 0usize;
    for ch in input.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push(TRUNCATION_MARKER);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_line_drops_an_escape_sequence_whole() {
        assert_eq!(sanitize_line("red\u{1b}[31mtext"), "redtext");
    }

    #[test]
    fn sanitize_line_folds_a_newline_to_a_space() {
        assert_eq!(sanitize_line("a\nb"), "a b");
    }

    #[test]
    fn sanitize_line_never_leaves_an_escape() {
        assert!(!sanitize_line("\u{1b}]0;title\u{7}x").contains('\u{1b}'));
    }

    #[test]
    fn fit_to_width_marks_a_cut_and_keeps_the_budget() {
        // The marker takes one column, so a cut at width 4 keeps three columns of text.
        let out = fit_to_width("abcdefgh", 4);
        assert!(out.ends_with('\u{2026}'), "{out:?}");
        assert!(
            unicode_width::UnicodeWidthStr::width(out.as_str()) <= 4,
            "{out:?}"
        );
    }

    #[test]
    fn fit_to_width_counts_a_wide_character_as_two() {
        // Three wide glyphs are six columns. At width 5 only two fit beside the marker.
        let out = fit_to_width("日本語", 5);
        assert!(
            unicode_width::UnicodeWidthStr::width(out.as_str()) <= 5,
            "{out:?}"
        );
        assert!(out.starts_with('日'), "{out:?}");
    }

    #[test]
    fn fit_to_width_keeps_short_text_unmarked() {
        assert_eq!(fit_to_width("ab", 10), "ab");
    }

    #[test]
    fn fit_to_width_of_zero_is_empty() {
        assert_eq!(fit_to_width("abc", 0), "");
    }
}
