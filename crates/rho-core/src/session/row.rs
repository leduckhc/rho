//! One row of a session list, built from a bounded head read and a bounded tail read.
//!
//! See `SPEC-session-store-wiring` sections 5 and 5a, and `D-a-bad-session-file-is-one-row`.
//!
//! **A full decode is too slow, and the measurement says so.** `ADR-jsonl-codec` measured a
//! typed decode of one 1848-record session at 2.01 milliseconds. Five hundred of those cost
//! about one second, and the budget is 100 milliseconds. So a row never decodes a whole file.

use std::io::{BufRead, Seek, SeekFrom};
use std::path::PathBuf;

use crate::Usage;
use crate::session::{
    Entry, ForkOrigin, MAX_LINE_BYTES, Record, SessionError, SessionId, decode, parse_header,
    read_capped_line,
};

/// The lines read from the head of a file to build one row.
pub const ROW_HEAD_LINES: usize = 8;

/// The bytes read from the tail of a file to build one row.
pub const ROW_TAIL_BYTES: u64 = 64 * 1024;

/// The longest automatic title, in bytes. One line of the first prompt, cut here.
const TITLE_LIMIT: usize = 60;

/// One row of a session list.
///
/// A file rho cannot read is a row, never a failed list. A store gathers junk over a year: a
/// foreign `.jsonl`, a truncated header, or a file from a newer rho is enough. One bad byte
/// must not hide every good session. See `D-a-bad-session-file-is-one-row`.
#[derive(Clone, Debug)]
pub enum SessionRow {
    Session(Box<SessionSummary>),
    Unreadable { path: PathBuf, reason: String },
}

/// The summary of one session, for a list or a picker.
///
/// Every field here has a renderer or a test that reads it. A field that no reader wants is
/// dead surface, and this project has a defect class for that. Three fields were cut from the
/// first draft for exactly that reason: `provider`, `approval`, and `sandbox`. The header still
/// records all three, and a resume still reads them from the header.
///
/// **A row carries no turn count.** A count needs every record, so it needs a full decode. The
/// row reports tokens and cost instead, and both are exact, because a `Usage` record is
/// cumulative. rho shows no number it did not read.
#[derive(Clone, Debug)]
pub struct SessionSummary {
    pub id: SessionId,
    pub path: PathBuf,
    /// An explicit name when the file holds one, else the first line of the first prompt.
    pub title: String,
    /// True when a `Name` record set the title. False when it came from the first prompt.
    pub title_is_explicit: bool,
    pub cwd: PathBuf,
    /// The header timestamp, as epoch milliseconds.
    pub started_millis: u64,
    /// The file modification time, as epoch milliseconds. It costs no read.
    pub last_active_millis: u64,
    pub size_bytes: u64,
    /// The model the file states on its second line.
    pub model: String,
    /// The last cumulative usage record. `None` when the session ran no turn.
    pub usage: Option<Usage>,
    /// True when the last record is `Closed`.
    pub closed: bool,
    /// Set when this file came from a fork. It is shown, and it is never trusted.
    pub forked_from: Option<ForkOrigin>,
}

/// The facts about a file that a byte source cannot carry.
///
/// `display_path` is data. The row builder never opens it, and a test proves that by passing a
/// path that does not exist.
#[derive(Clone, Debug)]
pub struct RowMeta {
    pub display_path: PathBuf,
    pub size_bytes: u64,
    pub last_active_millis: u64,
}

/// Build one row from one open source.
///
/// This is the seam the budget test drives. A first draft passed both a counting source **and**
/// a path, so an implementation could ignore the source, open the path, and read the whole
/// file. The counting source would then report almost nothing and the byte assertion would
/// pass. That is the memory-cap defect of `D-bash-line-cap`, rebuilt by the very fix meant to
/// prevent it. So the builder takes no openable path.
///
/// Two bounded reads:
///
/// - **The head.** The first `ROW_HEAD_LINES` lines. It gives the header, the model, and the
///   first user message.
/// - **The tail.** The last `ROW_TAIL_BYTES` bytes. It gives the newest name, the last
///   cumulative usage, and whether the file closed. A tail read can start inside a line, and
///   the first partial line is dropped, always.
pub fn row_from<R: BufRead + Seek>(mut source: R, meta: RowMeta) -> SessionRow {
    let unreadable = |reason: String| SessionRow::Unreadable {
        path: meta.display_path.clone(),
        reason,
    };

    // ---- the head ----
    let mut head_lines: Vec<String> = Vec::with_capacity(ROW_HEAD_LINES);
    let mut buf = Vec::new();
    for _ in 0..ROW_HEAD_LINES {
        match read_capped_line(&mut source, &mut buf) {
            Ok(true) => {
                let line = String::from_utf8_lossy(&buf);
                head_lines.push(line.trim_end_matches(['\n', '\r']).to_string());
            }
            Ok(false) => break,
            Err(error) => return unreadable(error.to_string()),
        }
    }
    let Some(first) = head_lines.first() else {
        return unreadable("the session file is empty".to_string());
    };
    let (_header_id, header) = match parse_header(first) {
        Ok(pair) => pair,
        Err(error) => return unreadable(error.to_string()),
    };
    // The id comes from the header when the file states one, else from the file name, which is
    // where the id used to live.
    let id = if header.session_id.is_empty() {
        stem_id(&meta.display_path)
    } else {
        SessionId::parse(&header.session_id).ok()
    };
    let Some(id) = id else {
        return unreadable(format!(
            "the file states no session id, and {} is not one",
            meta.display_path.display()
        ));
    };
    let started_millis = header_millis(first);

    let mut model = String::new();
    let mut title = String::new();
    let mut title_is_explicit = false;
    for line in head_lines.iter().skip(1) {
        match decode::<Entry>(line) {
            Ok(entry) => match entry.record {
                Record::ModelChange {
                    model: named,
                    provider: _,
                } => model = named,
                Record::Name { title: named } => {
                    title = named;
                    title_is_explicit = true;
                }
                Record::Message { message } if title.is_empty() => {
                    if let Some(text) = first_text(&message) {
                        title = automatic_title(&text);
                    }
                }
                _ => {}
            },
            // A line that does not decode in the head is not a reason to fail a row. The head
            // is a best effort read, and the header is the only line that must parse.
            Err(_) => continue,
        }
    }

    // ---- the tail ----
    let tail_start = meta.size_bytes.saturating_sub(ROW_TAIL_BYTES);
    let mut usage = None;
    let mut closed = false;
    if let Err(error) = source.seek(SeekFrom::Start(tail_start)) {
        return unreadable(error.to_string());
    }
    // **A tail that starts inside a line yields no broken record.** There is no separate skip
    // of the partial first line, and that is deliberate. A partial line cannot decode into an
    // `Entry`, so the loop below drops it already.
    //
    // A first version skipped it explicitly as well. A mutation then showed that deleting
    // either guard changed nothing a test could see, so each masked the other. This project
    // met the same redundant-guard trap in `record_fits` and in `ProviderState::for_owner`,
    // and the answer is the same: keep one mechanism. `a_tail_read_drops_a_partial_first_line`
    // pins the outcome.
    loop {
        match read_capped_line(&mut source, &mut buf) {
            Ok(true) => {}
            Ok(false) => break,
            Err(_) => break,
        }
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim_end_matches(['\n', '\r']);
        if line.is_empty() {
            continue;
        }
        match decode::<Entry>(line) {
            Ok(entry) => {
                // Only a whole decoded record decides the close flag, so half a `Closed` line
                // left by a crash never reads as a close.
                closed = matches!(entry.record, Record::Closed);
                match entry.record {
                    Record::Usage { usage: seen } => usage = Some(seen),
                    Record::Name { title: named } => {
                        title = named;
                        title_is_explicit = true;
                    }
                    _ => {}
                }
            }
            Err(_) => continue,
        }
    }

    finish(
        id,
        meta,
        header.cwd,
        header.forked_from,
        title,
        title_is_explicit,
        model,
        started_millis,
        usage,
        closed,
    )
}

/// Assemble the row. It exists so the two early returns above cannot forget a field.
#[allow(clippy::too_many_arguments)]
fn finish(
    id: SessionId,
    meta: RowMeta,
    cwd: PathBuf,
    forked_from: Option<ForkOrigin>,
    title: String,
    title_is_explicit: bool,
    model: String,
    started_millis: u64,
    usage: Option<Usage>,
    closed: bool,
) -> SessionRow {
    SessionRow::Session(Box::new(SessionSummary {
        id,
        path: meta.display_path,
        title,
        title_is_explicit,
        cwd,
        started_millis,
        last_active_millis: meta.last_active_millis,
        size_bytes: meta.size_bytes,
        model,
        usage,
        closed,
        forked_from,
    }))
}

/// The session id in a file name, when the name is one.
fn stem_id(path: &std::path::Path) -> Option<SessionId> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| SessionId::parse(stem).ok())
}

/// The header timestamp, as epoch milliseconds.
///
/// The timestamp is a shared field of `Entry`, not part of the header record, so this reads it
/// from the same line the header came from. A line that states no number gives zero, and a row
/// then shows no start time rather than a wrong one.
fn header_millis(line: &str) -> u64 {
    decode::<Entry>(line)
        .ok()
        .and_then(|entry| entry.timestamp.parse::<u64>().ok())
        .unwrap_or(0)
}

/// The first text block of a message.
fn first_text(message: &crate::Message) -> Option<String> {
    message.content.iter().find_map(|block| match block {
        crate::ContentBlock::Text { text } => Some(text.clone()),
        _ => None,
    })
}

/// One line of the first prompt, cut at `TITLE_LIMIT` bytes on a character boundary.
fn automatic_title(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    let mut end = TITLE_LIMIT.min(line.len());
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    line[..end].to_string()
}

/// The largest line a row read accepts. It shares the reader's cap, so a crafted file cannot
/// make a list allocate without bound.
const _: () = assert!(MAX_LINE_BYTES > ROW_TAIL_BYTES as usize);

/// Turn a read failure into one row, so a caller never loses a whole list.
pub(crate) fn unreadable_row(path: PathBuf, error: &SessionError) -> SessionRow {
    SessionRow::Unreadable {
        path,
        reason: error.to_string(),
    }
}
