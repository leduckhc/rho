//! Paste collapsing and attachments.
//!
//! A large paste collapses to one chip, and the full text is held aside so it still
//! reaches the model on send. A paste burst on a terminal with no bracketed paste
//! flushes through the paste path, so a pasted `?` never opens the help. An image
//! attachment becomes a bounded chip, capped by size and confined to the session
//! root. See `SPEC-tui-experience` sections 5 and 6.

use std::path::Path;

use unicode_width::UnicodeWidthStr;

/// A paste at or above this character count collapses to one chip.
pub const LARGE_PASTE_CHARS: usize = 1000;

/// The largest gap, in milliseconds, between two keys that still counts as one
/// burst. A human types slower than this; a pasted stream arrives faster.
const BURST_GAP_MILLIS: u64 = 10;

/// The fewest keys a fast run needs before it flushes as a paste rather than as
/// individual key presses. A lone fast key keeps its shortcut meaning.
const BURST_MIN_KEYS: usize = 2;

/// Bytes in one mebibyte, the unit the size labels report.
const BYTES_PER_MB: f64 = 1024.0 * 1024.0;

/// Render a byte count as a megabyte label, for example `1.2MB` or `12MB`.
///
/// A whole number of megabytes drops the decimal, so the limit message reads
/// `12MB` and not `12.0MB`.
fn megabyte_label(bytes: u64) -> String {
    let mb = bytes as f64 / BYTES_PER_MB;
    let rounded = (mb * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 {
        format!("{}MB", rounded as u64)
    } else {
        format!("{rounded:.1}MB")
    }
}

/// The bounded composer height, in text rows. Ten rows with the box borders.
pub const COMPOSER_MAX_TEXT_ROWS: usize = 8;

/// The provider size cap for one image attachment, in bytes.
pub const IMAGE_MAX_BYTES: u64 = 5 * 1024 * 1024;

/// The chip that stands in for one large paste in the composer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteChip {
    /// The character count of the held text.
    pub chars: usize,
    /// The repeat index. The first paste of a size is 1, the second is 2.
    pub repeat: u32,
}

/// The chip label, for example `[paste 12431 chars]` or `[paste 12431 chars #2]`.
pub fn paste_chip_label(chip: &PasteChip) -> String {
    if chip.repeat <= 1 {
        format!("[paste {} chars]", chip.chars)
    } else {
        format!("[paste {} chars #{}]", chip.chars, chip.repeat)
    }
}

/// The chip that stands in for one image attachment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageChip {
    /// The attachment index. The first is 1.
    pub index: u32,
    /// The image size in bytes.
    pub bytes: u64,
}

/// The chip label, for example `[image #1 1.2MB]`.
pub fn image_chip_label(chip: &ImageChip) -> String {
    format!("[image #{} {}]", chip.index, megabyte_label(chip.bytes))
}

/// The outcome of an attach attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachOutcome {
    /// The attachment is accepted, as a bounded chip.
    Attached(ImageChip),
    /// The attachment is refused. The string is the footer message, in `warn`.
    /// A refused attachment never reaches the model.
    Refused(String),
}

/// Attach an image by path.
///
/// The path must stay inside `session_root`, so a draft cannot read a file the
/// session may not read. The image must not exceed `IMAGE_MAX_BYTES`. A refusal
/// carries the footer message and no chip, so the attachment never reaches the model.
pub fn attach_image(session_root: &Path, path: &Path, bytes: u64) -> AttachOutcome {
    // Confinement is a security boundary, so it is checked before anything else.
    // A draft must not read a file the session may not read.
    if !confine(session_root, path) {
        return AttachOutcome::Refused("image path is outside the session root".to_string());
    }
    if bytes > IMAGE_MAX_BYTES {
        return AttachOutcome::Refused(format!(
            "image too large: {}, the limit is {}",
            megabyte_label(bytes),
            megabyte_label(IMAGE_MAX_BYTES),
        ));
    }
    AttachOutcome::Attached(ImageChip { index: 1, bytes })
}

/// True when `path` resolves to a location inside `session_root`.
///
/// Both sides are canonicalized, so a symlink cannot smuggle a path out of the
/// root. A path that cannot be resolved is treated as outside.
fn confine(session_root: &Path, path: &Path) -> bool {
    let (Ok(root), Ok(target)) = (session_root.canonicalize(), path.canonicalize()) else {
        return false;
    };
    target.starts_with(&root)
}

/// One key in a burst, with the gap in milliseconds since the previous key.
///
/// A test passes the gaps as data, so no test reads a clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BurstKey {
    /// The character the key carries.
    pub ch: char,
    /// The milliseconds since the previous key.
    pub gap_millis: u64,
}

/// One routed input, after the burst detector runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoutedInput {
    /// A single key press that keeps its shortcut meaning, for example `?` opens help.
    Key(char),
    /// A collapsed paste. Its characters carry no shortcut meaning, so a pasted `?`
    /// never opens the help.
    Paste(String),
}

/// Route a burst of key events.
///
/// Keys inside the burst window collapse to one paste, flushed through the paste
/// path. So a pasted `?` becomes paste text, and never a help shortcut.
pub fn route_burst(keys: &[BurstKey]) -> Vec<RoutedInput> {
    let mut routed = Vec::new();
    let mut run: Vec<char> = Vec::new();

    let flush = |run: &mut Vec<char>, routed: &mut Vec<RoutedInput>| {
        if run.len() >= BURST_MIN_KEYS {
            routed.push(RoutedInput::Paste(run.iter().collect()));
        } else {
            routed.extend(run.iter().map(|&ch| RoutedInput::Key(ch)));
        }
        run.clear();
    };

    for (index, key) in keys.iter().enumerate() {
        // The first key opens a run. A later key joins the run only when it lands
        // inside the burst window, otherwise the run flushes and a new one opens.
        if index > 0 && key.gap_millis > BURST_GAP_MILLIS {
            flush(&mut run, &mut routed);
        }
        run.push(key.ch);
    }
    flush(&mut run, &mut routed);
    routed
}

/// One unit of the draft, as the cursor moves over it. A chip is one unit, so one left
/// key steps over a whole paste.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// One character of typed text.
    Char(char),
    /// One newline, which starts a display row.
    Newline,
    /// One collapsed paste, shown as its chip label.
    Paste,
}

/// The first row of the drawn window, so the cursor row is always inside it.
///
/// The draft can be taller than `COMPOSER_MAX_TEXT_ROWS`. The window then scrolls with the
/// cursor: it holds the cursor row, and it holds the newest rows once the cursor reaches
/// the end. A window that always started at row zero drew the cursor outside the box.
fn window_start(lines: &[String], positions: &[(usize, usize)], cursor: usize) -> usize {
    if lines.len() <= COMPOSER_MAX_TEXT_ROWS {
        return 0;
    }
    let cursor_row = positions.get(cursor).map(|(row, _)| *row).unwrap_or(0);
    // Keep the cursor row on the last drawn line while the draft grows downward.
    cursor_row.saturating_sub(COMPOSER_MAX_TEXT_ROWS - 1)
}

/// The composer draft.
///
/// It holds the units, its paste chips, and the full text held aside for each large
/// paste. A chip is one unit, so one backspace deletes the whole chip. The height is
/// bounded, so a tall draft scrolls inside the box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Composer {
    /// The draft, in send order, as one cell per unit. The order is the send order, so
    /// `model_text` expands each held paste in place.
    cells: Vec<Cell>,
    /// The cursor, as a unit index. Zero sits before the first unit.
    cursor: usize,
    /// The kill buffer, filled by a kill and restored by a yank.
    kill: Vec<Cell>,
}

/// One ordered cell of the draft. Each cell is exactly one unit.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Cell {
    /// One character the user typed inline.
    Char(char),
    /// One newline, which starts a display row.
    Newline,
    /// A large paste, shown as a chip but holding its full text aside.
    Paste { chip: PasteChip, text: String },
}

impl Cell {
    /// The unit this cell represents.
    fn unit(&self) -> Unit {
        match self {
            Cell::Char(ch) => Unit::Char(*ch),
            Cell::Newline => Unit::Newline,
            Cell::Paste { .. } => Unit::Paste,
        }
    }

    /// The text this cell sends to the model.
    fn model_text(&self) -> String {
        match self {
            Cell::Char(ch) => ch.to_string(),
            Cell::Newline => "\n".to_string(),
            Cell::Paste { text, .. } => text.clone(),
        }
    }

    /// The text this cell shows on screen.
    fn display_text(&self) -> String {
        match self {
            Cell::Char(ch) => ch.to_string(),
            Cell::Newline => String::new(),
            Cell::Paste { chip, .. } => paste_chip_label(chip),
        }
    }
}

/// True when the unit ends a word, so a word motion stops at it.
fn is_word_separator(unit: Unit) -> bool {
    match unit {
        Unit::Char(ch) => ch.is_whitespace(),
        Unit::Newline => true,
        Unit::Paste => false,
    }
}

impl Composer {
    /// Make a new, empty composer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Accept a pasted string.
    ///
    /// A paste at or above `LARGE_PASTE_CHARS` collapses to a chip. It holds the full
    /// text aside. It returns the chip, or `None` when the paste stays inline.
    pub fn paste(&mut self, text: &str) -> Option<PasteChip> {
        let chars = text.chars().count();
        if chars < LARGE_PASTE_CHARS {
            self.insert(text);
            return None;
        }
        // A second held paste of the same size takes the next repeat index, so two
        // same-size pastes stay distinct. This is codex's repeat suffix.
        let repeat = self
            .cells
            .iter()
            .filter(|cell| matches!(cell, Cell::Paste { chip, .. } if chip.chars == chars))
            .count() as u32
            + 1;
        let chip = PasteChip { chars, repeat };
        self.cells.insert(
            self.cursor,
            Cell::Paste {
                chip: chip.clone(),
                text: text.to_string(),
            },
        );
        self.cursor += 1;
        Some(chip)
    }

    /// Type printable text into the draft at the cursor.
    ///
    /// It splits the text into one cell per character. A newline becomes a newline unit.
    pub fn insert(&mut self, text: &str) {
        for ch in text.chars() {
            let cell = if ch == '\n' {
                Cell::Newline
            } else {
                Cell::Char(ch)
            };
            self.cells.insert(self.cursor, cell);
            self.cursor += 1;
        }
    }

    /// Return the text sent to the model on submit. It expands every held paste in place.
    pub fn model_text(&self) -> String {
        self.cells.iter().map(Cell::model_text).collect()
    }

    /// Delete one unit before the cursor. A chip deletes whole. It moves the cursor one
    /// unit left. It returns `true` when a chip was removed.
    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        let removed = self.cells.remove(self.cursor);
        matches!(removed, Cell::Paste { .. })
    }

    /// Return the count of paste chips held aside.
    pub fn chip_count(&self) -> usize {
        self.cells
            .iter()
            .filter(|cell| matches!(cell, Cell::Paste { .. }))
            .count()
    }

    /// Return the rendered height in rows. It caps at `COMPOSER_MAX_TEXT_ROWS` plus the
    /// two box borders.
    pub fn height_rows(&self) -> usize {
        // Count newline units, plus the first row.
        let lines = self
            .cells
            .iter()
            .filter(|cell| matches!(cell, Cell::Newline))
            .count()
            + 1;
        lines.min(COMPOSER_MAX_TEXT_ROWS) + 2
    }

    /// Return the units of the draft, in send order.
    pub fn units(&self) -> Vec<Unit> {
        self.cells.iter().map(Cell::unit).collect()
    }

    /// Return the cursor, as a unit index. Zero sits before the first unit.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Return the unit at index `at`.
    fn unit_at(&self, at: usize) -> Unit {
        self.cells[at].unit()
    }

    /// Move the cursor one unit left. It stops at the start of the draft.
    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move the cursor one unit right. It stops at the end of the draft.
    pub fn move_right(&mut self) {
        if self.cursor < self.cells.len() {
            self.cursor += 1;
        }
    }

    /// Move the cursor left to the start of the word. It skips separators first.
    pub fn move_word_left(&mut self) {
        while self.cursor > 0 && is_word_separator(self.unit_at(self.cursor - 1)) {
            self.cursor -= 1;
        }
        while self.cursor > 0 && !is_word_separator(self.unit_at(self.cursor - 1)) {
            self.cursor -= 1;
        }
    }

    /// Move the cursor right to the end of the word. It skips separators first.
    pub fn move_word_right(&mut self) {
        let len = self.cells.len();
        while self.cursor < len && is_word_separator(self.unit_at(self.cursor)) {
            self.cursor += 1;
        }
        while self.cursor < len && !is_word_separator(self.unit_at(self.cursor)) {
            self.cursor += 1;
        }
    }

    /// Move the cursor to the start of the current row.
    pub fn move_line_start(&mut self) {
        while self.cursor > 0 && self.unit_at(self.cursor - 1) != Unit::Newline {
            self.cursor -= 1;
        }
    }

    /// Move the cursor to the end of the current row.
    pub fn move_line_end(&mut self) {
        while self.cursor < self.cells.len() && self.unit_at(self.cursor) != Unit::Newline {
            self.cursor += 1;
        }
    }

    /// Move the cursor one display row up. It returns `false` when no row is above.
    pub fn move_row_up(&mut self, width: usize) -> bool {
        let (_, positions) = self.layout(width);
        let (row, col) = positions[self.cursor];
        if row == 0 {
            return false;
        }
        self.cursor = pick_index_on_row(&positions, row - 1, col);
        true
    }

    /// Move the cursor one display row down. It returns `false` when no row is below.
    pub fn move_row_down(&mut self, width: usize) -> bool {
        let (lines, positions) = self.layout(width);
        let (row, col) = positions[self.cursor];
        if row + 1 >= lines.len() {
            return false;
        }
        self.cursor = pick_index_on_row(&positions, row + 1, col);
        true
    }

    /// Insert a newline at the cursor.
    pub fn insert_newline(&mut self) {
        self.cells.insert(self.cursor, Cell::Newline);
        self.cursor += 1;
    }

    /// Delete the unit after the cursor. A chip deletes whole. At the end it does nothing.
    pub fn delete_forward(&mut self) {
        if self.cursor < self.cells.len() {
            self.cells.remove(self.cursor);
        }
    }

    /// Cut from the cursor to the row end into the kill buffer.
    pub fn kill_to_line_end(&mut self) {
        let mut end = self.cursor;
        while end < self.cells.len() && self.unit_at(end) != Unit::Newline {
            end += 1;
        }
        self.kill = self.cells.drain(self.cursor..end).collect();
    }

    /// Cut from the row start to the cursor into the kill buffer.
    pub fn kill_to_line_start(&mut self) {
        let mut start = self.cursor;
        while start > 0 && self.unit_at(start - 1) != Unit::Newline {
            start -= 1;
        }
        self.kill = self.cells.drain(start..self.cursor).collect();
        self.cursor = start;
    }

    /// Cut one word to the left into the kill buffer.
    pub fn kill_word_left(&mut self) {
        let mut start = self.cursor;
        while start > 0 && is_word_separator(self.cells[start - 1].unit()) {
            start -= 1;
        }
        while start > 0 && !is_word_separator(self.cells[start - 1].unit()) {
            start -= 1;
        }
        self.kill = self.cells.drain(start..self.cursor).collect();
        self.cursor = start;
    }

    /// Paste the kill buffer at the cursor.
    pub fn yank(&mut self) {
        let buffer = self.kill.clone();
        let count = buffer.len();
        for (offset, cell) in buffer.into_iter().enumerate() {
            self.cells.insert(self.cursor + offset, cell);
        }
        self.cursor += count;
    }

    /// Return the display rows at this width. It wraps by columns and caps by
    /// `COMPOSER_MAX_TEXT_ROWS`.
    pub fn display_lines(&self, width: usize) -> Vec<String> {
        let (lines, positions) = self.layout(width);
        let start = window_start(&lines, &positions, self.cursor);
        lines
            .into_iter()
            .skip(start)
            .take(COMPOSER_MAX_TEXT_ROWS)
            .collect()
    }

    /// Return the cursor cell, as a row and a column into `display_lines`.
    ///
    /// The row is relative to the drawn window, so the cursor is always on screen. A
    /// composer that reported an uncapped row drew the cursor outside its own box.
    pub fn cursor_cell(&self, width: usize) -> (usize, usize) {
        let (lines, positions) = self.layout(width);
        let start = window_start(&lines, &positions, self.cursor);
        let (row, column) = positions[self.cursor];
        (row.saturating_sub(start), column)
    }

    /// Return true when the draft holds no unit.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Replace the whole draft, and put the cursor at the end. It drops every held
    /// paste. It keeps the kill buffer.
    pub fn set_text(&mut self, text: &str) {
        self.cells.clear();
        self.cursor = 0;
        self.insert(text);
    }

    /// Return the draft as one string, with a chip label for each held paste.
    ///
    /// It keeps every newline. So the user sees the draft the same way the composer
    /// draws it, but on one string and with no wrap.
    pub fn display_string(&self) -> String {
        let mut out = String::new();
        for cell in &self.cells {
            match cell {
                Cell::Char(ch) => out.push(*ch),
                Cell::Newline => out.push('\n'),
                Cell::Paste { chip, .. } => out.push_str(&paste_chip_label(chip)),
            }
        }
        out
    }

    /// Take the model text, and empty the draft. The cursor returns to zero.
    pub fn take(&mut self) -> String {
        let text = self.model_text();
        self.cells.clear();
        self.cursor = 0;
        text
    }

    /// Lay out the draft into wrapped rows and one cursor position per unit index.
    ///
    /// The rows and the positions come from one pass, so `cursor_cell` and
    /// `display_lines` agree about each wrap. A wide glyph takes two columns.
    fn layout(&self, width: usize) -> (Vec<String>, Vec<(usize, usize)>) {
        let mut lines = vec![String::new()];
        let mut col = 0usize;
        let mut positions = Vec::with_capacity(self.cells.len() + 1);
        for cell in &self.cells {
            positions.push((lines.len() - 1, col));
            match cell {
                Cell::Newline => {
                    lines.push(String::new());
                    col = 0;
                }
                _ => {
                    let text = cell.display_text();
                    let cell_width = text.width();
                    if width > 0 && col > 0 && col + cell_width > width {
                        lines.push(String::new());
                        col = 0;
                    }
                    lines
                        .last_mut()
                        .expect("a row is always present")
                        .push_str(&text);
                    col += cell_width;
                }
            }
        }
        positions.push((lines.len() - 1, col));
        (lines, positions)
    }
}

/// Pick the unit index on `target_row` that best matches column `col`.
///
/// It prefers the last index at or before the column. It falls back to the first index
/// on the row. The caller checks that the row exists first.
fn pick_index_on_row(positions: &[(usize, usize)], target_row: usize, col: usize) -> usize {
    let mut best: Option<usize> = None;
    for (index, &(row, position_col)) in positions.iter().enumerate() {
        if row != target_row {
            continue;
        }
        match best {
            None => best = Some(index),
            Some(current) => {
                let (_, current_col) = positions[current];
                let better = if position_col <= col {
                    current_col > col || position_col > current_col
                } else {
                    current_col > col && position_col < current_col
                };
                if better {
                    best = Some(index);
                }
            }
        }
    }
    best.expect("the caller checks the row exists")
}
