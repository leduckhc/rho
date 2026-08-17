//! Text safety helpers for the transcript.
//!
//! Tool output is untrusted. A file can hold any byte, so a tool result can hold
//! a control character or a terminal escape sequence. The renderer must never
//! send such a byte to the terminal, because it can move the cursor, change the
//! colour, or clear the screen. These helpers make an untrusted string safe to
//! draw, and make any line fit the frame width.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// The glyph that replaces a stripped control character.
const REPLACEMENT: char = '\u{fffd}';

/// The marker that shows a line was cut to fit the width.
const TRUNCATION_MARKER: char = '…';

/// Make an untrusted string safe to draw on one line.
///
/// The function removes every control character and every escape sequence. It
/// replaces each control character with the Unicode replacement glyph, so the
/// reader can see that output was present. It keeps normal printable text, including
/// wide characters and text from other languages. It also flattens a newline to the
/// replacement glyph, because one row is one line.
pub fn sanitize_line(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if is_unsafe(ch) {
            out.push(REPLACEMENT);
        } else {
            out.push(ch);
        }
    }
    out
}

/// True when a character must not reach the terminal.
///
/// A control character can drive the terminal. The `\u{7f}` delete character and
/// the C1 control block are also unsafe. A normal space is safe.
fn is_unsafe(ch: char) -> bool {
    if ch == ' ' {
        return false;
    }
    ch.is_control() || ch == '\u{7f}' || ('\u{80}'..='\u{9f}').contains(&ch)
}

/// Cut a string so its display width is not wider than `max_width`.
///
/// The function measures width with `unicode-width`, so a wide glyph counts as
/// two columns. When the string is too wide, the function keeps a prefix and adds
/// a one-column marker, so the result still fits. rho truncates a long line. It
/// does not wrap it, so one transcript row stays one line. The caller must
/// sanitise the string first.
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
