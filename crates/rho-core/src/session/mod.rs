//! The session log. An append-only JSONL record of one conversation.
//!
//! See `SPEC-14`, `ADR-004`, and `ADR-005`. This module holds the record set, the
//! codec seam, the writer, the reader, the store, and the event recorder.
//!
//! Stage T4 defines the public surface with `todo!()` bodies. Stage T5 makes it real.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{AgentEvent, AgentStopReason, ContentBlock, Message, Usage};

// ---------------------------------------------------------------------------
// Section 2. The record set.
// ---------------------------------------------------------------------------

/// A record id. A short, unique string, minted per record.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RecordId(pub String);

/// One line of the session file. The shared fields sit beside the tagged body, so
/// the on-disk shape is `{ "type": ..., "id": ..., "parentId": ..., "timestamp": ..., ... }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: RecordId,
    #[serde(rename = "parentId")]
    pub parent_id: Option<RecordId>,
    /// An RFC 3339 timestamp, for example `2026-06-25T22:17:00.785Z`.
    pub timestamp: String,
    #[serde(flatten)]
    pub record: Record,
}

/// The body of one record. The `type` tag selects the variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Record {
    /// The header. The first record. One per file. `approval` and `sandbox` name
    /// the resolved modes the session ran under. See section 8a.
    Session {
        version: u32,
        cwd: PathBuf,
        approval: String,
        sandbox: String,
    },
    /// The provider or the model changed.
    ModelChange { provider: String, model: String },
    /// One conversation message. It carries the redacted content.
    Message { message: Message },
    /// Cumulative usage after a turn.
    Usage { usage: Usage },
    /// The run stopped, with a reason.
    Stop { stop_reason: AgentStopReason },
    /// The session closed cleanly. The last record of a closed file.
    Closed,
}

/// The largest single record written to the file, in bytes.
pub const MAX_RECORD_BYTES: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// Section 3. The codec seam.
// ---------------------------------------------------------------------------

/// Encode one record to a single JSONL line. It adds no trailing newline.
pub fn encode<T: Serialize>(value: &T) -> Result<String, SessionError> {
    let _ = value;
    todo!("stage T5 implements the codec")
}

/// Decode one JSONL line to a record.
pub fn decode<T: DeserializeOwned>(line: &str) -> Result<T, SessionError> {
    let _ = line;
    todo!("stage T5 implements the codec")
}

// ---------------------------------------------------------------------------
// Section 4. The writer, the reader, and the store.
// ---------------------------------------------------------------------------

/// The session header, without the shared record fields.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionHeader {
    pub version: u32,
    pub session_id: String,
    pub cwd: PathBuf,
    /// The resolved approval mode name. One of `read-only`, `ask`, `allow-all`.
    pub approval: String,
    /// The resolved sandbox mode name. One of `off`, `confined`, `strict`.
    pub sandbox: String,
}

/// The largest single line a reader accepts, in bytes. A longer line is a decode
/// error, never an allocation. See section 6a.
pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

/// A typed session error.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("session io error: {0}")]
    Io(String),
    #[error("cannot encode a record: {0}")]
    Encode(String),
    #[error("cannot decode a record: {0}")]
    Decode(String),
    #[error("unsupported session version {0}")]
    Version(u32),
    /// A resume would widen a permission, and the user did not allow it.
    #[error(
        "a resume would widen {field} from {stored} to {requested}; pass --allow-widen to allow it"
    )]
    Widen {
        field: &'static str,
        stored: String,
        requested: String,
    },
}

/// The approval modes, ordered from strict to permissive.
///
/// The order is the whole point. It makes a widening resume representable as a
/// refusal. See `SPEC-14` section 8a.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StoredApproval {
    ReadOnly,
    Ask,
    AllowAll,
}

/// The sandbox modes, ordered from strict to permissive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StoredSandbox {
    Strict,
    Confined,
    Off,
}

impl StoredApproval {
    /// Parse a stored name. An unknown name is the strictest mode, never the loosest.
    pub fn parse(_name: &str) -> Self {
        todo!("stage T5 implements the stored approval parse")
    }
}

impl StoredSandbox {
    /// Parse a stored name. An unknown name is the strictest mode, never the loosest.
    pub fn parse(_name: &str) -> Self {
        todo!("stage T5 implements the stored sandbox parse")
    }
}

/// Compare the stored modes in a header against the modes this run would use.
///
/// Return `Ok(())` when the run keeps or narrows both modes. Return
/// `SessionError::Widen` when the run would widen either mode and `allow_widen` is
/// false. Return `Ok(())` for a wider run when `allow_widen` is true, because the user
/// asked for it on purpose.
pub fn check_resume_permission(
    _header: &SessionHeader,
    _approval: StoredApproval,
    _sandbox: StoredSandbox,
    _allow_widen: bool,
) -> Result<(), SessionError> {
    todo!("stage T5 implements the resume permission check")
}

/// Appends records to one session file. It owns the open file handle.
#[allow(dead_code)]
pub struct SessionWriter {
    path: PathBuf,
    head: Option<RecordId>,
    closed: bool,
}

impl SessionWriter {
    /// Append one record. Mint an id. Link the parent. Stamp the time. Write one
    /// line. Flush. Return the new id. Never rewrite an earlier byte.
    pub fn append(
        &mut self,
        record: Record,
        parent: Option<RecordId>,
    ) -> Result<RecordId, SessionError> {
        let _ = (record, parent);
        todo!("stage T5 implements the writer")
    }

    /// The id of the last record written. A later append links to it by default.
    pub fn head(&self) -> Option<RecordId> {
        todo!("stage T5 implements the writer")
    }

    /// The file path.
    pub fn path(&self) -> &Path {
        todo!("stage T5 implements the writer")
    }

    /// Write the `Closed` record. Idempotent. A second call writes nothing.
    pub fn close(&mut self) -> Result<(), SessionError> {
        todo!("stage T5 implements the writer")
    }
}

/// What one read produced.
#[derive(Clone, Debug)]
pub struct ReadResult {
    pub header: SessionHeader,
    pub entries: Vec<Entry>,
    /// True when the last line was partial and dropped. Resume warns on this.
    pub truncated_tail: bool,
}

/// A cheap summary for a list. It reads only the first line of a file.
#[derive(Clone, Debug)]
pub struct SessionSummary {
    pub session_id: String,
    pub path: PathBuf,
    pub cwd: PathBuf,
    pub size_bytes: u64,
}

/// Reads a session file into records.
pub struct SessionReader;

impl SessionReader {
    /// Read every whole record. Drop only a truncated last line. See section 6.
    /// Cap one line at `MAX_LINE_BYTES`. A longer line is a `SessionError::Decode`,
    /// never an unbounded allocation. See section 6a.
    pub fn read(path: &Path) -> Result<ReadResult, SessionError> {
        let _ = path;
        todo!("stage T5 implements the reader")
    }
}

/// The set of session files under one directory.
#[allow(dead_code)]
pub struct SessionStore {
    root: PathBuf,
}

impl SessionStore {
    /// A store rooted at a directory. The directory holds one file per session.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let _ = root.into();
        todo!("stage T5 implements the store")
    }

    /// Create a new session file. Write the header. Return a writer. This is the
    /// open path and the new path. `approval` and `sandbox` name the resolved
    /// modes, and go into the header record.
    pub fn create(
        &self,
        session_id: &str,
        cwd: &Path,
        approval: &str,
        sandbox: &str,
    ) -> Result<SessionWriter, SessionError> {
        let _ = (session_id, cwd, approval, sandbox);
        todo!("stage T5 implements the store")
    }

    /// Open an existing file to append more records. Used by resume and fork.
    pub fn append_to(&self, path: &Path) -> Result<SessionWriter, SessionError> {
        let _ = path;
        todo!("stage T5 implements the store")
    }

    /// List sessions with a cheap summary. Read only the first line of each file.
    /// The first line obeys the same `MAX_LINE_BYTES` cap as `read`.
    pub fn list(&self) -> Result<Vec<SessionSummary>, SessionError> {
        todo!("stage T5 implements the store")
    }

    /// Delete one session file. Every branch in the file goes with it.
    pub fn delete(&self, session_id: &str) -> Result<(), SessionError> {
        let _ = session_id;
        todo!("stage T5 implements the store")
    }

    /// Fork a session at `from`. Copy the branch that ends at `from` into a new
    /// file with `new_id`. The original file is not changed. Return a writer on
    /// the new file.
    pub fn fork(
        &self,
        from_path: &Path,
        from: &RecordId,
        new_id: &str,
    ) -> Result<SessionWriter, SessionError> {
        let _ = (from_path, from, new_id);
        todo!("stage T5 implements the store")
    }
}

/// Rebuild the message list along the branch that ends at `head`. Walk parent
/// links from `head` to the root. Reverse the walk. Return the messages in order.
pub fn branch_messages(entries: &[Entry], head: &RecordId) -> Vec<Message> {
    let _ = (entries, head);
    todo!("stage T5 implements branch_messages")
}

// ---------------------------------------------------------------------------
// Section 5. How a session opts in or out.
// ---------------------------------------------------------------------------

/// A session's persistence. `Off` writes nothing. `File` appends to a writer.
pub enum SessionLog {
    Off,
    File(SessionWriter),
}

impl SessionLog {
    /// Append one record. On a write failure, degrade to ephemeral with a warning.
    /// Return the id when written. Return `None` when ephemeral or degraded.
    pub fn record(&mut self, record: Record, parent: Option<RecordId>) -> Option<RecordId> {
        let _ = (record, parent);
        todo!("stage T5 implements the log")
    }

    /// True when this log writes nothing.
    pub fn is_ephemeral(&self) -> bool {
        todo!("stage T5 implements the log")
    }
}

/// Folds the agent event stream into session records.
#[allow(dead_code)]
pub struct SessionRecorder {
    log: SessionLog,
}

impl SessionRecorder {
    /// Build a recorder over a log. `SessionLog::Off` gives an ephemeral recorder.
    pub fn new(log: SessionLog) -> Self {
        let _ = log;
        todo!("stage T5 implements the recorder")
    }

    /// Record the user's prompt as a `Message` record. Redact the content first.
    pub fn record_prompt(&mut self, input: &[ContentBlock]) -> Option<RecordId> {
        let _ = input;
        todo!("stage T5 implements the recorder")
    }

    /// Fold one agent event. Write an assistant message at a turn end, a tool
    /// result at a tool end, a usage record on a usage event, and a stop record at
    /// the agent end. Redact every tool argument first. Return an id when it writes.
    pub fn observe(&mut self, event: &AgentEvent) -> Option<RecordId> {
        let _ = event;
        todo!("stage T5 implements the recorder")
    }

    /// On a cancel, complete any open tool pairing, then write the stop record.
    /// See section 8.
    pub fn record_cancel(&mut self) -> Option<RecordId> {
        todo!("stage T5 implements the recorder")
    }

    /// True when the log is ephemeral, or degraded to ephemeral.
    pub fn is_ephemeral(&self) -> bool {
        todo!("stage T5 implements the recorder")
    }
}
