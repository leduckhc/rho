//! Paste collapsing and attachments.
//!
//! A large paste collapses to one chip, and the full text is held aside so it still
//! reaches the model on send. A paste burst on a terminal with no bracketed paste
//! flushes through the paste path, so a pasted `?` never opens the help. An image
//! attachment becomes a bounded chip, capped by size and confined to the session
//! root. See `SPEC-tui-experience` sections 5 and 6.

use std::path::Path;

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

/// The composer draft.
///
/// It holds the visible text, its paste chips, and the full text held aside for each
/// large paste. A chip is one unit, so one backspace deletes the whole chip. The
/// height is bounded, so a tall draft scrolls inside the box.
#[derive(Clone, Debug, Default)]
pub struct Composer {
    /// The draft, in order, as typed text and held pastes. The order is the send
    /// order, so `model_text` expands each held paste in place.
    segments: Vec<Segment>,
}

/// One ordered piece of the draft.
#[derive(Clone, Debug)]
enum Segment {
    /// Text the user typed inline.
    Text(String),
    /// A large paste, shown as a chip but holding its full text aside.
    Paste { chip: PasteChip, text: String },
}

impl Composer {
    /// A new, empty composer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Accept a pasted string.
    ///
    /// A paste at or above `LARGE_PASTE_CHARS` collapses to a chip, and the full text
    /// is held aside. Return the chip, or `None` when the paste stays inline.
    pub fn paste(&mut self, text: &str) -> Option<PasteChip> {
        let chars = text.chars().count();
        if chars < LARGE_PASTE_CHARS {
            self.insert(text);
            return None;
        }
        // A second held paste of the same size takes the next repeat index, so two
        // same-size pastes stay distinct. This is codex's repeat suffix.
        let repeat = self
            .segments
            .iter()
            .filter(|segment| matches!(segment, Segment::Paste { chip, .. } if chip.chars == chars))
            .count() as u32
            + 1;
        let chip = PasteChip { chars, repeat };
        self.segments.push(Segment::Paste {
            chip: chip.clone(),
            text: text.to_string(),
        });
        Some(chip)
    }

    /// Type printable text into the draft at the cursor.
    pub fn insert(&mut self, text: &str) {
        if let Some(Segment::Text(existing)) = self.segments.last_mut() {
            existing.push_str(text);
        } else {
            self.segments.push(Segment::Text(text.to_string()));
        }
    }

    /// The text sent to the model on submit. Every held paste is expanded in place.
    pub fn model_text(&self) -> String {
        self.segments
            .iter()
            .map(|segment| match segment {
                Segment::Text(text) => text.as_str(),
                Segment::Paste { text, .. } => text.as_str(),
            })
            .collect()
    }

    /// Delete one unit at the cursor. A chip deletes whole. Return `true` when a chip
    /// was removed.
    pub fn backspace(&mut self) -> bool {
        match self.segments.last_mut() {
            None => false,
            Some(Segment::Paste { .. }) => {
                self.segments.pop();
                true
            }
            Some(Segment::Text(text)) => {
                text.pop();
                if text.is_empty() {
                    self.segments.pop();
                }
                false
            }
        }
    }

    /// The count of paste chips held aside.
    pub fn chip_count(&self) -> usize {
        self.segments
            .iter()
            .filter(|segment| matches!(segment, Segment::Paste { .. }))
            .count()
    }

    /// The rendered height in rows, capped at `COMPOSER_MAX_TEXT_ROWS` plus the two
    /// box borders.
    pub fn height_rows(&self) -> usize {
        let display: String = self
            .segments
            .iter()
            .map(|segment| match segment {
                Segment::Text(text) => text.clone(),
                Segment::Paste { chip, .. } => paste_chip_label(chip),
            })
            .collect();
        // At least one text row, even for an empty draft.
        let lines = display.split('\n').count().max(1);
        lines.min(COMPOSER_MAX_TEXT_ROWS) + 2
    }
}
