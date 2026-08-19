//! Markdown as colour, at line level.
//!
//! rho drew a model answer as plain text, so `# heading`, a fence, and `> quote` reached the
//! screen as their own punctuation. This splits a block into rows and says what each row is,
//! so the renderer can colour it instead.
//!
//! **Line level only, and deliberately so.** Every kind here styles a whole row, so the
//! markup can be stripped *before* the text is wrapped and the measure stays honest.
//! `wrap_block` measures what it is given: removing `**` after wrapping would leave every
//! affected row four columns narrower than the wrap assumed. Inline emphasis therefore needs
//! a run-level row contract and wrapping over runs, which is phase 2 of
//! `SPEC-tui-markdown`. See `D-markdown-line-level-first`.
//!
//! **This is a fixed subset, not a parser.** No `pulldown-cmark` and no `syntect`. rho's
//! binary size and start time are features, and a general parser invites the scope jcode
//! took: its markdown crate is 7140 lines. pi refuses to guess a code language for a related
//! reason, and its comment says auto-detection "can misidentify prose as AppleScript ...
//! coloring random English words as keywords". rho guesses nothing.
//!
//! **Every rule below requires a separator.** A coding agent's prose is full of `--no-mouse`,
//! `#[derive(Debug)]`, `1.2.3`, and `>out.txt`. A marker with no space after it is text.

/// What one row of a block is, for colour only.
///
/// A new kind is a new variant plus one arm in `role_for`, and the exhaustive match makes the
/// compiler name every place that must answer for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkdownKind {
    /// Ordinary prose, and a blank row.
    Text,
    /// A heading, with its hashes removed.
    Heading,
    /// The ``` line that opens or closes a block, with its language kept.
    Fence,
    /// A line inside a fenced block. Never reinterpreted.
    CodeBlock,
    /// A quoted line, with its `>` replaced by a bar.
    Quote,
    /// A list item, with its marker replaced by a glyph or kept as a number.
    Bullet,
    /// A horizontal rule. The renderer draws it full width.
    Rule,
}

/// One row of a block: the text to draw, and what it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownLine {
    /// The text with its line markup removed, ready to wrap.
    pub text: String,
    pub kind: MarkdownKind,
}

/// The glyph that replaces a list marker.
const BULLET: &str = "•";
/// The glyph that marks a quoted line.
const QUOTE_BAR: &str = "┃";
/// The fewest markers a horizontal rule needs.
const RULE_MIN: usize = 3;
/// The most hashes a heading may carry.
const HEADING_MAX: usize = 6;

/// Split a block into rows, and say what each row is.
///
/// The text must already be sanitised. This never adds an escape and never inspects one: it
/// reads only a line's leading punctuation.
pub fn scan_markdown(text: &str) -> Vec<MarkdownLine> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for raw in text.split('\n') {
        let trimmed = raw.trim_start_matches(' ');
        let indent_len = raw.len() - trimmed.len();
        let indent = &raw[..indent_len];

        // A fence toggles, and it is the only markup a code line may carry. An unclosed fence
        // keeps the rest of the block as code, which is the normal state of a streamed answer.
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            out.push(MarkdownLine {
                text: raw.to_string(),
                kind: MarkdownKind::Fence,
            });
            continue;
        }
        if in_fence {
            out.push(MarkdownLine {
                text: raw.to_string(),
                kind: MarkdownKind::CodeBlock,
            });
            continue;
        }

        if let Some(rest) = heading_body(trimmed) {
            out.push(MarkdownLine {
                text: rest.to_string(),
                kind: MarkdownKind::Heading,
            });
            continue;
        }
        if is_rule(trimmed) {
            out.push(MarkdownLine {
                text: String::new(),
                kind: MarkdownKind::Rule,
            });
            continue;
        }
        if let Some(rest) = bullet_body(trimmed) {
            out.push(MarkdownLine {
                text: format!("{indent}{BULLET} {rest}"),
                kind: MarkdownKind::Bullet,
            });
            continue;
        }
        if let Some(rest) = numbered_body(trimmed) {
            out.push(MarkdownLine {
                text: format!("{indent}{rest}"),
                kind: MarkdownKind::Bullet,
            });
            continue;
        }
        if let Some(rest) = quote_body(trimmed) {
            out.push(MarkdownLine {
                text: format!("{indent}{QUOTE_BAR} {rest}"),
                kind: MarkdownKind::Quote,
            });
            continue;
        }
        out.push(MarkdownLine {
            text: raw.to_string(),
            kind: MarkdownKind::Text,
        });
    }
    out
}

/// The text of a heading, if the line is one. A heading needs a space after its hashes, so
/// `#[derive(Debug)]` and `#42` stay text.
fn heading_body(line: &str) -> Option<&str> {
    let hashes = line.len() - line.trim_start_matches('#').len();
    if hashes == 0 || hashes > HEADING_MAX {
        return None;
    }
    let rest = &line[hashes..];
    rest.strip_prefix(' ').map(str::trim_end)
}

/// True when the line is only rule markers, at least three of one kind.
fn is_rule(line: &str) -> bool {
    let line = line.trim_end();
    if line.len() < RULE_MIN {
        return false;
    }
    ['-', '*', '_']
        .iter()
        .any(|marker| line.chars().all(|ch| ch == *marker))
}

/// The text of a list item, if the line is one. A marker needs a space after it, so
/// `--no-mouse` and `-42` stay text.
fn bullet_body(line: &str) -> Option<&str> {
    for marker in ['-', '*', '+'] {
        if let Some(rest) = line.strip_prefix(marker)
            && let Some(rest) = rest.strip_prefix(' ')
        {
            return Some(rest.trim_end());
        }
    }
    None
}

/// A numbered item keeps its number, so the reader keeps the order. `1.2.3` is not a list,
/// because a digit follows the dot.
fn numbered_body(line: &str) -> Option<&str> {
    let digits = line.len()
        - line
            .trim_start_matches(|ch: char| ch.is_ascii_digit())
            .len();
    if digits == 0 {
        return None;
    }
    let rest = &line[digits..];
    let rest = rest.strip_prefix('.')?;
    rest.strip_prefix(' ')?;
    Some(line.trim_end())
}

/// The text of a quoted line. `>out.txt` is a shell redirect, not a quote.
fn quote_body(line: &str) -> Option<&str> {
    line.strip_prefix('>')
        .and_then(|rest| rest.strip_prefix(' '))
        .map(str::trim_end)
}
