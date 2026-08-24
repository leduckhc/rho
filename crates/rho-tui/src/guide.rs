//! The two minute tour, as a paged panel. See `SPEC-tui-guide`.
//!
//! rho promised a tour in two places and built none. The slash list carried `/guide`,
//! and the first frame advertised it as a starter hint, so the first screen a new user
//! saw invited them to run a command that failed.
//!
//! **Every key and command name here is interpolated, never typed.** A key comes from
//! `bindings()` with its own summary, and a command comes from `slash_commands()`. Prose
//! is a literal, because a fully generated page would make the anti-drift test vacuous:
//! a page built from the table can only ever hold real keys. See
//! `D-the-guide-is-a-paged-panel`.

use crate::bindings::{Binding, bindings};

/// The tallest a page may be, so a short screen loses little.
///
/// A panel yields to the transcript, as the help does, and `panel_lines` truncates from
/// the bottom. The interface state holds no terminal height, so the guide cannot refuse
/// to open on a short screen. A small page is the protection instead.
pub const MAX_GUIDE_PAGE_ROWS: usize = 6;

/// One page of the guide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuidePage {
    /// The heading row, drawn in the heading role.
    pub title: &'static str,
    /// The body rows. An empty string draws a blank line.
    pub rows: Vec<String>,
}

/// The guide panel state: which page it shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Guide {
    /// The zero-based page index. It never exceeds the last page.
    pub page: usize,
}

/// The binding with this exact key, or `None` when the table does not hold it.
///
/// A page asks for a key by name and gets the table's own summary with it, so a renamed
/// binding reaches the tour and a deleted one disappears from it.
fn binding(keys: &str) -> Option<&'static Binding> {
    bindings().iter().find(|candidate| candidate.keys == keys)
}

/// One row for a key: the key, then the table's summary for it.
///
/// A key the table does not hold draws nothing at all, rather than a row promising a key
/// that answers nothing. That is the rule `D-a-panel-nobody-can-open` set.
fn key_row(keys: &str) -> Option<String> {
    binding(keys).map(|found| format!("  {:<14}{}", found.keys, found.summary))
}

/// Every page of the guide, in order, for this session.
///
/// This is the only source of the page count.
pub fn guide_pages(model: &str, provider: &str) -> Vec<GuidePage> {
    let mut pages = Vec::new();

    // Page one: what rho is, and what this session is talking to.
    let mut first = vec![
        "rho sends your prompt to a model. It then runs the tools the model asks for.".to_string(),
        String::new(),
        format!("  {:<14}{model}", "model"),
        format!("  {:<14}{provider}", "provider"),
    ];
    first.extend(key_row("enter"));
    first.extend(key_row("ctrl-c"));
    pages.push(GuidePage {
        title: "What rho is",
        rows: first,
    });

    // Page two: the blunt one. What rho may do to the machine, and how to narrow it.
    pages.push(GuidePage {
        title: "What rho may do here",
        rows: vec![
            "rho reads, writes, and edits files, and it runs shell commands.".to_string(),
            "A path outside the session root is refused.".to_string(),
            "It approves every tool call unless you narrow it.".to_string(),
            String::new(),
            "  --read-only   deny every tool that changes state".to_string(),
            "  --sandbox     confine the shell: off, confined, or strict".to_string(),
        ],
    });

    // Page three: getting around. Every row is the table's own key and summary.
    let mut third: Vec<String> = ["/", "?", "ctrl-r", "ctrl-x ctrl-e", "↑ ↓", "pageup"]
        .iter()
        .filter_map(|keys| key_row(keys))
        .collect();
    third.truncate(MAX_GUIDE_PAGE_ROWS);
    pages.push(GuidePage {
        title: "Getting around",
        rows: third,
    });

    pages
}

/// The footer hint while the guide is open. Pages are numbered from one.
pub fn guide_footer_hint(page: usize, pages: usize) -> String {
    // Numbered from one, because a reader counts from one.
    format!(
        "page {} of {pages} · ← → pages · esc close",
        page.saturating_add(1)
    )
}
