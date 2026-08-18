//! Concise mode: a collapsed tool row and its fold state.
//!
//! Concise mode collapses a tool row to its header. The header keeps the verb, the
//! payload, the duration slot, the status glyph, and the caret. An expanded row
//! adds the body and keeps the header. Concise mode is opt-in, and the default is
//! off. A failed row expands itself, because the output is the point. See
//! `SPEC-tui-experience` section 7 and `docs/tui-design.md` section 6.
//!
//! Note: the src file list for this stage named `motion.rs`, `theme.rs`, and
//! `bindings.rs`, but not a home for the concise signatures. This module holds
//! them, because the concise tests need types to compile against, and it touches
//! no file owned by another writer.

use crate::state::Row;

/// True when concise mode collapses a tool row to its header. Off by default.
pub const CONCISE_MODE_DEFAULT: bool = false;

/// Whether one transcript row is collapsed or expanded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowFold {
    Collapsed,
    Expanded,
}

/// The fold a fresh tool row takes, from the concise setting and whether it failed.
///
/// With concise mode off, a row is expanded, so its body shows. With concise mode
/// on, a row is collapsed, unless it failed. A failed row expands itself.
pub fn initial_tool_fold(concise: bool, failed: bool) -> RowFold {
    // The body shows unless concise mode is on. A failed row always shows its body,
    // because its output is the point.
    if !concise || failed {
        RowFold::Expanded
    } else {
        RowFold::Collapsed
    }
}

/// Toggle a fold state. `enter` on a selected row runs this.
pub fn toggle_fold(fold: RowFold) -> RowFold {
    match fold {
        RowFold::Collapsed => RowFold::Expanded,
        RowFold::Expanded => RowFold::Collapsed,
    }
}

/// The caret glyph for a fold state. A collapsed row shows `▸`, an expanded row
/// shows `▾`.
pub fn fold_caret(fold: RowFold) -> &'static str {
    match fold {
        RowFold::Collapsed => "▸",
        RowFold::Expanded => "▾",
    }
}

/// The visible lines of a tool row under a fold state.
///
/// A collapsed row is one header line, with the caret and the verb. An expanded row
/// keeps the header and adds the body lines.
pub fn tool_row_lines(row: &Row, fold: RowFold, _width: usize) -> Vec<String> {
    let Row::Tool { name, preview, .. } = row else {
        return Vec::new();
    };

    let header = format!("{} {}", fold_caret(fold), name);
    let mut lines = vec![header];

    if fold == RowFold::Expanded {
        lines.extend(preview.lines().map(str::to_string));
    }

    lines
}
