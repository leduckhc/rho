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
    let _ = chip;
    todo!("paste_chip_label is unimplemented in the red stage")
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
    let _ = chip;
    todo!("image_chip_label is unimplemented in the red stage")
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
    let _ = (session_root, path, bytes);
    todo!("attach_image is unimplemented in the red stage")
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
    let _ = keys;
    todo!("route_burst is unimplemented in the red stage")
}

/// The composer draft.
///
/// It holds the visible text, its paste chips, and the full text held aside for each
/// large paste. A chip is one unit, so one backspace deletes the whole chip. The
/// height is bounded, so a tall draft scrolls inside the box.
#[derive(Clone, Debug, Default)]
pub struct Composer {
    _private: (),
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
        let _ = text;
        todo!("Composer::paste is unimplemented in the red stage")
    }

    /// Type printable text into the draft at the cursor.
    pub fn insert(&mut self, text: &str) {
        let _ = text;
        todo!("Composer::insert is unimplemented in the red stage")
    }

    /// The text sent to the model on submit. Every held paste is expanded in place.
    pub fn model_text(&self) -> String {
        todo!("Composer::model_text is unimplemented in the red stage")
    }

    /// Delete one unit at the cursor. A chip deletes whole. Return `true` when a chip
    /// was removed.
    pub fn backspace(&mut self) -> bool {
        todo!("Composer::backspace is unimplemented in the red stage")
    }

    /// The count of paste chips held aside.
    pub fn chip_count(&self) -> usize {
        todo!("Composer::chip_count is unimplemented in the red stage")
    }

    /// The rendered height in rows, capped at `COMPOSER_MAX_TEXT_ROWS` plus the two
    /// box borders.
    pub fn height_rows(&self) -> usize {
        todo!("Composer::height_rows is unimplemented in the red stage")
    }
}
