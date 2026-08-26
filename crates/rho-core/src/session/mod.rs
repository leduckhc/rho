//! The session log. An append-only JSONL record of one conversation.
//!
//! See `SPEC-sessions`, `ADR-session-format`, and `ADR-jsonl-codec`. This module holds the record set, the
//! codec seam, the writer, the reader, the store, and the event recorder.
//!
//! Stage T4 defined the public surface. Stage T5 made every body real.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{AgentEvent, AgentStopReason, ContentBlock, Message, Role, StreamEvent, Usage};

mod key;
mod lock;
mod row;
pub use key::{GIT_ENTRY_MAX_BYTES, PrefixMatch, ProjectKey, SessionId, default_store_root};
pub use lock::{SessionLock, classify_lock_failure};
pub use row::{ROW_HEAD_LINES, ROW_TAIL_BYTES, RowMeta, SessionRow, SessionSummary, row_from};

// ---------------------------------------------------------------------------
// Section 2. The record set.
// ---------------------------------------------------------------------------

/// A record id. A short, unique string, minted per record.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RecordId(pub String);

/// A record id prints as its own text, so an error message can name one.
///
/// Two error messages in section 7e of `SPEC-session-store-wiring` need this. A cold
/// compile of the contract in a scratch crate found the gap.
impl std::fmt::Display for RecordId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The origin of a forked session. It is shown, and it is never trusted.
///
/// It opens no file and grants nothing. A forged origin is therefore harmless. See
/// `SPEC-session-store-wiring` section 9.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForkOrigin {
    /// The id of the session this file was forked from.
    pub session_id: String,
    /// The record the fork started at, in that session.
    pub record_id: RecordId,
}

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
        /// The session id. It was implicit in the file name before.
        ///
        /// A rename of the file then changed the id a reader saw, and a copy of a file
        /// carried no id at all.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        /// Set when this file came from a fork.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        forked_from: Option<ForkOrigin>,
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
    /// An explicit session title. **A leaf record.** It is never a parent.
    ///
    /// The newest `Name` record wins. See `D-a-session-title-costs-nothing`.
    Name { title: String },
}

/// Is this record a leaf, which is never a parent?
///
/// The match has no wildcard on purpose. A new record forces a reader to decide its class,
/// instead of inheriting a fail-open default. See `D-chain-records-are-frozen` and
/// `D-plugin-does-not-classify-itself`.
fn is_leaf_record(record: &Record) -> bool {
    match record {
        Record::Name { .. } => true,
        Record::Session { .. }
        | Record::ModelChange { .. }
        | Record::Message { .. }
        | Record::Usage { .. }
        | Record::Stop { .. }
        | Record::Closed
        | Record::Reopened => false,
    }
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
    /// Set when this file came from a fork. It is shown, and never trusted.
    pub forked_from: Option<ForkOrigin>,
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
    /// A record names a parent the reader does not hold, so the chain has a hole.
    ///
    /// The check runs from the child side, because a tagged reader cannot tell an unknown
    /// leaf from an unknown chain record. Both fail to decode into the same dropped count,
    /// and the id of a line that did not decode is inside that line. So the class is derived
    /// from the data instead of declared by a writer. See
    /// `SPEC-session-store-wiring` section 6a.
    #[error("record {child} names parent {parent}, which this build could not read")]
    Orphan { child: RecordId, parent: RecordId },
    /// A record names a leaf record as its parent. A leaf is never a parent.
    #[error("record {child} names parent {parent}, which is a leaf record and never a parent")]
    LeafParent { child: RecordId, parent: RecordId },
    /// Two records in one file share an id.
    #[error("record id {id} appears twice in the file")]
    DuplicateId { id: RecordId },
    /// A walk asked for a record the file does not hold.
    ///
    /// A user typing `--at r99` reaches this, so the message names the id.
    #[error("the session holds no record {id}")]
    NoSuchRecord { id: RecordId },
    /// A prefix matched more than one session. The message lists every match.
    #[error("the id prefix {prefix} matches {} sessions: {}", matches.len(), matches.join(", "))]
    AmbiguousPrefix {
        prefix: String,
        matches: Vec<String>,
    },
    /// A prefix matched no session in this project.
    #[error("no session in this project starts with {prefix}")]
    NoSuchSession { prefix: String },
    /// `--continue` found no session to continue.
    #[error("no session to continue in {project}; start one without --continue")]
    NoSessionToContinue { project: String },
    /// A session title was empty or blank.
    ///
    /// It had no name at first, so the refusal arrived as a decode error and read like file
    /// corruption. A live drive showed `cannot decode a record: a session title cannot be
    /// empty`. Every error case gets a name.
    #[error("a session title cannot be empty")]
    EmptyTitle,
    /// A create could not find a free id after `MINT_ATTEMPTS` tries.
    #[error("could not mint a free session id after {attempts} tries; the store may be full")]
    MintExhausted { attempts: usize },
    /// Another process holds this session.
    #[error("session {id} is open in another process. Use another session, or close that one.")]
    Busy { id: String },
    /// The filesystem cannot hold an advisory lock.
    ///
    /// The message calls `path.display()`, because `PathBuf` does not implement `Display`.
    #[error(
        "the filesystem at {} cannot lock a session. Set a store on a local disk.",
        path.display()
    )]
    LockUnsupported { path: PathBuf },
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
    /// Every record id the file already holds.
    ///
    /// A count is not enough. `append_to` used `entries.len() + 2`, and the reader drops a
    /// record it cannot decode, so two drops made the count mint an id the file held. A fork
    /// added one per copied record, and a branch is not contiguous, so a copied chain of
    /// `r1`, `r2`, `r4` made a second `r4`.
    ///
    /// So the writer mints against the **set**, and never against a count. It seeds itself
    /// inside `append_to` and inside `fork`, so no caller can forget it and no caller can
    /// seed the wrong ids. See `D-a-record-id-is-minted-against-the-set` and section 7a.
    known_ids: HashSet<String>,
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
            known_ids: HashSet::new(),
        }
    }

    /// Seed the id set from a file the writer is about to append to.
    ///
    /// It is private, so no caller can seed the wrong ids and force a collision. A first
    /// draft of the contract offered a public `seed_ids`, and then forbade every test from
    /// calling it. A public method a spec forbids is a hazard.
    fn seed_ids(&mut self, header_id: &RecordId, entries: &[Entry]) {
        self.known_ids.insert(header_id.0.clone());
        for entry in entries {
            self.known_ids.insert(entry.id.0.clone());
        }
    }

    /// Mint the next record id. The ids are unique within one file.
    ///
    /// It skips every id the file already holds, so a dropped record, a copied branch, and an
    /// imported file all mint a free id.
    fn mint_id(&mut self) -> RecordId {
        loop {
            let id = RecordId(format!("r{}", self.next_id));
            self.next_id += 1;
            if self.known_ids.insert(id.0.clone()) {
                return id;
            }
        }
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
        // **A leaf record never becomes the head.** A leaf is never a parent, so a later record
        // that linked to it would name a leaf as its parent and the whole file would be refused
        // at read time. A live drive met exactly that after `rho sessions name`:
        //
        //     record r24 names parent r23, which is a leaf record and never a parent
        //
        // So the chain skips a leaf, and the next record links to the last chain record.
        let is_leaf = is_leaf_record(&entry.record);
        self.write_entry(&entry)?;
        if !is_leaf {
            self.head = Some(id.clone());
        }
        Ok(id)
    }

    /// Write the spilled payloads for one record to a sidecar file.
    fn spill_to_sidecar(&self, id: &RecordId, spills: &[String]) -> Result<(), SessionError> {
        let path = self.sidecar_path(id);
        let mut file = File::create(&path).map_err(|e| io_error(path.as_path(), e))?;
        // A spill holds whatever a tool read, so it gets the mode the session file gets. A
        // default umask would make it `0o644`. See `D-a-session-file-is-private`.
        set_owner_only(&path)?;
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

/// Make one file readable and writable by its owner alone.
///
/// A session file holds a whole conversation. A default umask makes it `0o644`, and then any
/// local user or a synced backup folder reads it. `transcript.rs` already solved this, and
/// this follows it. See `D-a-session-file-is-private`.
fn set_owner_only(path: &Path) -> Result<(), SessionError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| io_error(path, e))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Create a directory and every missing parent, and make each one rho creates owner only.
///
/// It sets the mode on the directories it creates, and never on one that already existed. So
/// a user's own `~` keeps its mode, and every directory under the store root is `0o700`.
fn create_private_dir(dir: &Path) -> Result<(), SessionError> {
    // The deepest existing ancestor marks where rho's own directories begin.
    let mut ours: Vec<&Path> = Vec::new();
    let mut cursor = Some(dir);
    while let Some(current) = cursor {
        if current.exists() {
            break;
        }
        ours.push(current);
        cursor = current.parent();
    }
    std::fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
    #[cfg(unix)]
    for created in ours {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(created, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| io_error(created, e))?;
    }
    #[cfg(not(unix))]
    let _ = ours;
    Ok(())
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

/// A sink that counts encoded bytes and stops as soon as the count passes a limit.
///
/// It allocates nothing, and it short-circuits, so measuring a record costs at most the limit
/// rather than the size of the record.
struct ByteCounter {
    count: usize,
    limit: usize,
}

impl Write for ByteCounter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.count += buf.len();
        if self.count > self.limit {
            // Any error stops the serializer. The caller reads the stop as "does not fit".
            return Err(std::io::Error::other("the record is over the limit"));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Does this record fit the record cap once its fields are bounded?
///
/// A record that does not fit was not written by rho, because the write path caps the whole
/// encoded line. Dropping it is the same answer the write path would have given.
///
/// **The first version gated this on the raw line length**, on the reasoning that a line under
/// the cap could not encode to more than the cap. That is false, and a probe measured it: a
/// line packed with floats in exponent form re-encodes 3.8 times larger, because `1e15` becomes
/// `1000000000000000.0`. A 60 kB record therefore slipped the gate and landed at 228 kB. The
/// count is now exact and runs for **every** record. `ByteCounter` stops at the cap, so
/// measuring one record costs no more than the cap however large the record is.
fn record_fits(entry: &Entry) -> bool {
    #[cfg_attr(feature = "fast-json", allow(unused_mut))]
    let mut counter = ByteCounter {
        count: 0,
        limit: MAX_RECORD_BYTES,
    };
    // One mechanism, not two. A first version also compared `counter.count` at the end, and a
    // mutation showed that each guard masked the other: deleting either changed nothing a test
    // could see. The same redundant-guard trap appeared in `ProviderState::for_owner`, and the
    // answer is the same. The abort is the guard, and `a_counter_stops_at_its_limit` pins it.
    #[cfg(not(feature = "fast-json"))]
    {
        serde_json::to_writer(&mut counter, entry).is_ok()
    }
    // `sonic_rs` writes through its own `WriteExt`, which `ByteCounter` does not implement, so
    // the optional fast codec measures by encoding instead. One allocation, bounded by the line
    // cap. The answer is the same, and only the cost differs, on a path that is opt-in.
    #[cfg(feature = "fast-json")]
    {
        let _ = counter;
        matches!(encode(entry), Ok(line) if line.len() <= MAX_RECORD_BYTES)
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
    /// The record id of the header line.
    ///
    /// The header itself is not in `entries`, because `read_from` consumes the first line
    /// before its loop. So a caller that re-parents a record, or that seeds an id set, needs
    /// the header id from here. Without it a fork re-parents onto a guess, and
    /// `D-a-record-id-is-minted-against-the-set` cannot hold.
    pub header_id: RecordId,
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
fn parse_header(line: &str) -> Result<(RecordId, SessionHeader), SessionError> {
    let entry: Entry = decode(line)?;
    let header_id = entry.id.clone();
    match entry.record {
        Record::Session {
            version,
            cwd,
            approval,
            sandbox,
            session_id,
            forked_from,
        } => {
            if version != SESSION_FORMAT_VERSION {
                return Err(SessionError::Version(version));
            }
            Ok((
                header_id,
                SessionHeader {
                    version,
                    // A file written before this field existed states no id. The reader then
                    // falls back to the file stem, in `SessionReader::read`.
                    session_id: session_id.unwrap_or_default(),
                    cwd,
                    approval,
                    sandbox,
                    forked_from,
                },
            ))
        }
        _ => Err(SessionError::Decode(
            "the first record is not a session header".to_string(),
        )),
    }
}

/// Refuse a file whose record ids are ambiguous, or whose chain has a hole.
///
/// Two checks, and both run before any caller walks a parent link.
///
/// 1. No id appears twice. A duplicate makes a walk ambiguous, so a resume could rebuild
///    either of two conversations.
/// 2. Every non-root `parent_id` resolves to a record the reader holds, and that record is
///    not a leaf.
///
/// Check 2 is the version rule of section 6a. A tagged reader cannot tell an unknown leaf
/// from an unknown chain record, so the class is derived from the data: a skipped chain
/// record shows up as a child that points at nothing, and nothing ever points at a leaf.
fn check_integrity(header_id: &RecordId, entries: &[Entry]) -> Result<(), SessionError> {
    let mut known: HashMap<&str, bool> = HashMap::with_capacity(entries.len() + 1);
    known.insert(header_id.0.as_str(), false);
    for entry in entries {
        if known
            .insert(entry.id.0.as_str(), is_leaf_record(&entry.record))
            .is_some()
        {
            return Err(SessionError::DuplicateId {
                id: entry.id.clone(),
            });
        }
    }
    for entry in entries {
        let Some(parent) = &entry.parent_id else {
            continue;
        };
        match known.get(parent.0.as_str()) {
            None => {
                return Err(SessionError::Orphan {
                    child: entry.id.clone(),
                    parent: parent.clone(),
                });
            }
            Some(true) => {
                return Err(SessionError::LeafParent {
                    child: entry.id.clone(),
                    parent: parent.clone(),
                });
            }
            Some(false) => {}
        }
    }
    Ok(())
}

impl SessionReader {
    /// Read every whole record. Drop only a truncated last line. See section 6.
    /// Cap one line at `MAX_LINE_BYTES`. A longer line is a `SessionError::Decode`,
    /// never an unbounded allocation. See section 6a.
    pub fn read(path: &Path) -> Result<ReadResult, SessionError> {
        let file = File::open(path).map_err(|e| io_error(path, e))?;
        let mut result = Self::read_from(BufReader::new(file))?;
        // A file written before the header carried its own id states none. The stem is then
        // the id, because the stem is where the id used to live.
        if result.header.session_id.is_empty()
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
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
        let (header_id, header) = parse_header(first.trim_end_matches(['\n', '\r']))?;

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
                    let entry = cap_entry_state(entry);
                    // The write path checks the **whole** encoded record against
                    // `MAX_RECORD_BYTES`, and the read path checked only each field. So a
                    // record of ten thousand small blocks passed every field cap and still
                    // weighed megabytes. A security review named that asymmetry as a class, and
                    // this is the second member of it.
                    //
                    // The count runs for every record, and it stops at the cap, so it costs no
                    // more than the cap however large the record is. An earlier version gated it
                    // on the raw line length, and a probe showed that a line under the cap can
                    // re-encode past it.
                    if !record_fits(&entry) {
                        dropped_records += 1;
                        tracing::warn!(
                            bytes = line.len(),
                            "a record exceeded the record cap after its fields were bounded; \
                             it was dropped"
                        );
                        // The same ceiling as a decode failure. A security review found it
                        // covered only that path, so a file of valid but oversize records
                        // walked to the end while the ceiling never fired.
                        if dropped_records >= MAX_DROPPED_RECORDS {
                            tracing::warn!(
                                dropped = dropped_records,
                                "the session file had too many records that did not fit; \
                                 the read stopped"
                            );
                            break;
                        }
                        continue;
                    }
                    entries.push(entry);
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
        // The refusal runs here, before any caller walks a parent link. See section 6a.
        check_integrity(&header_id, &entries)?;
        Ok(ReadResult {
            header,
            header_id,
            entries,
            truncated_tail,
            dropped_records,
        })
    }
}

/// How many times a create re-mints an id before it gives up.
///
/// The id is a one second stamp plus four hex characters, which is 65536 values. For N
/// sessions minting in one second the collision chance is about `N * (N - 1) / 2 / 65536`.
/// At 50 concurrent sessions that is 1.87 percent. So a retry is needed, and it is bounded
/// so a full store cannot spin. See section 7c.
pub const MINT_ATTEMPTS: usize = 8;

/// What a new session needs. One struct, so a later field breaks no caller.
///
/// A four-argument constructor already hid a fake model id and an approve-all policy in this
/// project. See `D-no-four-argument-session-new`.
#[derive(Clone, Debug)]
pub struct NewSession<'a> {
    pub id: &'a SessionId,
    pub cwd: &'a Path,
    pub approval: &'a str,
    pub sandbox: &'a str,
    /// The provider and the model this session starts with.
    pub provider: &'a str,
    pub model: &'a str,
    /// Set only by a fork.
    pub forked_from: Option<ForkOrigin>,
}

/// What a new session needs, when the caller has no id yet.
///
/// The id cannot be a field here, because a retry mints a second one. See section 7 and
/// `SessionStore::create_minted`.
#[derive(Clone, Debug)]
pub struct NewSessionWithoutId<'a> {
    pub cwd: &'a Path,
    pub approval: &'a str,
    pub sandbox: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    pub forked_from: Option<ForkOrigin>,
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

    /// The file one session id lives in.
    ///
    /// A caller that holds an id from `rows` or `resolve_prefix` needs the path to read or to
    /// reopen it. Without this a caller would rebuild the naming rule, and two spellings of one
    /// rule drift.
    pub fn path_of(&self, id: &SessionId) -> PathBuf {
        self.session_path(id.as_str())
    }

    /// Create a session file. Write the header, then one `ModelChange` record.
    ///
    /// The model record is written here, not by a caller. So every file states its model on
    /// the second line, and a row reads it from the head. Nothing can forget it.
    ///
    /// **The file is created exclusively.** An existing path is an error, never a
    /// truncation. The old body called `File::create`, which truncates, so about one run in
    /// 53 at 50 concurrent sessions would have erased another session in silence. See
    /// section 7c.
    ///
    /// The file is created `0o600`, and every directory rho creates under the store root
    /// `0o700`. See `D-a-session-file-is-private`.
    pub fn create(&self, new: NewSession<'_>) -> Result<SessionWriter, SessionError> {
        let path = self.session_path(new.id.as_str());
        Self::create_file(&path, new)
    }

    /// Create a session at one exact path, with the same rules `create` applies.
    ///
    /// `create` calls this, so the store path and this path are the same code. It exists for
    /// the `session-file` config key, which names one exact file and overrides the store. See
    /// `D-session-store-layout`.
    pub fn create_file(path: &Path, new: NewSession<'_>) -> Result<SessionWriter, SessionError> {
        if let Some(parent) = path.parent() {
            create_private_dir(parent)?;
        }
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| io_error(path, e))?;
        set_owner_only(path)?;
        let mut writer = SessionWriter::with_sink(path.to_path_buf(), Box::new(file));
        let header = Record::Session {
            version: SESSION_FORMAT_VERSION,
            cwd: new.cwd.to_path_buf(),
            approval: new.approval.to_string(),
            sandbox: new.sandbox.to_string(),
            session_id: Some(new.id.as_str().to_string()),
            forked_from: new.forked_from,
        };
        let header_id = writer.append(header, None)?;
        // The second line always states the model, so a bounded head read finds it.
        writer.append(
            Record::ModelChange {
                provider: new.provider.to_string(),
                model: new.model.to_string(),
            },
            Some(header_id),
        )?;
        Ok(writer)
    }

    /// Create a session, and mint a fresh id when the first one is taken.
    ///
    /// This is what a run calls. It returns the id it used, because a retry replaces the
    /// first one and a caller cannot recompute it.
    pub fn create_minted(
        &self,
        now_millis: u64,
        new: NewSessionWithoutId<'_>,
    ) -> Result<(SessionId, SessionWriter), SessionError> {
        self.create_minted_from(now_millis, random_suffixes(), new)
    }

    /// Create a session, taking each id suffix from `suffixes`.
    ///
    /// **The retry needs this seam, or its test is theatre.** `create_minted` draws its own
    /// suffix, so two calls in one millisecond get two ids and no collision ever happens. A
    /// test could then never reach the retry. So the suffix source is a parameter here,
    /// exactly as `SessionReader::read_from` and `ProjectKey::resolve_from` take their input.
    ///
    /// `create_minted` calls this, so the store path and the tested path are the same code.
    pub fn create_minted_from<I: Iterator<Item = u16>>(
        &self,
        now_millis: u64,
        suffixes: I,
        new: NewSessionWithoutId<'_>,
    ) -> Result<(SessionId, SessionWriter), SessionError> {
        let mut last = None;
        for suffix in suffixes.take(MINT_ATTEMPTS) {
            let id = SessionId::mint(now_millis, suffix);
            let request = NewSession {
                id: &id,
                cwd: new.cwd,
                approval: new.approval,
                sandbox: new.sandbox,
                provider: new.provider,
                model: new.model,
                forked_from: new.forked_from.clone(),
            };
            match self.create(request) {
                Ok(writer) => return Ok((id, writer)),
                // A taken path costs one more mint. Any other failure is real, and it stops
                // here rather than being retried eight times.
                Err(SessionError::Io(message)) if is_already_exists(&message) => {
                    last = Some(message);
                }
                Err(other) => return Err(other),
            }
        }
        tracing::warn!(
            attempts = MINT_ATTEMPTS,
            last = ?last,
            "every minted session id was taken"
        );
        Err(SessionError::MintExhausted {
            attempts: MINT_ATTEMPTS,
        })
    }

    /// Open an existing file to append more records. Used by resume and fork.
    pub fn append_to(&self, path: &Path) -> Result<SessionWriter, SessionError> {
        let read = SessionReader::read(path)?;
        // The head is the last **chain** record, and never a leaf. A leaf is never a parent, so a
        // reopen that took a trailing `Name` record as the head would make the next append name a
        // leaf and the file would be refused. A live drive met that after `rho sessions name`.
        let head = read
            .entries
            .iter()
            .rev()
            .find(|entry| !is_leaf_record(&entry.record))
            .map(|entry| entry.id.clone())
            .unwrap_or_else(|| read.header_id.clone());
        // Hold an appending handle for the life of the reopened session.
        let file = OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|e| io_error(path, e))?;
        let mut writer = SessionWriter::with_sink(path.to_path_buf(), Box::new(file));
        writer.head = Some(head);
        // Seed the id set from the file itself, inside the store. A caller cannot forget it,
        // and a caller cannot seed the wrong ids. See section 7a.
        writer.seed_ids(&read.header_id, &read.entries);
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

    /// Every session in the store, newest first. A file rho cannot read is one row.
    ///
    /// It reads `ROW_HEAD_LINES` lines and `ROW_TAIL_BYTES` bytes per file, through `row_from`.
    /// So the store path and the tested path are the same code, and no file is fully decoded.
    ///
    /// It replaces `list`, which returned four fields no picker wants and failed the whole list
    /// on one unreadable file. Two methods for one job would leave dead surface.
    pub fn rows(&self) -> Result<Vec<SessionRow>, SessionError> {
        let mut paths = self.session_files()?;
        // The id sorts by time, so a name sort gives newest first and opens no file. See
        // `D-a-session-id-sorts-by-time`.
        paths.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        let mut out = Vec::with_capacity(paths.len());
        for path in paths {
            out.push(self.row_for(&path));
        }
        Ok(out)
    }

    /// One row for one file. Every failure becomes a row, never an error.
    fn row_for(&self, path: &Path) -> SessionRow {
        let meta = match std::fs::metadata(path) {
            Ok(meta) => meta,
            Err(error) => return row::unreadable_row(path.to_path_buf(), &io_error(path, error)),
        };
        let last_active_millis = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|since| since.as_millis() as u64)
            .unwrap_or(0);
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) => return row::unreadable_row(path.to_path_buf(), &io_error(path, error)),
        };
        row_from(
            BufReader::new(file),
            RowMeta {
                display_path: path.to_path_buf(),
                size_bytes: meta.len(),
                last_active_millis,
            },
        )
    }

    /// Every `.jsonl` file directly under the store root.
    ///
    /// A missing root is an empty store, not an error. A user with no sessions yet runs
    /// `rho sessions list` and reads "no sessions", never a stack of io text.
    fn session_files(&self) -> Result<Vec<PathBuf>, SessionError> {
        let dir = match std::fs::read_dir(&self.root) {
            Ok(dir) => dir,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_error(self.root.as_path(), e)),
        };
        let mut out = Vec::new();
        for entry in dir {
            let path = entry.map_err(|e| io_error(self.root.as_path(), e))?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                out.push(path);
            }
        }
        Ok(out)
    }

    /// Resolve a prefix to one id, to none, or to every match.
    ///
    /// A prefix never picks one of several. An ambiguous prefix returns every match, so the
    /// caller can list them all. See `D-a-session-id-sorts-by-time`.
    ///
    /// It reads no file. Every id is in a file name.
    pub fn resolve_prefix(&self, prefix: &str) -> Result<PrefixMatch, SessionError> {
        let mut matches: Vec<SessionId> = self
            .session_files()?
            .iter()
            .filter_map(|path| path.file_stem().and_then(|stem| stem.to_str()))
            .filter(|stem| stem.starts_with(prefix))
            .filter_map(|stem| SessionId::parse(stem).ok())
            .collect();
        matches.sort();
        match matches.len() {
            0 => Ok(PrefixMatch::None),
            1 => Ok(PrefixMatch::One(matches.remove(0))),
            _ => Ok(PrefixMatch::Many(matches)),
        }
    }

    /// The newest session in this store that holds no `Closed` record.
    ///
    /// **This is the crash offer, and it is not what `--continue` takes.** A run that ends on
    /// its own writes a `Closed` record, so this skips it. The first version of the contract
    /// used one method for both questions, and then bare `--continue` answered "no session to
    /// continue" right after a successful run. A live drive found it, and every unit test had
    /// passed. See `D-continue-takes-the-newest-session-closed-or-not`.
    ///
    /// It skips a session another process holds. See section 7d.
    pub fn newest_open(&self) -> Result<Option<SessionId>, SessionError> {
        self.newest_matching(|summary| !summary.closed)
    }

    /// The newest session `--continue` takes, closed or not.
    ///
    /// A closed file reopens, and `append_to` states the reopen on disk. So continuing a
    /// conversation a user closed is normal, and it is the common case.
    ///
    /// It skips a session another process holds, and it skips a file rho cannot read.
    pub fn newest_resumable(&self) -> Result<Option<SessionId>, SessionError> {
        self.newest_matching(|_| true)
    }

    /// The newest readable, unlocked session that passes `wanted`.
    ///
    /// One walk for both questions above, so the lock skip and the unreadable skip cannot drift
    /// between them.
    fn newest_matching(
        &self,
        wanted: impl Fn(&SessionSummary) -> bool,
    ) -> Result<Option<SessionId>, SessionError> {
        for row in self.rows()? {
            let SessionRow::Session(summary) = row else {
                // An unreadable file cannot be resumed. It can be deleted, so a user can clean
                // the store. See `D-a-bad-session-file-is-one-row`.
                continue;
            };
            if !wanted(&summary) {
                continue;
            }
            if lock::is_locked_elsewhere(&self.lock_path(&summary.id), summary.id.as_str()) {
                tracing::debug!(
                    id = summary.id.as_str(),
                    "a session is open in another process; the search moved past it"
                );
                continue;
            }
            return Ok(Some(summary.id));
        }
        Ok(None)
    }

    /// The lock file for one session.
    fn lock_path(&self, id: &SessionId) -> PathBuf {
        self.root.join(format!("{}.lock", id.as_str()))
    }

    /// Take the advisory lock for one session.
    ///
    /// `SessionError::Busy` names the session when another process holds it.
    /// `SessionError::LockUnsupported` names the path when the filesystem cannot lock, and then
    /// the run stops. A warning that continued would fail open.
    ///
    /// Every write path takes this: a create, a resume, and a fork of the target it writes. A
    /// read-only path takes no lock, so `list` and `show` always work.
    pub fn lock(&self, id: &SessionId) -> Result<SessionLock, SessionError> {
        let path = self.lock_path(id);
        if let Some(parent) = path.parent() {
            // A store that cannot even hold its directory cannot hold a lock, so the refusal
            // names the lock path rather than leaking a create error.
            create_private_dir(parent)
                .map_err(|_| SessionError::LockUnsupported { path: path.clone() })?;
        }
        lock::take_lock(&path, id.as_str())
    }

    /// Delete one session file. Every branch in the file goes with it.
    ///
    /// It removes the file and every `<id>.*.sidecar` beside it. It does not overwrite the
    /// bytes, so a recovery tool may still find them. It does not remove a session forked
    /// from this one, because a fork is its own file.
    pub fn delete(&self, session_id: &SessionId) -> Result<(), SessionError> {
        let path = self.session_path(session_id.as_str());
        std::fs::remove_file(&path).map_err(|e| io_error(path.as_path(), e))?;
        // Remove any sidecar files that belong to this session too.
        let prefix = format!("{}.", session_id.as_str());
        if let Ok(dir) = std::fs::read_dir(&self.root) {
            for entry in dir.flatten() {
                let sidecar = entry.path();
                let is_sidecar = sidecar.extension().and_then(|e| e.to_str()) == Some("sidecar");
                let matches = sidecar
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(&prefix))
                    .unwrap_or(false);
                // The lock file goes too. `flock` dies with the process, so the file left behind
                // holds nothing, and a delete that left it would leave a name in the store for a
                // session that no longer exists. A live drive showed the leftover after a crash.
                let is_lock = sidecar.extension().and_then(|e| e.to_str()) == Some("lock");
                if (is_sidecar || is_lock) && matches {
                    let _ = std::fs::remove_file(&sidecar);
                }
            }
        }
        Ok(())
    }

    /// Fork a session at `from`. Copy the branch that ends at `from` into a new
    /// file with `new_id`. The original file is not changed. Return a writer on
    /// the new file.
    ///
    /// The first copied record is **re-parented** onto the new header id. It kept its old
    /// parent before, which resolved only because a native header happens to be minted as
    /// `r0`. An imported file has a hex header id, and then the copied record pointed at an
    /// id the new file does not hold. See section 7b.
    pub fn fork(
        &self,
        from_path: &Path,
        from: &RecordId,
        new_id: &SessionId,
    ) -> Result<SessionWriter, SessionError> {
        let read = SessionReader::read(from_path)?;
        let chain = walk_chain(&read.entries, from, Some(&read.header_id))?;

        create_private_dir(&self.root)?;
        let new_path = self.session_path(new_id.as_str());
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&new_path)
            .map_err(|e| io_error(new_path.as_path(), e))?;
        set_owner_only(&new_path)?;
        let mut writer = SessionWriter::with_sink(new_path, Box::new(file));
        // Every id the copied branch carries. The writer must not mint one of them, and a
        // count cannot see that, because a branch is not contiguous. See section 7a.
        for entry in &chain {
            writer.known_ids.insert(entry.id.0.clone());
        }
        // Write a fresh header for the new file, then copy the branch verbatim.
        let header = Record::Session {
            version: read.header.version,
            cwd: read.header.cwd,
            approval: read.header.approval,
            sandbox: read.header.sandbox,
            session_id: Some(new_id.as_str().to_string()),
            forked_from: Some(ForkOrigin {
                session_id: read.header.session_id.clone(),
                record_id: from.clone(),
            }),
        };
        let header_id = writer.mint_id();
        writer.write_entry(&Entry {
            id: header_id.clone(),
            parent_id: None,
            timestamp: now_timestamp(),
            record: header,
        })?;
        writer.head = Some(header_id.clone());
        for (index, entry) in chain.into_iter().enumerate() {
            let entry = if index == 0 {
                // Re-parent onto the new header. The rule does not depend on any id being
                // `r0`, so an imported file forks as well as a native one.
                Entry {
                    parent_id: Some(header_id.clone()),
                    ..entry
                }
            } else {
                entry
            };
            writer.write_entry(&entry)?;
            writer.head = Some(entry.id.clone());
        }
        Ok(writer)
    }
}

/// Walk parent links from `head` to the root, and return the chain in file order.
///
/// A missing parent is an error, never a short list. A short list would drop the end of a
/// conversation, and the provider request would still look valid. `branch_messages` and
/// `fork` both had `None => break` here. See section 6b.
fn walk_chain(
    entries: &[Entry],
    head: &RecordId,
    root: Option<&RecordId>,
) -> Result<Vec<Entry>, SessionError> {
    let map: HashMap<&RecordId, &Entry> = entries.iter().map(|e| (&e.id, e)).collect();
    let mut chain = Vec::new();
    let mut cursor = head.clone();
    loop {
        // The header is not in `entries`, because `read_from` consumes the first line before
        // its loop. So a caller that read a file names the header id here, and a chain that
        // reaches it has reached the root. A caller with hand-built entries passes `None`,
        // and then every parent must resolve inside `entries`.
        if root == Some(&cursor) {
            break;
        }
        let Some(entry) = map.get(&cursor) else {
            // The first lookup is the requested head, so a caller that asked for a record
            // the file does not hold gets a different error from a chain with a hole.
            return Err(match chain.last() {
                None => SessionError::NoSuchRecord { id: cursor },
                Some(child) => SessionError::Orphan {
                    child: (*child as &Entry).id.clone(),
                    parent: cursor,
                },
            });
        };
        chain.push(*entry);
        match &entry.parent_id {
            // The header is not in `entries`, so the walk ends at a record with no parent.
            None => break,
            Some(parent) => cursor = parent.clone(),
        }
    }
    chain.reverse();
    Ok(chain.into_iter().cloned().collect())
}

/// Does this io message say the path already exists?
///
/// `SessionError::Io` carries a string, so the kind is gone by the time a caller sees it.
/// `create_minted` needs to tell a taken id from a real failure, and it must not retry a
/// permission error eight times.
fn is_already_exists(message: &str) -> bool {
    message.contains("File exists")
        || message.contains("already exists")
        || message.contains("AlreadyExists")
}

/// Four hex characters per attempt, drawn from the clock and the process id.
///
/// `rho-core` has no random dependency, and this is not a secret. The suffix only has to keep
/// two sessions in one second apart, and the exclusive create in section 7c catches the rest.
/// See section 9, which says the suffix is not an access control.
fn random_suffixes() -> impl Iterator<Item = u16> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let mut state = nanos ^ (u64::from(std::process::id()) << 17) ^ 0x9e37_79b9_7f4a_7c15;
    std::iter::repeat_with(move || {
        // xorshift64. Enough for a collision suffix, and it needs no crate.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 24) as u16
    })
}

/// Rebuild the message list along the branch that ends at `head`. Walk parent
/// links from `head` to the root. Reverse the walk. Return the messages in order.
///
/// A trailing `ToolCall` with no matching `ToolResult` is repaired with a synthetic
/// error result, so the rebuilt list holds a complete pairing and the next provider
/// request is valid. See section 8a.
///
/// **A missing parent is an error, never a short list.** The old body broke out of the walk,
/// so a hole in the chain dropped the end of a conversation and the provider request still
/// looked valid. See section 6b.
///
/// `root` names the header record id, which is not in `entries`. A caller that read a file
/// passes `Some(&read.header_id)`. A caller with hand-built entries passes `None`, and then
/// every parent must resolve inside `entries`.
pub fn branch_messages(
    entries: &[Entry],
    head: &RecordId,
    root: Option<&RecordId>,
) -> Result<Vec<Message>, SessionError> {
    let chain = walk_chain(entries, head, root)?;

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
    Ok(messages)
}

/// Rewrite every stale result-handle preview in a rebuilt context.
///
/// A resumed context holds text like `<tool_result_preview handle="r-0001">`. The store behind
/// that handle died with the earlier run, and the per-session nonce enforces that on purpose.
/// So the model would call `read_tool_result`, get an error, and spend a turn learning that the
/// evidence is gone.
///
/// The rewrite says the evidence expired and keeps the byte count. The model then re-runs the
/// command instead. **The file on disk does not change**, because the file is append-only. Only
/// the rebuilt context does. See `D-a-stale-result-handle-expires-on-resume`.
pub fn expire_stale_result_handles(messages: &mut [Message]) {
    for message in messages {
        for block in &mut message.content {
            expire_in_block(block);
        }
    }
}

/// The marker a stored result preview opens with.
const PREVIEW_OPEN: &str = "<tool_result_preview";

/// Rewrite one block, and every block nested inside a tool result.
fn expire_in_block(block: &mut ContentBlock) {
    match block {
        ContentBlock::Text { text } if text.contains(PREVIEW_OPEN) => {
            *text = expired_preview(text);
        }
        ContentBlock::ToolResult { content, .. } => {
            for nested in content {
                expire_in_block(nested);
            }
        }
        _ => {}
    }
}

/// The replacement text for one stale preview.
///
/// It keeps the preview head the record already holds, because that is real evidence the model
/// read once. It removes only the promise that the handle still works.
fn expired_preview(text: &str) -> String {
    let stored_bytes = attribute(text, "stored_bytes").unwrap_or_else(|| "an unknown".to_string());
    let head = between_tags(text);
    format!(
        "{head}\n[rho stored {stored_bytes} bytes of this result in an earlier run. The evidence \
         expired when that run ended, so no handle can read it. Run the command again if you \
         need the rest.]"
    )
}

/// One attribute value from the preview tag.
fn attribute(text: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = text.find(&needle)? + needle.len();
    let rest = &text[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// The text between the preview tags, which is the head the model already read.
fn between_tags(text: &str) -> String {
    let Some(open_end) = text.find('>') else {
        return String::new();
    };
    let rest = &text[open_end + 1..];
    let end = rest.find("</tool_result_preview>").unwrap_or(rest.len());
    rest[..end].trim().to_string()
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
///
/// **It folds the stream. It does not record it.** The file holds messages, so a resume needs
/// no second fold. See `D-recorder-consumes-events`.
pub struct SessionRecorder {
    log: SessionLog,
    /// The blocks of the assistant turn now streaming, keyed by the provider block index.
    ///
    /// The index orders them, so a replay sends the blocks back in the order the provider
    /// produced them. Without this the recorder wrote no assistant message at all, and a
    /// resume replayed a `ToolResult` that matched no `ToolCall`. See
    /// `D-a-recorder-writes-the-assistant-turn`.
    turn: BTreeMap<u32, TurnBlock>,
    /// Calls whose `ToolCall` block is on disk and whose result is not.
    ///
    /// A cancel completes each one with a synthetic error result, so the file never holds half
    /// a pairing. See `D-cancel-keeps-the-session-open`.
    awaiting_result: Vec<(String, String)>,
    /// Calls a `ToolStart` announced with no `ToolCall` block on disk.
    ///
    /// A provider that emits no tool-call stream event lands here, and so does a caller that
    /// drives `ToolStart` directly. A cancel then writes the call with an empty argument
    /// object, because nothing better is known.
    unwritten_calls: Vec<(String, String)>,
}

/// One block of the assistant turn being folded.
enum TurnBlock {
    Text(String),
    /// Reasoning text, and the provider payload that replays it.
    Thinking {
        text: String,
        state: Option<crate::ProviderState>,
    },
    Call {
        id: String,
        name: String,
        /// `None` until `ToolCallEnd` states the parsed arguments.
        arguments: Option<serde_json::Value>,
        state: Option<crate::ProviderState>,
    },
}

impl TurnBlock {
    /// The name of this block kind, for a warning that names what it dropped.
    fn kind(&self) -> &'static str {
        match self {
            TurnBlock::Text(_) => "text",
            TurnBlock::Thinking { .. } => "thinking",
            TurnBlock::Call { .. } => "tool_call",
        }
    }

    /// The content block this turn block becomes on disk, or `None` when it holds nothing.
    fn into_content(self) -> Option<ContentBlock> {
        match self {
            TurnBlock::Text(text) if text.is_empty() => None,
            TurnBlock::Text(text) => Some(ContentBlock::Text { text }),
            // A payload means the provider needs the reasoning echoed back, so the block must
            // be able to travel. With no payload it is history for the reader alone.
            TurnBlock::Thinking { text, state } => match state {
                Some(state) => Some(ContentBlock::ReasoningReplay {
                    text,
                    state: Some(state),
                }),
                None if text.is_empty() => None,
                None => Some(ContentBlock::ReasoningTrace { text }),
            },
            TurnBlock::Call {
                id,
                name,
                arguments,
                state,
            } => Some(ContentBlock::ToolCall {
                id,
                name,
                // An empty object stands in only when the provider never completed the call.
                // A guessed argument set would be replayed as if the model had sent it.
                arguments: arguments.unwrap_or_else(|| serde_json::json!({})),
                state,
            }),
        }
    }
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
            turn: BTreeMap::new(),
            awaiting_result: Vec::new(),
            unwritten_calls: Vec::new(),
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

    /// Fold one agent event.
    ///
    /// The recorder holds the parts of the current assistant turn. `TextDelta` appends text.
    /// `ThinkingEnd` closes a reasoning block with its replay payload. `ToolCallEnd` completes
    /// a call with its parsed arguments. `TurnEnd` writes **one** `Message` record with role
    /// `Assistant`, in provider block order. An empty turn writes nothing.
    ///
    /// So the file order is always the call, then its result, and a `ToolCall` on disk with no
    /// `ToolResult` is impossible on the run path. See section 6d.
    ///
    /// Redaction runs on every block before it reaches the file. Return an id when it writes.
    pub fn observe(&mut self, event: &AgentEvent) -> Option<RecordId> {
        match event {
            AgentEvent::TurnStart => {
                self.turn.clear();
                None
            }
            // A provider may begin the assistant message after the turn started. The blocks of
            // the previous turn are already written, so this only guards a provider that emits
            // no `TurnStart`.
            AgentEvent::Stream(StreamEvent::MessageStart { .. }) => {
                self.turn.clear();
                None
            }
            AgentEvent::Stream(StreamEvent::TextStart { index }) => {
                self.turn.insert(*index, TurnBlock::Text(String::new()));
                None
            }
            AgentEvent::Stream(StreamEvent::TextDelta { index, delta }) => {
                match self
                    .turn
                    .entry(*index)
                    .or_insert_with(|| TurnBlock::Text(String::new()))
                {
                    TurnBlock::Text(text) => text.push_str(delta),
                    // A provider that reuses an index for two kinds is a provider defect, and
                    // dropping the delta is better than corrupting the other block.
                    other => tracing::warn!(
                        index = *index,
                        kind = other.kind(),
                        "a text delta arrived for a block of another kind; it was dropped"
                    ),
                }
                None
            }
            AgentEvent::Stream(StreamEvent::ThinkingStart { index }) => {
                self.turn.insert(
                    *index,
                    TurnBlock::Thinking {
                        text: String::new(),
                        state: None,
                    },
                );
                None
            }
            AgentEvent::Stream(StreamEvent::ThinkingDelta { index, delta }) => {
                match self
                    .turn
                    .entry(*index)
                    .or_insert_with(|| TurnBlock::Thinking {
                        text: String::new(),
                        state: None,
                    }) {
                    TurnBlock::Thinking { text, .. } => text.push_str(delta),
                    other => tracing::warn!(
                        index = *index,
                        kind = other.kind(),
                        "a thinking delta arrived for a block of another kind; it was dropped"
                    ),
                }
                None
            }
            AgentEvent::Stream(StreamEvent::ThinkingEnd { index, state }) => {
                if let Some(TurnBlock::Thinking { state: slot, .. }) = self.turn.get_mut(index) {
                    // Verbatim. A rewritten payload cannot replay, so rule 9 keeps it whole.
                    *slot = state.clone();
                }
                None
            }
            AgentEvent::Stream(StreamEvent::ToolCallStart { index, id, name }) => {
                self.turn.insert(
                    *index,
                    TurnBlock::Call {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: None,
                        state: None,
                    },
                );
                None
            }
            AgentEvent::Stream(StreamEvent::ToolCallEnd {
                index,
                arguments,
                state,
            }) => {
                if let Some(TurnBlock::Call {
                    arguments: slot,
                    state: payload,
                    ..
                }) = self.turn.get_mut(index)
                {
                    *slot = Some(arguments.clone());
                    *payload = state.clone();
                }
                None
            }
            AgentEvent::TurnEnd { .. } => self.flush_turn(),
            AgentEvent::ToolStart { id, name, .. } => {
                // The call is already on disk when the stream announced it. Only a provider
                // that emits no tool-call event, or a caller driving this directly, lands here.
                let known = self.awaiting_result.iter().any(|(open, _)| open == id);
                if !known {
                    self.unwritten_calls.push((id.clone(), name.clone()));
                }
                None
            }
            AgentEvent::ToolEnd { id, output } => {
                self.awaiting_result.retain(|(open, _)| open != id);
                self.unwritten_calls.retain(|(open, _)| open != id);
                // **The result is wrapped in a `ToolResult` block, and it names its call.**
                // The old body wrote the raw output blocks, so the record carried no
                // `tool_call_id`. The live context wraps it, in `Agent::finish_tool`, so the
                // recorded conversation had a different shape from the one the model saw. A
                // resume then sent a tool message a provider cannot match to any call, and
                // `branch_messages` invented a synthetic error result beside the real one.
                let message = Message {
                    role: Role::Tool,
                    content: vec![redact_block(&ContentBlock::ToolResult {
                        tool_call_id: id.clone(),
                        content: output.content.clone(),
                        is_error: output.is_error,
                    })],
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

    /// Write the assistant message the turn built, and forget the parts.
    ///
    /// It writes nothing for a turn with no content, so a file gains no blank message.
    fn flush_turn(&mut self) -> Option<RecordId> {
        let parts = std::mem::take(&mut self.turn);
        let mut content = Vec::new();
        for block in parts.into_values() {
            if let TurnBlock::Call { id, name, .. } = &block {
                self.awaiting_result.push((id.clone(), name.clone()));
            }
            if let Some(block) = block.into_content() {
                content.push(redact_block(&block));
            }
        }
        if content.is_empty() {
            return None;
        }
        self.log.record(
            Record::Message {
                message: Message {
                    role: Role::Assistant,
                    content,
                },
            },
            None,
        )
    }

    /// Write an explicit title as a `Name` leaf record.
    ///
    /// An empty or blank title is refused, so a row never shows a blank name. A title costs no
    /// model call. See `D-a-session-title-costs-nothing`.
    pub fn record_name(&mut self, title: &str) -> Result<Option<RecordId>, SessionError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(SessionError::EmptyTitle);
        }
        Ok(self.log.record(
            Record::Name {
                title: title.to_string(),
            },
            None,
        ))
    }

    /// On a cancel, write the partial assistant message, complete any open tool pairing, then
    /// write the stop record. See `D-cancel-keeps-the-session-open`.
    ///
    /// The partial message carries the **real** arguments the provider sent, because the turn
    /// holds them. The old body invented an empty object for every open call, so a resume
    /// replayed a call the model never made.
    pub fn record_cancel(&mut self) -> Option<RecordId> {
        // Whatever the turn already built goes to disk, including a completed tool call.
        let mut last = self.flush_turn();
        // A call a `ToolStart` announced with no block on disk. An empty object stands in,
        // because nothing better is known, and the id and the name keep the pairing valid.
        let unwritten = std::mem::take(&mut self.unwritten_calls);
        if !unwritten.is_empty() {
            let calls = unwritten
                .iter()
                .map(|(id, name)| ContentBlock::ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: serde_json::json!({}),
                    // A guessed payload would be replayed as if the provider had sent it.
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
            self.awaiting_result.extend(unwritten);
        }
        for (id, _) in std::mem::take(&mut self.awaiting_result) {
            last = self.log.record(
                Record::Message {
                    message: synthetic_error_result(&id, "the tool call was cancelled"),
                },
                None,
            );
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

    /// Write the `Closed` record, so the file states its own close.
    ///
    /// A run that ended on its own calls this. A cancel does not, because a cancel keeps the
    /// session open and usable. See `D-cancel-keeps-the-session-open`. A closed file is never
    /// offered by `newest_open`, so a crash offers only a session that really has no close.
    ///
    /// It is idempotent, and an ephemeral log does nothing.
    pub fn close(&mut self) -> Result<(), SessionError> {
        match &mut self.log {
            SessionLog::Off => Ok(()),
            SessionLog::File(writer) => writer.close(),
        }
    }
}

#[cfg(test)]
mod byte_counter_tests {
    //! The counter that bounds the cost of measuring a record.
    //!
    //! Without an early stop, measuring an eight-megabyte record would walk all of it to learn
    //! that it exceeds sixty-four kilobytes. A mutation review found that the stop and a final
    //! length comparison masked each other, so the comparison went and this pins the stop.
    //!
    //! **These tests were written once and lost.** A review agent whose isolation failed
    //! restored an older copy of this file over them, and the loss was invisible: a missing test
    //! fails nothing. A commit message then claimed `a_counter_stops_at_its_limit` existed while
    //! it did not. So they are here again, and an audit of every test name claimed in a commit
    //! message now runs against the tree.

    use super::*;

    /// The boundary itself: exactly the limit fits, and one byte more does not.
    ///
    /// A mutation review changed `>` to `>=`. A record that encodes to exactly the cap must be
    /// kept, because the write path would have written it.
    #[test]
    fn a_counter_accepts_exactly_its_limit_and_no_more() {
        let mut counter = ByteCounter {
            count: 0,
            limit: 100,
        };
        assert!(
            counter.write(&[b'x'; 100]).is_ok(),
            "exactly the limit fits, so a record at the cap is kept"
        );
        assert_eq!(counter.count, 100);
        assert!(
            counter.write(&[b'x'; 1]).is_err(),
            "one byte past the limit does not"
        );
    }

    #[test]
    fn a_counter_stops_at_its_limit() {
        let mut counter = ByteCounter {
            count: 0,
            limit: 4096,
        };
        let chunk = [b'x'; 1024];
        let mut writes = 0;
        let mut failed = false;
        for _ in 0..1000 {
            writes += 1;
            if counter.write(&chunk).is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed, "the counter must stop rather than count for ever");
        assert!(
            writes <= 6,
            "it stops just past the limit, not at the end of the input: {writes} writes"
        );
        assert!(
            counter.count <= 4096 + chunk.len(),
            "the work is bounded by the limit, not by the input: {} bytes",
            counter.count
        );
    }

    #[test]
    fn a_counter_under_its_limit_accepts_every_write() {
        let mut counter = ByteCounter {
            count: 0,
            limit: 4096,
        };
        assert!(counter.write(&[b'y'; 100]).is_ok());
        assert!(counter.write(&[b'y'; 100]).is_ok());
        assert_eq!(counter.count, 200);
    }
}
