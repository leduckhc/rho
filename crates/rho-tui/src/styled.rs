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

/// One drawn row: styled runs, left to right.
pub type StyledLine = Vec<(String, Style)>;

/// A row of one run. Most rows have no inline styling and use this.
pub fn one(run: (String, Style)) -> StyledLine {
    vec![run]
}

// `styled_width` and `styled_text` lived here and are deliberately gone.
//
// Both were public, both were exported, and a test-quality audit found **zero callers anywhere in
// the workspace**, tests included. `styled_text`'s own doc claimed it was "for a test and for the
// transcript dump": no test called it, and there is no dump. That is exactly the untested public
// surface AGENTS.md step 8 names as where this project's defects have lived, so they are removed
// rather than given a test that exists only to justify them.
//
// A row's width is measured where it is needed, inside `put`, which is the one place that has to
// know. If a caller ever needs the width of a row again, it comes back with the caller.
