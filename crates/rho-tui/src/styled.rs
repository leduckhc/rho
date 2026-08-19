//! A drawn row, as styled runs.
//!
//! A row used to be `(String, Style)`, so one style covered a whole row and `**bold**` could
//! not be bold. A run is the smallest unit that carries its own style, and a row is a sequence
//! of runs, left to right.
//!
//! **Where the width invariant lives.** Review asked whether this should be a struct with a
//! smart constructor, because a bare alias cannot stop a producer building a row wider than
//! the terminal. The answer here is that the invariant lives at the single consumer, `put`,
//! which clips a row to the width and pads it out. One place enforces it, in code, for every
//! producer including a future one. A struct would spread the same rule across 30 call sites
//! and still depend on each of them calling it. See `SPEC-tui-markdown` section 3a item 2.

use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

/// One drawn row: styled runs, left to right.
pub type StyledLine = Vec<(String, Style)>;

/// A row of one run. Most rows have no inline styling and use this.
pub fn one(run: (String, Style)) -> StyledLine {
    vec![run]
}

/// The display width of a row, summed over its runs.
pub fn styled_width(line: &StyledLine) -> usize {
    line.iter().map(|(text, _)| text.width()).sum()
}

/// The plain text of a row, runs joined. For a test and for the transcript dump.
pub fn styled_text(line: &StyledLine) -> String {
    line.iter().map(|(text, _)| text.as_str()).collect()
}
