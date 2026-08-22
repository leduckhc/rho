//! The session log. An append-only JSONL record of one conversation.
//!
//! See `SPEC-sessions`, `ADR-session-format`, and `ADR-jsonl-codec`. This module holds the record set, the
//! codec seam, the writer, the reader, the store, and the event recorder.
//!
//! Stage T4 defined the public surface. Stage T5 made every body real.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{AgentEvent, AgentStopReason, ContentBlock, Message, Role, StreamEvent, Usage};

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
    /// The record time, as epoch milliseconds in a decimal string, for example
    /// `1750890000785`. One session file holds this one format on every line, so a
    /// consumer never guesses per line. The pi import converts a pi RFC 3339 timestamp
    /// into this format, so an imported file matches a native file.
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
    Stop {
        #[serde(rename = "reason")]
        stop_reason: AgentStopReason,
    },
    /// The session closed cleanly. The last record of a closed file, unless a resume
    /// reopened it. Then a `Reopened` record follows, and nothing else may.
    Closed,
    /// A resume reopened a closed session.
    ///
    /// A closed file ends with `Closed`. A resume may still append, because a user may
    /// continue a conversation they closed. So the reopen is stated on disk. Without this
    /// record a reader would find `Closed` in the middle of a file, and it could not tell
    /// a closed session from one that kept talking. See `SPEC-sessions` section 8.
    Reopened,
}

/// The largest single record written to the file, in bytes.
pub const MAX_RECORD_BYTES: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// Section 3. The codec seam.
// ---------------------------------------------------------------------------

/// Encode one record to a single JSONL line. It adds no trailing newline.
pub fn encode<T: Serialize>(value: &T) -> Result<String, SessionError> {
    #[cfg(not(feature = "fast-json"))]
    {
        serde_json::to_string(value).map_err(|e| SessionError::Encode(e.to_string()))
    }
    #[cfg(feature = "fast-json")]
    {
        sonic_rs::to_string(value).map_err(|e| SessionError::Encode(e.to_string()))
    }
}

/// Decode one JSONL line to a record.
pub fn decode<T: DeserializeOwned>(line: &str) -> Result<T, SessionError> {
    #[cfg(not(feature = "fast-json"))]
    {
        serde_json::from_str(line).map_err(|e| SessionError::Decode(e.to_string()))
    }
    #[cfg(feature = "fast-json")]
    {
        sonic_rs::from_str(line).map_err(|e| SessionError::Decode(e.to_string()))
    }
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

/// How many records may fail to decode before a read gives up.
///
/// A bad record in the middle is skipped and counted, so a corrupt file no longer loses its
/// tail. A crafted file of nothing but bad lines would then buy a full pass, so the pass has a
/// ceiling. A security review asked for it. The number is generous: real corruption is a byte
/// or a line, not a thousand.
pub const MAX_DROPPED_RECORDS: usize = 1024;

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
/// refusal. See `SPEC-sessions` section 8a.
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
    pub fn parse(name: &str) -> Self {
        match name {
            "ask" => StoredApproval::Ask,
            "allow-all" => StoredApproval::AllowAll,
            // `read-only`, and every name this build does not know, is the strictest
            // mode. An unknown name must never widen a permission. See D-plugin-does-not-classify-itself.
            _ => StoredApproval::ReadOnly,
        }
    }

    /// The stored name of this mode.
    fn name(self) -> &'static str {
        match self {
            StoredApproval::ReadOnly => "read-only",
            StoredApproval::Ask => "ask",
            StoredApproval::AllowAll => "allow-all",
        }
    }
}

impl StoredSandbox {
    /// Parse a stored name. An unknown name is the strictest mode, never the loosest.
    pub fn parse(name: &str) -> Self {
        match name {
            "confined" => StoredSandbox::Confined,
            "off" => StoredSandbox::Off,
            // `strict`, and every unknown name, is the strictest mode. See D-plugin-does-not-classify-itself.
            _ => StoredSandbox::Strict,
        }
    }

    /// The stored name of this mode.
    fn name(self) -> &'static str {
        match self {
            StoredSandbox::Strict => "strict",
            StoredSandbox::Confined => "confined",
            StoredSandbox::Off => "off",
        }
    }
}

/// Compare the stored modes in a header against the modes this run would use.
///
/// Return `Ok(())` when the run keeps or narrows both modes. Return
/// `SessionError::Widen` when the run would widen either mode and `allow_widen` is
/// false. Return `Ok(())` for a wider run when `allow_widen` is true, because the user
/// asked for it on purpose.
pub fn check_resume_permission(
    header: &SessionHeader,
    approval: StoredApproval,
    sandbox: StoredSandbox,
    allow_widen: bool,
) -> Result<(), SessionError> {
    if allow_widen {
        return Ok(());
    }
    // A larger value is a more permissive mode, because both enums order from strict to
    // permissive. So a requested mode greater than the stored mode is a widen.
    let stored_approval = StoredApproval::parse(&header.approval);
    if approval > stored_approval {
        return Err(SessionError::Widen {
            field: "approval",
            stored: stored_approval.name().to_string(),
            requested: approval.name().to_string(),
        });
    }
    let stored_sandbox = StoredSandbox::parse(&header.sandbox);
    if sandbox > stored_sandbox {
        return Err(SessionError::Widen {
            field: "sandbox",
            stored: stored_sandbox.name().to_string(),
            requested: sandbox.name().to_string(),
        });
    }
    Ok(())
}

/// Appends records to one session file. It owns an open sink and mints ids.
///
/// The writer holds one sink for the life of the session, and writes one record as one
/// write. It never reopens the file and never rewrites an earlier byte. See `SPEC-sessions`
/// section 3. A held handle costs about 1 microsecond per record, against about 17.5
/// microseconds for a reopen per record, measured on macos arm64 over 20000 records.
///
/// The sink is a seam. `SessionStore` passes an open file. A test passes a sink that
/// fails on demand, which proves the degrade path without removing the session
/// directory. This mirrors `SessionReader::read_from`, the reader seam.
pub struct SessionWriter {
    path: PathBuf,
    sink: Box<dyn Write + Send>,
    head: Option<RecordId>,
    next_id: u64,
    closed: bool,
}

impl SessionWriter {
    /// Build a writer over any sink. The store passes an open file. A test passes a
    /// sink that fails on demand, to prove the degrade path. See `SPEC-sessions` section 3
    /// and decision D-write-failure-degrades.
    pub fn with_sink(path: impl Into<PathBuf>, sink: Box<dyn Write + Send>) -> Self {
        Self {
            path: path.into(),
            sink,
            head: None,
            next_id: 0,
            closed: false,
        }
    }

    /// Mint the next record id. The ids are unique within one file.
    fn mint_id(&mut self) -> RecordId {
        let id = RecordId(format!("r{}", self.next_id));
        self.next_id += 1;
        id
    }

    /// The sidecar path for one record, next to the session file.
    fn sidecar_path(&self, id: &RecordId) -> PathBuf {
        let stem = self
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "session".to_string());
        self.path.with_file_name(format!("{stem}.{}.sidecar", id.0))
    }

    /// Write one already-built entry as a single line. One record is one write, so the
    /// line and its newline go to the sink in one call. It appends and never rewrites.
    fn write_entry(&mut self, entry: &Entry) -> Result<(), SessionError> {
        let mut bytes = encode(entry)?.into_bytes();
        bytes.push(b'\n');
        self.sink
            .write_all(&bytes)
            .map_err(|e| io_error(self.path.as_path(), e))?;
        self.sink
            .flush()
            .map_err(|e| io_error(self.path.as_path(), e))?;
        Ok(())
    }

    /// Append one record. Mint an id. Link the parent. Stamp the time. Write one
    /// line. Flush. Return the new id. Never rewrite an earlier byte.
    pub fn append(
        &mut self,
        record: Record,
        parent: Option<RecordId>,
    ) -> Result<RecordId, SessionError> {
        let id = self.mint_id();
        let timestamp = now_timestamp();
        let entry = Entry {
            id: id.clone(),
            parent_id: parent.clone(),
            timestamp: timestamp.clone(),
            record,
        };
        let line = encode(&entry)?;
        let entry = if line.len() > MAX_RECORD_BYTES {
            // The record is over the cap. Cap the oversize string values, spill the full
            // payload to a sidecar, and rebuild the entry from the capped record. See
            // SPEC-sessions section 3 and D-cap-a-large-tool-result.
            let mut spills = Vec::new();
            let capped = cap_record(entry.record, &mut spills);
            if !spills.is_empty() {
                self.spill_to_sidecar(&id, &spills)?;
            }
            Entry {
                id: id.clone(),
                parent_id: parent,
                timestamp,
                record: capped,
            }
        } else {
            entry
        };
        self.write_entry(&entry)?;
        self.head = Some(id.clone());
        Ok(id)
    }

    /// Write the spilled payloads for one record to a sidecar file.
    fn spill_to_sidecar(&self, id: &RecordId, spills: &[String]) -> Result<(), SessionError> {
        let path = self.sidecar_path(id);
        let mut file = File::create(&path).map_err(|e| io_error(path.as_path(), e))?;
        for spill in spills {
            file.write_all(spill.as_bytes())
                .map_err(|e| io_error(path.as_path(), e))?;
            file.write_all(b"\n")
                .map_err(|e| io_error(path.as_path(), e))?;
        }
        file.flush().map_err(|e| io_error(path.as_path(), e))?;
        Ok(())
    }

    /// The id of the last record written. A later append links to it by default.
    pub fn head(&self) -> Option<RecordId> {
        self.head.clone()
    }

    /// The file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write the `Closed` record. Idempotent. A second call writes nothing.
    pub fn close(&mut self) -> Result<(), SessionError> {
        if self.closed {
            return Ok(());
        }
        let parent = self.head.clone();
        self.append(Record::Closed, parent)?;
        self.closed = true;
        Ok(())
    }
}

/// Build an io error that names the path that failed.
///
/// A bare os message says "No such file or directory" and never says which file. A user
/// with 500 sessions learns nothing from that. So every io error names its path, exactly
/// as `ConfigError::Read` does.
fn io_error(path: &Path, error: std::io::Error) -> SessionError {
    SessionError::Io(format!("{}: {error}", path.display()))
}

/// The session format version this build reads and writes.
const SESSION_FORMAT_VERSION: u32 = 1;

/// The largest byte length of one string value kept inline in a capped record. A longer
/// value spills to a sidecar and leaves a head plus a note in its place.
const STRING_HEAD_LIMIT: usize = 4 * 1024;

/// The record time, as epoch milliseconds in a decimal string. One session file holds
/// this one timestamp format on every line, so a consumer never guesses per line. The
/// pi import converts an RFC 3339 timestamp into this format. `rho-core` adds no date
/// dependency, because a date crate for one field is not worth the tree.
fn now_timestamp() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{millis}")
}

/// The largest byte length of any string value inside a JSON value.
fn max_json_string(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::String(s) => s.len(),
        serde_json::Value::Array(items) => items.iter().map(max_json_string).max().unwrap_or(0),
        serde_json::Value::Object(map) => map.values().map(max_json_string).max().unwrap_or(0),
        _ => 0,
    }
}

/// The head of a string, cut on a character boundary at or below `limit` bytes.
fn utf8_head(text: &str, limit: usize) -> &str {
    let mut end = limit.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Replace an oversize string with a head plus a note that states the full byte count.
fn cap_string_value(text: &str) -> String {
    format!(
        "{}\u{2026} [rho capped a value of {} bytes; the full payload is in a sidecar file]",
        utf8_head(text, STRING_HEAD_LIMIT),
        text.len()
    )
}

/// Cap oversize string values inside a JSON value, in place of the tree. The full value
/// is spilled by the caller, so this only shrinks the inline copy.
fn cap_json_strings(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) if s.len() > STRING_HEAD_LIMIT => {
            serde_json::Value::String(cap_string_value(&s))
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(cap_json_strings).collect())
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, cap_json_strings(v)))
                .collect(),
        ),
        other => other,
    }
}

/// Cap one content block. It keeps the block kind, so a `ToolCall` stays a `ToolCall`.
fn cap_block(block: ContentBlock, spills: &mut Vec<String>) -> ContentBlock {
    match block {
        ContentBlock::Text { text } if text.len() > STRING_HEAD_LIMIT => {
            let capped = cap_string_value(&text);
            spills.push(text);
            ContentBlock::Text { text: capped }
        }
        ContentBlock::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => ContentBlock::ToolResult {
            tool_call_id,
            content: content.into_iter().map(|b| cap_block(b, spills)).collect(),
            is_error,
        },
        ContentBlock::ToolCall {
            id,
            name,
            arguments,
            state,
        } if max_json_string(&arguments) > STRING_HEAD_LIMIT => {
            // A `ToolCall` is never rewritten to a text note. Keep the id and the name,
            // spill the full arguments, and cap the oversize string values inside them.
            spills.push(arguments.to_string());
            ContentBlock::ToolCall {
                id,
                name,
                arguments: cap_json_strings(arguments),
                state: cap_state(state),
            }
        }
        ContentBlock::ToolCall {
            id,
            name,
            arguments,
            state,
        } => ContentBlock::ToolCall {
            id,
            name,
            arguments,
            state: cap_state(state),
        },
        // Reasoning text is capped exactly as assistant text is. A named arm, because a
        // wildcard here let a new block join the file uncapped. See rule 10.
        ContentBlock::ReasoningTrace { text } if text.len() > STRING_HEAD_LIMIT => {
            let capped = cap_string_value(&text);
            spills.push(text);
            ContentBlock::ReasoningTrace { text: capped }
        }
        ContentBlock::ReasoningTrace { text } => ContentBlock::ReasoningTrace { text },
        ContentBlock::ReasoningReplay { text, state } => {
            let state = cap_state(state);
            if text.len() > STRING_HEAD_LIMIT {
                let capped = cap_string_value(&text);
                spills.push(text);
                ContentBlock::ReasoningReplay {
                    text: capped,
                    state,
                }
            } else {
                ContentBlock::ReasoningReplay { text, state }
            }
        }
        other => other,
    }
}

/// Bound one record read from a file, with the **same** caps the write path applies.
///
/// A security review found the first version of this bounded the payload and not the text, so
/// a crafted file could carry a megabyte of reasoning text and re-upload it on every turn:
/// the very cost the read-side bound was added to stop. One field bounded on one side only is
/// the same defect wearing a different field name, so read and write now share `cap_block`.
///
/// The spilled text is dropped rather than written to a sidecar. A sidecar belongs to a record
/// rho wrote, and this record came from somewhere else.
fn cap_entry_state(mut entry: Entry) -> Entry {
    if let Record::Message { message } = &mut entry.record {
        let content = std::mem::take(&mut message.content);
        let mut dropped = Vec::new();
        message.content = content
            .into_iter()
            .map(|block| cap_block(block, &mut dropped))
            .collect();
        if !dropped.is_empty() {
            tracing::warn!(
                fields = dropped.len(),
                "a record read from a file carried oversize content; it was bounded"
            );
        }
    }
    entry
}

/// Bound one replay payload. Rule 10: a payload over `MAX_RECORD_BYTES` is dropped whole.
///
/// A payload is opaque, so it cannot be trimmed. Half a signature still looks like a
/// signature and would be replayed as one, and a record that cannot be read back makes the
/// whole session unresumable. So the answer is all or nothing, and the drop is reported.
fn cap_state(state: Option<crate::ProviderState>) -> Option<crate::ProviderState> {
    let state = state?;
    let size = serde_json::to_string(&state.value).map(|text| text.len());
    match size {
        Ok(size) if size <= MAX_RECORD_BYTES => Some(state),
        Ok(size) => {
            // The report names the size and the owner, and never the value, per rule 9. It
            // is a log line and not a spill, because a spilled payload cannot be replayed
            // from a sidecar and would only carry opaque bytes into a second file.
            // The provider name may come from a file, so it is sanitised. See
            // `D-one-redaction-home`.
            tracing::warn!(
                provider = %rho_redact::sanitize_line(&state.owner.provider),
                size,
                "a reasoning payload exceeded the record cap and was dropped"
            );
            None
        }
        // A value that cannot be encoded cannot be written either, so it goes the same way.
        Err(_) => {
            tracing::warn!("a reasoning payload could not be encoded and was dropped");
            None
        }
    }
}

/// Cap one record so its encoded line fits `MAX_RECORD_BYTES`. Only a `Message` carries
/// free-form text or arguments, so only a `Message` needs a cap.
fn cap_record(record: Record, spills: &mut Vec<String>) -> Record {
    match record {
        Record::Message { message } => Record::Message {
            message: Message {
                role: message.role,
                content: message
                    .content
                    .into_iter()
                    .map(|b| cap_block(b, spills))
                    .collect(),
            },
        },
        other => other,
    }
}

/// What one read produced.
#[derive(Clone, Debug)]
pub struct ReadResult {
    pub header: SessionHeader,
    pub entries: Vec<Entry>,
    /// True when the last line was partial and dropped. Resume warns on this.
    pub truncated_tail: bool,
    /// How many records in the **middle** of the file did not decode.
    ///
    /// A security review found that any bad line stopped the read, so one flipped byte
    /// silently discarded every later record and reported it as a truncated tail. A bad
    /// record in the middle is now skipped and counted, and the tail rule is unchanged.
    /// See `D-a-bad-middle-record-is-skipped-and-counted`.
    pub dropped_records: usize,
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

/// Read one line into `buf`, up to `MAX_LINE_BYTES`. Return `true` when a line was read,
/// `false` at end of input. A line that reaches the cap is a `SessionError::Decode`,
/// never an unbounded allocation, and the reader stops before it pulls the whole line.
/// See section 6a and D-reader-line-cap.
fn read_capped_line<R: BufRead>(source: &mut R, buf: &mut Vec<u8>) -> Result<bool, SessionError> {
    buf.clear();
    loop {
        let available = match source.fill_buf() {
            Ok(bytes) => bytes,
            Err(e) => return Err(SessionError::Io(e.to_string())),
        };
        if available.is_empty() {
            return Ok(!buf.is_empty());
        }
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            if buf.len() + pos + 1 > MAX_LINE_BYTES {
                return Err(SessionError::Decode(format!(
                    "a session line exceeds {MAX_LINE_BYTES} bytes"
                )));
            }
            buf.extend_from_slice(&available[..=pos]);
            source.consume(pos + 1);
            return Ok(true);
        }
        if buf.len() + available.len() > MAX_LINE_BYTES {
            // Stop before consuming the overflow, so the reader never pulls the whole
            // giant line through. This is the bound the counting-reader test asserts.
            return Err(SessionError::Decode(format!(
                "a session line exceeds {MAX_LINE_BYTES} bytes"
            )));
        }
        let taken = available.len();
        buf.extend_from_slice(available);
        source.consume(taken);
    }
}

/// The header fields and the entries parsed from one source.
fn parse_header(line: &str) -> Result<SessionHeader, SessionError> {
    let entry: Entry = decode(line)?;
    match entry.record {
        Record::Session {
            version,
            cwd,
            approval,
            sandbox,
        } => {
            if version != SESSION_FORMAT_VERSION {
                return Err(SessionError::Version(version));
            }
            Ok(SessionHeader {
                version,
                session_id: String::new(),
                cwd,
                approval,
                sandbox,
            })
        }
        _ => Err(SessionError::Decode(
            "the first record is not a session header".to_string(),
        )),
    }
}

impl SessionReader {
    /// Read every whole record. Drop only a truncated last line. See section 6.
    /// Cap one line at `MAX_LINE_BYTES`. A longer line is a `SessionError::Decode`,
    /// never an unbounded allocation. See section 6a.
    pub fn read(path: &Path) -> Result<ReadResult, SessionError> {
        let file = File::open(path).map_err(|e| io_error(path, e))?;
        let mut result = Self::read_from(BufReader::new(file))?;
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            result.header.session_id = stem.to_string();
        }
        Ok(result)
    }

    /// Read a session from any buffered source.
    ///
    /// This is the seam a test uses. A test passes a reader that counts the bytes it
    /// hands out, and asserts the count stays at or under `MAX_LINE_BYTES` for one
    /// line. So a test can fail against an unbounded implementation. See section 6a.
    pub fn read_from<R: BufRead>(mut source: R) -> Result<ReadResult, SessionError> {
        let mut buf = Vec::new();
        if !read_capped_line(&mut source, &mut buf)? {
            return Err(SessionError::Decode(
                "the session file is empty".to_string(),
            ));
        }
        let first = String::from_utf8_lossy(&buf);
        let header = parse_header(first.trim_end_matches(['\n', '\r']))?;

        let mut entries = Vec::new();
        let mut truncated_tail = false;
        let mut pending_bad_line = false;
        let mut dropped_records = 0usize;
        while read_capped_line(&mut source, &mut buf)? {
            let line = String::from_utf8_lossy(&buf);
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                continue;
            }
            match decode::<Entry>(line) {
                // A payload from a file rho did not write is bounded here as well as on the
                // way in. A security review found the bound was write-only, so a foreign
                // file could carry a payload up to the line cap and replay it every turn.
                Ok(entry) => {
                    // A bad line with a good record after it was corruption in the middle,
                    // not a truncated tail. Count it and keep going, so one flipped byte
                    // cannot discard the rest of the session in silence.
                    if pending_bad_line {
                        dropped_records += 1;
                        pending_bad_line = false;
                    }
                    entries.push(cap_entry_state(entry));
                }
                Err(_) => {
                    // Two bad lines in a row means the first one was in the middle.
                    if pending_bad_line {
                        dropped_records += 1;
                    }
                    pending_bad_line = true;
                    // A file of nothing but bad lines is no longer a stop, so it is now a full
                    // pass. A security review asked for a ceiling, because a crafted file of a
                    // million bad lines would otherwise buy a million iterations.
                    if dropped_records >= MAX_DROPPED_RECORDS {
                        tracing::warn!(
                            dropped = dropped_records,
                            "the session file had too many records that did not decode; \
                             the read stopped"
                        );
                        break;
                    }
                    continue;
                }
            }
        }
        // The last bad line, if any, is the tail. Every earlier one was counted above.
        if pending_bad_line {
            truncated_tail = true;
        }
        if dropped_records > 0 {
            tracing::warn!(
                dropped = dropped_records,
                "the session file had records that did not decode; they were skipped"
            );
        }
        if truncated_tail {
            // A crash can cut the last line in half. Drop it, but never silently. See
            // D-truncated-tail-warns and D-write-failure-degrades.
            tracing::warn!("the session file had a truncated last line; it was dropped");
        }
        Ok(ReadResult {
            header,
            entries,
            truncated_tail,
            dropped_records,
        })
    }
}

/// The set of session files under one directory.
pub struct SessionStore {
    root: PathBuf,
}

impl SessionStore {
    /// A store rooted at a directory. The directory holds one file per session.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The file path for one session id.
    fn session_path(&self, session_id: &str) -> PathBuf {
        self.root.join(format!("{session_id}.jsonl"))
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
        std::fs::create_dir_all(&self.root).map_err(|e| io_error(self.root.as_path(), e))?;
        let path = self.session_path(session_id);
        // Create or truncate, so a new session starts with a clean file. The writer
        // holds this handle for the life of the session. See SPEC-sessions section 3.
        let file = File::create(&path).map_err(|e| io_error(path.as_path(), e))?;
        let mut writer = SessionWriter::with_sink(path, Box::new(file));
        let header = Record::Session {
            version: SESSION_FORMAT_VERSION,
            cwd: cwd.to_path_buf(),
            approval: approval.to_string(),
            sandbox: sandbox.to_string(),
        };
        writer.append(header, None)?;
        Ok(writer)
    }

    /// Open an existing file to append more records. Used by resume and fork.
    pub fn append_to(&self, path: &Path) -> Result<SessionWriter, SessionError> {
        let read = SessionReader::read(path)?;
        let head = read.entries.last().map(|e| e.id.clone());
        // Seed the id counter past every id already in the file, so a later append
        // never mints an id that collides with an earlier record.
        let next_id = read.entries.len() as u64 + 2;
        // Hold an appending handle for the life of the reopened session.
        let file = OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|e| io_error(path, e))?;
        let mut writer = SessionWriter::with_sink(path.to_path_buf(), Box::new(file));
        writer.head = head;
        writer.next_id = next_id;
        // A closed file ends with `Closed`. State the reopen on disk, so a reader never
        // finds `Closed` in the middle of a file with no explanation.
        let was_closed = matches!(
            read.entries.last().map(|entry| &entry.record),
            Some(Record::Closed)
        );
        if was_closed {
            let parent = writer.head.clone();
            writer.append(Record::Reopened, parent)?;
        }
        Ok(writer)
    }

    /// List sessions with a cheap summary. Read only the first line of each file.
    /// The first line obeys the same `MAX_LINE_BYTES` cap as `read`.
    pub fn list(&self) -> Result<Vec<SessionSummary>, SessionError> {
        let mut out = Vec::new();
        let dir = match std::fs::read_dir(&self.root) {
            Ok(dir) => dir,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(SessionError::Io(e.to_string())),
        };
        for entry in dir {
            let path = entry.map_err(|e| io_error(self.root.as_path(), e))?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let file = File::open(&path).map_err(|e| io_error(path.as_path(), e))?;
            let mut reader = BufReader::new(file);
            let mut buf = Vec::new();
            if !read_capped_line(&mut reader, &mut buf)? {
                continue;
            }
            let line = String::from_utf8_lossy(&buf);
            let header = parse_header(line.trim_end_matches(['\n', '\r']))?;
            let session_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            out.push(SessionSummary {
                session_id,
                path,
                cwd: header.cwd,
                size_bytes,
            });
        }
        Ok(out)
    }

    /// Delete one session file. Every branch in the file goes with it.
    pub fn delete(&self, session_id: &str) -> Result<(), SessionError> {
        let path = self.session_path(session_id);
        std::fs::remove_file(&path).map_err(|e| io_error(path.as_path(), e))?;
        // Remove any sidecar files that belong to this session too.
        let prefix = format!("{session_id}.");
        if let Ok(dir) = std::fs::read_dir(&self.root) {
            for entry in dir.flatten() {
                let sidecar = entry.path();
                let is_sidecar = sidecar.extension().and_then(|e| e.to_str()) == Some("sidecar");
                let matches = sidecar
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(&prefix))
                    .unwrap_or(false);
                if is_sidecar && matches {
                    let _ = std::fs::remove_file(&sidecar);
                }
            }
        }
        Ok(())
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
        let read = SessionReader::read(from_path)?;
        // Walk parent links from `from` to the root, then reverse to file order.
        let map: HashMap<&RecordId, &Entry> = read.entries.iter().map(|e| (&e.id, e)).collect();
        let mut chain = Vec::new();
        let mut cursor = Some(from.clone());
        while let Some(id) = cursor {
            match map.get(&id) {
                Some(entry) => {
                    chain.push((*entry).clone());
                    cursor = entry.parent_id.clone();
                }
                None => break,
            }
        }
        chain.reverse();

        std::fs::create_dir_all(&self.root).map_err(|e| io_error(self.root.as_path(), e))?;
        let new_path = self.session_path(new_id);
        let file = File::create(&new_path).map_err(|e| io_error(new_path.as_path(), e))?;
        let mut writer = SessionWriter::with_sink(new_path, Box::new(file));
        // Write a fresh header for the new file, then copy the branch verbatim.
        let header = Record::Session {
            version: read.header.version,
            cwd: read.header.cwd,
            approval: read.header.approval,
            sandbox: read.header.sandbox,
        };
        let header_id = writer.mint_id();
        writer.write_entry(&Entry {
            id: header_id.clone(),
            parent_id: None,
            timestamp: now_timestamp(),
            record: header,
        })?;
        writer.head = Some(header_id);
        for entry in chain {
            writer.write_entry(&entry)?;
            writer.head = Some(entry.id.clone());
            writer.next_id += 1;
        }
        Ok(writer)
    }
}

/// Rebuild the message list along the branch that ends at `head`. Walk parent
/// links from `head` to the root. Reverse the walk. Return the messages in order.
///
/// A trailing `ToolCall` with no matching `ToolResult` is repaired with a synthetic
/// error result, so the rebuilt list holds a complete pairing and the next provider
/// request is valid. See section 8a.
pub fn branch_messages(entries: &[Entry], head: &RecordId) -> Vec<Message> {
    let map: HashMap<&RecordId, &Entry> = entries.iter().map(|e| (&e.id, e)).collect();
    let mut chain = Vec::new();
    let mut cursor = Some(head.clone());
    while let Some(id) = cursor {
        match map.get(&id) {
            Some(entry) => {
                chain.push(*entry);
                cursor = entry.parent_id.clone();
            }
            None => break,
        }
    }
    chain.reverse();

    let mut messages: Vec<Message> = chain
        .iter()
        .filter_map(|entry| match &entry.record {
            Record::Message { message } => Some(message.clone()),
            _ => None,
        })
        .collect();

    // Repair any tool call that has no matching result.
    let mut calls = Vec::new();
    let mut results = HashSet::new();
    for message in &messages {
        for block in &message.content {
            match block {
                ContentBlock::ToolCall { id, .. } => calls.push(id.clone()),
                ContentBlock::ToolResult { tool_call_id, .. } => {
                    results.insert(tool_call_id.clone());
                }
                _ => {}
            }
        }
    }
    for id in calls {
        if results.insert(id.clone()) {
            messages.push(synthetic_error_result(
                &id,
                "the tool call did not finish before the session ended",
            ));
        }
    }
    messages
}

/// A tool message that carries one synthetic error result for a call id.
fn synthetic_error_result(tool_call_id: &str, reason: &str) -> Message {
    Message {
        role: Role::Tool,
        content: vec![ContentBlock::ToolResult {
            tool_call_id: tool_call_id.to_string(),
            content: vec![ContentBlock::Text {
                text: reason.to_string(),
            }],
            is_error: true,
        }],
    }
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
        match self {
            SessionLog::Off => None,
            SessionLog::File(writer) => {
                // A later record links to the head by default, so the log chains records.
                let parent = parent.or_else(|| writer.head());
                match writer.append(record, parent) {
                    Ok(id) => Some(id),
                    Err(error) => {
                        // A write failure degrades the session to ephemeral. It never ends
                        // the run. See D-write-failure-degrades and defect 9.
                        tracing::warn!(
                            %error,
                            "a session write failed; the session log degrades to ephemeral"
                        );
                        *self = SessionLog::Off;
                        None
                    }
                }
            }
        }
    }

    /// True when this log writes nothing.
    pub fn is_ephemeral(&self) -> bool {
        matches!(self, SessionLog::Off)
    }
}

/// Folds the agent event stream into session records.
pub struct SessionRecorder {
    log: SessionLog,
    /// The tool calls opened in the current turn that have no result yet.
    open_calls: Vec<(String, String)>,
}

/// Redact every credential-shaped tool argument inside one content block. A message
/// content block never holds a `Secret`, but a `ToolCall.arguments` value is free JSON,
/// so it can carry a key. See section 5a and D-redact-tool-arguments.
fn redact_block(block: &ContentBlock) -> ContentBlock {
    match block {
        ContentBlock::ToolCall {
            id,
            name,
            arguments,
            state,
        } => ContentBlock::ToolCall {
            id: id.clone(),
            name: name.clone(),
            arguments: rho_redact::redact_json_secrets(arguments),
            // A payload is opaque provider bytes, and a rewritten payload cannot replay.
            // Rule 9 keeps it verbatim, and rule 10 bounds it instead.
            state: state.clone(),
        },
        // Named arms, because a wildcard let a new block bypass redaction in silence. A
        // reasoning text is model output, so it gets the same treatment as assistant text.
        ContentBlock::ReasoningTrace { text } => {
            ContentBlock::ReasoningTrace { text: text.clone() }
        }
        ContentBlock::ReasoningReplay { text, state } => ContentBlock::ReasoningReplay {
            text: text.clone(),
            state: state.clone(),
        },
        ContentBlock::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => ContentBlock::ToolResult {
            tool_call_id: tool_call_id.clone(),
            content: content.iter().map(redact_block).collect(),
            is_error: *is_error,
        },
        other => other.clone(),
    }
}

impl SessionRecorder {
    /// Build a recorder over a log. `SessionLog::Off` gives an ephemeral recorder.
    pub fn new(log: SessionLog) -> Self {
        Self {
            log,
            open_calls: Vec::new(),
        }
    }

    /// Record the user's prompt as a `Message` record. Redact the content first.
    pub fn record_prompt(&mut self, input: &[ContentBlock]) -> Option<RecordId> {
        let content = input.iter().map(redact_block).collect();
        let message = Message {
            role: Role::User,
            content,
        };
        self.log.record(Record::Message { message }, None)
    }

    /// Fold one agent event. Write an assistant message at a turn end, a tool
    /// result at a tool end, a usage record on a usage event, and a stop record at
    /// the agent end. Redact every tool argument first. Return an id when it writes.
    pub fn observe(&mut self, event: &AgentEvent) -> Option<RecordId> {
        match event {
            AgentEvent::TurnStart => {
                self.open_calls.clear();
                None
            }
            AgentEvent::ToolStart { id, name, .. } => {
                self.open_calls.push((id.clone(), name.clone()));
                None
            }
            AgentEvent::ToolEnd { id, output } => {
                self.open_calls.retain(|(open, _)| open != id);
                let content = output.content.iter().map(redact_block).collect();
                let message = Message {
                    role: Role::Tool,
                    content,
                };
                self.log.record(Record::Message { message }, None)
            }
            AgentEvent::Stream(StreamEvent::Usage(usage)) => {
                self.log.record(Record::Usage { usage: *usage }, None)
            }
            AgentEvent::AgentEnd { stop_reason } => self.log.record(
                Record::Stop {
                    stop_reason: *stop_reason,
                },
                None,
            ),
            _ => None,
        }
    }

    /// On a cancel, complete any open tool pairing, then write the stop record.
    /// See section 8.
    pub fn record_cancel(&mut self) -> Option<RecordId> {
        let mut last = None;
        let open = std::mem::take(&mut self.open_calls);
        if !open.is_empty() {
            // Write the assistant message that carries the open tool calls, so every
            // ToolCall is on disk before its result. The arguments are not known here,
            // so an empty object stands in; the id and the name keep the pairing valid.
            let calls = open
                .iter()
                .map(|(id, name)| ContentBlock::ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: serde_json::json!({}),
                    // The payload is not known here, and a guessed one would be replayed.
                    state: None,
                })
                .collect();
            last = self.log.record(
                Record::Message {
                    message: Message {
                        role: Role::Assistant,
                        content: calls,
                    },
                },
                None,
            );
            for (id, _) in open {
                last = self.log.record(
                    Record::Message {
                        message: synthetic_error_result(&id, "the tool call was cancelled"),
                    },
                    None,
                );
            }
        }
        let stop = self.log.record(
            Record::Stop {
                stop_reason: AgentStopReason::Canceled,
            },
            None,
        );
        stop.or(last)
    }

    /// True when the log is ephemeral, or degraded to ephemeral.
    pub fn is_ephemeral(&self) -> bool {
        self.log.is_ephemeral()
    }
}
