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

use unicode_width::UnicodeWidthStr;

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
    /// A table's header row, already aligned. Drawn bold.
    TableHead,
    /// The rule under a table's header, already drawn with rule glyphs.
    TableRule,
    /// A table's body row, already aligned.
    TableRow,
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
/// The column divider inside a drawn table.
const TABLE_COLUMN: char = '\u{2502}';
/// The horizontal glyph of a table's rule row.
const TABLE_DASH: char = '\u{2500}';
/// Where a table's rule row crosses a column divider.
const TABLE_CROSS: char = '\u{253c}';
/// The gap either side of a table's column divider.
const TABLE_PAD: usize = 1;
/// The fewest markers a horizontal rule needs.
const RULE_MIN: usize = 3;
/// The most hashes a heading may carry.
const HEADING_MAX: usize = 6;

/// Split a block into rows, and say what each row is.
///
/// The text must already be sanitised. This never adds an escape and never inspects one: it
/// reads only a line's leading punctuation.
pub fn scan_markdown(text: &str) -> Vec<MarkdownLine> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut index = 0usize;
    while index < lines.len() {
        let raw = lines[index];
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
            index += 1;
            continue;
        }
        if in_fence {
            out.push(MarkdownLine {
                text: raw.to_string(),
                kind: MarkdownKind::CodeBlock,
            });
            index += 1;
            continue;
        }

        // A table spans several lines, so it is recognised here and not per line. It needs a
        // header and an alignment rule under it, or it is not a table and stays verbatim.
        if let Some(table) = scan_table(&lines[index..]) {
            let consumed = table.consumed;
            out.extend(table.rows);
            index += consumed;
            continue;
        }

        if let Some(rest) = heading_body(trimmed) {
            out.push(MarkdownLine {
                text: rest.to_string(),
                kind: MarkdownKind::Heading,
            });
            index += 1;
            continue;
        }
        if is_rule(trimmed) {
            out.push(MarkdownLine {
                text: String::new(),
                kind: MarkdownKind::Rule,
            });
            index += 1;
            continue;
        }
        if let Some(rest) = bullet_body(trimmed) {
            out.push(MarkdownLine {
                text: format!("{indent}{BULLET} {rest}"),
                kind: MarkdownKind::Bullet,
            });
            index += 1;
            continue;
        }
        if let Some(rest) = numbered_body(trimmed) {
            out.push(MarkdownLine {
                text: format!("{indent}{rest}"),
                kind: MarkdownKind::Bullet,
            });
            index += 1;
            continue;
        }
        if let Some(rest) = quote_body(trimmed) {
            out.push(MarkdownLine {
                text: format!("{indent}{QUOTE_BAR} {rest}"),
                kind: MarkdownKind::Quote,
            });
            index += 1;
            continue;
        }
        out.push(MarkdownLine {
            text: raw.to_string(),
            kind: MarkdownKind::Text,
        });
        index += 1;
    }
    out
}

/// How a column's text sits in its width.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Centre,
    Right,
}

/// A drawn table, and how many source lines it used.
struct Table {
    rows: Vec<MarkdownLine>,
    consumed: usize,
}

/// Recognise a table at the start of `lines`, and draw it.
///
/// A table needs a header row and an alignment rule under it. Without the rule it is not a
/// table, and every line stays verbatim, because half a table drawn is worse than none. That
/// rule came from the contract review.
fn scan_table(lines: &[&str]) -> Option<Table> {
    if lines.len() < 2 {
        return None;
    }
    let header = split_cells(lines[0])?;
    let aligns = parse_aligns(lines[1], header.len())?;
    let mut body: Vec<Vec<String>> = Vec::new();
    let mut consumed = 2usize;
    while let Some(row) = lines.get(consumed).and_then(|line| split_cells(line)) {
        body.push(row);
        consumed += 1;
    }

    // Column widths come from the widest cell, header included.
    let columns = header.len();
    let mut widths: Vec<usize> = header.iter().map(|cell| cell.width()).collect();
    for row in &body {
        for (index, cell) in row.iter().take(columns).enumerate() {
            widths[index] = widths[index].max(cell.width());
        }
    }

    let mut rows = Vec::with_capacity(body.len() + 2);
    rows.push(MarkdownLine {
        text: draw_row(&header, &widths, &aligns),
        kind: MarkdownKind::TableHead,
    });
    rows.push(MarkdownLine {
        text: draw_rule(&widths),
        kind: MarkdownKind::TableRule,
    });
    for row in &body {
        rows.push(MarkdownLine {
            text: draw_row(row, &widths, &aligns),
            kind: MarkdownKind::TableRow,
        });
    }
    Some(Table { rows, consumed })
}

/// The cells of one table line, or `None` when the line is not one.
///
/// A line needs at least two cells. The outer pipes are optional, because a model often leaves
/// them off.
fn split_cells(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return None;
    }
    let inner = trimmed.trim_start_matches('|').trim_end_matches('|');
    let cells: Vec<String> = inner
        .split('|')
        .map(|cell| visible_text(cell.trim()))
        .collect();
    if cells.len() < 2 {
        return None;
    }
    Some(cells)
}

/// A cell's text with its inline markers removed.
///
/// The width of a column is measured on what the reader sees. Leaving `**` in the measurement
/// and removing it later would shift every column to its right, so the markers come off first.
/// The cost is that emphasis inside a cell is dropped rather than styled: a cell would have to
/// carry runs for that, and a row here is one string. Stated in `SPEC-tui-markdown` section 3b.
fn visible_text(cell: &str) -> String {
    scan_inline(cell).into_iter().map(|run| run.text).collect()
}

/// The alignment of each column, from a rule row such as `|:---|---:|`.
///
/// Returns `None` when the row is not an alignment rule, which is what keeps two ordinary pipe
/// lines verbatim.
fn parse_aligns(line: &str, columns: usize) -> Option<Vec<Align>> {
    let cells = split_cells(line)?;
    if cells.len() != columns {
        return None;
    }
    let mut aligns = Vec::with_capacity(columns);
    for cell in &cells {
        let body = cell.trim();
        let left = body.starts_with(':');
        let right = body.ends_with(':');
        let dashes = body.trim_matches(':');
        if dashes.len() < 3 || !dashes.chars().all(|ch| ch == '-') {
            return None;
        }
        aligns.push(match (left, right) {
            (true, true) => Align::Centre,
            (false, true) => Align::Right,
            _ => Align::Left,
        });
    }
    Some(aligns)
}

/// One drawn row: each cell placed in its column, divided by a column glyph.
fn draw_row(cells: &[String], widths: &[usize], aligns: &[Align]) -> String {
    let pad = " ".repeat(TABLE_PAD);
    let divider = format!("{pad}{TABLE_COLUMN}{pad}");
    let mut parts: Vec<String> = Vec::with_capacity(widths.len());
    for (index, width) in widths.iter().enumerate() {
        // A ragged row is padded, never dropped, or data would disappear.
        let cell = cells.get(index).map(String::as_str).unwrap_or("");
        let align = aligns.get(index).copied().unwrap_or(Align::Left);
        parts.push(place(cell, *width, align));
    }
    parts.join(&divider).trim_end().to_string()
}

/// The rule row. It is drawn with rule glyphs, so it reads as a divider with no styling at all.
///
/// Each segment covers its column plus the one pad column before the divider, and a cross lands
/// exactly under every divider.
fn draw_rule(widths: &[usize]) -> String {
    let mut out = String::new();
    for (index, width) in widths.iter().enumerate() {
        if index > 0 {
            out.push(TABLE_CROSS);
        }
        let span = if index == 0 || index == widths.len() - 1 {
            width + TABLE_PAD
        } else {
            width + TABLE_PAD * 2
        };
        for _ in 0..span {
            out.push(TABLE_DASH);
        }
    }
    out
}

/// Place `text` in `width` columns, by its alignment.
fn place(text: &str, width: usize, align: Align) -> String {
    let room = width.saturating_sub(text.width());
    match align {
        Align::Left => format!("{text}{}", " ".repeat(room)),
        Align::Right => format!("{}{text}", " ".repeat(room)),
        Align::Centre => {
            let left = room / 2;
            format!("{}{text}{}", " ".repeat(left), " ".repeat(room - left))
        }
    }
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

// ---- Inline emphasis. -----------------------------------------------------------

/// One run of a line, with the emphasis that applies to it.
///
/// A run is the smallest unit that carries its own style. `**bold**` becomes one run with
/// `bold` set and the markers gone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineRun {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    /// An inline code span. Verbatim: no other markup applies inside it.
    pub code: bool,
}

/// Split a line into runs, removing the emphasis markers.
///
/// **Every rule here exists to avoid a false positive on code-heavy prose.** A naive "a pair on
/// the same line matches" rule italicises the `3` in `2 * 3 * 4`, which the contract review
/// caught before any code was written. So:
///
/// - **Flanking.** An opening marker is not followed by a space, and a closing marker is not
///   preceded by one. That is what saves arithmetic.
/// - **An underscore never carries emphasis.** This is a deliberate deviation from CommonMark,
///   which renders `__init__` as bold and `_x_` as italic. In a coding agent's prose an
///   underscore is an identifier: `wrap_block`, `snake_case`, `__init__`, `__all__`, `_private`.
///   Only `*` carries emphasis, so a dunder is never eaten.
/// - **No intraword star.** CommonMark italicises the `3` in `2*3*4`, because intraword `*`
///   emphasis is legal. rho requires a non-alphanumeric before the opening run and after the
///   closing run, so multiplication and globs survive.
/// - **A backslash escapes a marker**, and the backslash itself is dropped.
/// - **An unclosed marker is literal text.**
/// - **A code span is verbatim**, and a longer backtick fence may contain a shorter one.
///
/// The scanner never invents a character. Its output is the input with markers removed, which
/// is what lets the renderer trust the width. See `SPEC-tui-markdown` and
/// `D-markdown-line-level-first`.
pub fn scan_inline(text: &str) -> Vec<InlineRun> {
    let chars: Vec<char> = text.chars().collect();
    let mut runs: Vec<InlineRun> = Vec::new();
    let mut buf = String::new();
    let bold = false;
    let italic = false;
    let mut i = 0usize;

    // Close the open run, if it holds anything.
    macro_rules! flush {
        () => {
            if !buf.is_empty() {
                runs.push(InlineRun {
                    text: std::mem::take(&mut buf),
                    bold,
                    italic,
                    code: false,
                });
            }
        };
    }

    while i < chars.len() {
        let ch = chars[i];

        // A backslash escapes the next character, and is itself dropped.
        if ch == '\\' && i + 1 < chars.len() && is_marker(chars[i + 1]) {
            buf.push(chars[i + 1]);
            i += 2;
            continue;
        }

        // A code span wins over every other marker, and its body is verbatim.
        if ch == '`' {
            let fence = run_length(&chars, i, '`');
            if let Some(end) = find_code_close(&chars, i + fence, fence) {
                flush!();
                runs.push(InlineRun {
                    text: chars[i + fence..end].iter().collect(),
                    bold: false,
                    italic: false,
                    code: true,
                });
                i = end + fence;
                continue;
            }
            buf.push(ch);
            i += 1;
            continue;
        }

        // Only `*` opens emphasis. An underscore is an identifier character here.
        if ch == '*' {
            let count = run_length(&chars, i, ch).min(3);
            let opening = bold || italic;
            if !opening
                && can_open(&chars, i, count, ch)
                && let Some(end) = find_emphasis_close(&chars, i + count, count, ch)
            {
                flush!();
                let inner: String = chars[i + count..end].iter().collect();
                // A triple marker is both. A double is bold. A single is italic.
                let (run_bold, run_italic) = match count {
                    1 => (false, true),
                    2 => (true, false),
                    _ => (true, true),
                };
                for run in scan_inline(&inner) {
                    runs.push(InlineRun {
                        text: run.text,
                        bold: run.bold || run_bold,
                        italic: run.italic || run_italic,
                        code: run.code,
                    });
                }
                i = end + count;
                continue;
            }
            buf.push(ch);
            i += 1;
            continue;
        }

        buf.push(ch);
        i += 1;
    }
    flush!();
    if runs.is_empty() {
        runs.push(InlineRun {
            text: String::new(),
            bold: false,
            italic: false,
            code: false,
        });
    }
    runs
}

/// True for a character that carries emphasis meaning.
fn is_marker(ch: char) -> bool {
    // A backslash may escape an underscore even though `_` never opens emphasis, because a
    // writer who typed `\_` meant a literal underscore either way.
    matches!(ch, '*' | '_' | '`' | '\\')
}

/// How many of `marker` run from `start`.
fn run_length(chars: &[char], start: usize, marker: char) -> usize {
    chars[start..]
        .iter()
        .take_while(|ch| **ch == marker)
        .count()
}

/// Whether a marker at `start` may open a span.
///
/// The flanking rule: the character after the marker run must exist and must not be a space.
/// For `_`, the character before must not be a word character, so `snake_case` is left alone.
fn can_open(chars: &[char], start: usize, count: usize, _marker: char) -> bool {
    // The character after the marker run must exist and must not be a space. That is the
    // flanking rule, and it is what saves `2 * 3 * 4`.
    match chars.get(start + count) {
        None => false,
        Some(next) if next.is_whitespace() => false,
        Some(_) => {
            // And the character before must not be alphanumeric, which saves `2*3*4` and a
            // glob such as `a*b`. CommonMark allows intraword `*` emphasis; rho does not.
            !start
                .checked_sub(1)
                .and_then(|index| chars.get(index))
                .is_some_and(|ch| ch.is_alphanumeric())
        }
    }
}

/// Find the closing marker run of the same length, obeying the flanking rule.
fn find_emphasis_close(chars: &[char], from: usize, count: usize, marker: char) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == marker {
            let run = run_length(chars, i, marker);
            // The character before a closing marker must not be a space, and the character
            // after it must not be alphanumeric, so `2*3*4` finds no close.
            let before_ok = i > from && !chars[i - 1].is_whitespace();
            let after_ok = !chars.get(i + run).is_some_and(|ch| ch.is_alphanumeric());
            if run >= count && before_ok && after_ok {
                return Some(i);
            }
            i += run;
            continue;
        }
        i += 1;
    }
    None
}

/// Find the closing backtick fence of exactly `fence` length.
fn find_code_close(chars: &[char], from: usize, fence: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '`' {
            let run = run_length(chars, i, '`');
            if run == fence {
                return Some(i);
            }
            i += run;
            continue;
        }
        i += 1;
    }
    None
}
