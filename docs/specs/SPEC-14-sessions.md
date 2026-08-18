# SPEC-14 — Sessions

Status: draft for sprint 2.
Owning crate: `rho-core`, module `session`.
Features: F-50, F-51, F-52, F-53, F-54.

## 1. What a session file is

A session file is a log of one conversation on disk. It is JSONL. One record sits on
one line. The file is append-only. rho never rewrites a line, and never edits an
earlier byte.

Every record carries an id, a parent id, and a timestamp. So the records form a tree,
not a list. A branch is a walk of parent links, not a copy of the file. This is the
whole reason for the parent pointer. See `ADR-004`.

The format matches the shape that pi writes, on purpose, so the import path in section
9 is a field-by-field map and not a parser. rho keeps its own record set and its own
version. It does not adopt pi's model. See decision D-001.

## 2. The record set

```rust
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{AgentStopReason, Message, Usage};

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
```

The `Session` header names the format version, so a reader can refuse a version it does
not know. The `cwd` records where the session ran. The `approval` and `sandbox` fields
name the resolved modes the session ran under. A resume reads them, so a read-only
session never resumes under a wider mode by accident. See section 8a. The two fields are
strings, not `rho-core` enums, because the record stays a pure `serde` derive and
`rho-core` must not depend on `rho-config`. The value set for `approval` is `read-only`,
`ask`, and `allow-all`. The value set for `sandbox` is `off`, `confined`, and `strict`.
The `Message` record reuses the real
`rho_core::Message` type, so the on-disk content model and the in-memory content model
are the one type. See `crates/rho-core/src/content.rs`.

## 3. The codec seam

The JSON codec is one module with two functions. They are generic over the record type.
Both codecs satisfy the same signature. So the feature switch changes no caller and no
record type.

```rust
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Encode one record to a single JSONL line. It adds no trailing newline.
pub fn encode<T: Serialize>(value: &T) -> Result<String, SessionError>;

/// Decode one JSONL line to a record.
pub fn decode<T: DeserializeOwned>(line: &str) -> Result<T, SessionError>;
```

The default codec is `serde_json`. The `fast-json` cargo feature selects `sonic-rs`
instead. The feature is off by default.

The record types stay pure `serde` derives. No record type carries a codec-specific
attribute. An attribute from one codec would break the other. So the types name neither
codec.

### Why serde_json is the default, and sonic-rs is optional

The owner benchmarked the two candidates, and rejected `simd-json`. The full table and
the command are in `ADR-005`. `sonic-rs` wins on encode and on a large untyped decode.
It costs a 23 percent larger release binary. It costs a dependency tree of 102 lines
against 21. It falls back to a slow path on a target that is neither `x86_64` nor
`aarch64`. So it is a feature, not the default. `simd-json` lost on small records.

Selected numbers, from `ADR-005`, on a real 1848-record session and a 50000-record
corpus:

| path | serde_json | sonic-rs |
| --- | --- | --- |
| typed decode, session | 2.01 ms | 1.87 ms |
| typed encode, session | 2.15 ms | 0.76 ms |
| untyped value, large | 16.02 ms | 6.13 ms |

### The compatibility rule

Both codecs must produce a byte-identical line for the same record. Each must read the
other's output. So a file written under one codec loads under the other.

A named test proves it: `both_codecs_agree_byte_for_byte`. CI must run the codec tests
with `fast-json` on and with `fast-json` off. So neither path drifts.

### The record-size rule

A codec is fast only when a record is small. A tool result of ten megabytes must not
sit in the file verbatim. So the writer caps one record at `MAX_RECORD_BYTES`. The cap
is defined per record kind, because one rule cannot fit every shape. A `ToolResult` is
text, a `ToolCall` is a structured call, and an assistant `Message` is text too, so each
kind caps in the way that keeps it valid.

- A `ToolResult` block over the cap stores the head of the text, up to the cap. The
  writer stores a note in place of the dropped tail. The note states the full byte
  count. The full payload spills to a sidecar file under the session directory. The
  stored head plus the note is a valid `ContentBlock::Text`, so a reader needs no
  special case.
- An assistant `Message` text block over the cap caps the same way, with a head and a
  note.
- A `ToolCall` over the cap is never rewritten into a text note. That would drop
  `tool_call_id` and `name`, and break the pairing invariant that every `ToolCall` has a
  matching `ToolResult`. Instead the writer keeps the `id`, the `name`, and the
  `tool_call_id`, caps the oversize string values inside `arguments`, and spills the full
  `arguments` to a sidecar file. So the record stays a valid `ToolCall`.
- A `Session`, a `ModelChange`, a `Usage`, a `Stop`, and a `Closed` record are bounded
  by construction, so they need no cap.

So no written record of any kind exceeds the cap, and a `ToolCall` always stays a
`ToolCall`.

```rust
/// The largest single record written to the file, in bytes.
pub const MAX_RECORD_BYTES: usize = 64 * 1024;
```

## 4. The writer, the reader, and the store

```rust
use std::path::Path;

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
}

/// Appends records to one session file. It owns the open file handle.
pub struct SessionWriter { /* private */ }

impl SessionWriter {
    /// Append one record. Mint an id. Link the parent. Stamp the time. Write one
    /// line. Flush. Return the new id. Never rewrite an earlier byte.
    pub fn append(
        &mut self,
        record: Record,
        parent: Option<RecordId>,
    ) -> Result<RecordId, SessionError>;

    /// The id of the last record written. A later append links to it by default.
    pub fn head(&self) -> Option<RecordId>;

    /// The file path.
    pub fn path(&self) -> &Path;

    /// Write the `Closed` record. Idempotent. A second call writes nothing.
    pub fn close(&mut self) -> Result<(), SessionError>;
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
    pub fn read(path: &Path) -> Result<ReadResult, SessionError>;
}

/// The set of session files under one directory.
pub struct SessionStore { /* private */ }

impl SessionStore {
    /// A store rooted at a directory. The directory holds one file per session.
    pub fn new(root: impl Into<PathBuf>) -> Self;

    /// Create a new session file. Write the header. Return a writer. This is the
    /// open path and the new path. `approval` and `sandbox` name the resolved
    /// modes, and go into the header record.
    pub fn create(
        &self,
        session_id: &str,
        cwd: &Path,
        approval: &str,
        sandbox: &str,
    ) -> Result<SessionWriter, SessionError>;

    /// Open an existing file to append more records. Used by resume and fork.
    pub fn append_to(&self, path: &Path) -> Result<SessionWriter, SessionError>;

    /// List sessions with a cheap summary. Read only the first line of each file.
    /// The first line obeys the same `MAX_LINE_BYTES` cap as `read`.
    pub fn list(&self) -> Result<Vec<SessionSummary>, SessionError>;

    /// Delete one session file. Every branch in the file goes with it.
    pub fn delete(&self, session_id: &str) -> Result<(), SessionError>;

    /// Fork a session at `from`. Copy the branch that ends at `from` into a new
    /// file with `new_id`. The original file is not changed. Return a writer on
    /// the new file.
    pub fn fork(
        &self,
        from_path: &Path,
        from: &RecordId,
        new_id: &str,
    ) -> Result<SessionWriter, SessionError>;
}

/// Rebuild the message list along the branch that ends at `head`. Walk parent
/// links from `head` to the root. Reverse the walk. Return the messages in order.
pub fn branch_messages(entries: &[Entry], head: &RecordId) -> Vec<Message>;
```

## 5. How a session opts in or out

The session log is a consumer of the event stream, not a field inside `Session`. Every
frontend already consumes `AgentEvents` from `Session::prompt`. See F-02 and `ADR-003`.
So a recorder folds those events into records. This needs no change to the existing
`Session` or `SessionConfig`. See decision D-042.

```rust
use crate::{AgentEvent, ContentBlock};

/// A session's persistence. `Off` writes nothing. `File` appends to a writer.
pub enum SessionLog {
    Off,
    File(SessionWriter),
}

impl SessionLog {
    /// Append one record. On a write failure, degrade to ephemeral with a warning.
    /// Return the id when written. Return `None` when ephemeral or degraded.
    pub fn record(&mut self, record: Record, parent: Option<RecordId>) -> Option<RecordId>;

    /// True when this log writes nothing.
    pub fn is_ephemeral(&self) -> bool;
}

/// Folds the agent event stream into session records.
pub struct SessionRecorder { /* private */ }

impl SessionRecorder {
    /// Build a recorder over a log. `SessionLog::Off` gives an ephemeral recorder.
    pub fn new(log: SessionLog) -> Self;

    /// Record the user's prompt as a `Message` record. Redact the content first.
    pub fn record_prompt(&mut self, input: &[ContentBlock]) -> Option<RecordId>;

    /// Fold one agent event. Write an assistant message at a turn end, a tool
    /// result at a tool end, a usage record on a usage event, and a stop record at
    /// the agent end. Redact every tool argument first. Return an id when it writes.
    pub fn observe(&mut self, event: &AgentEvent) -> Option<RecordId>;

    /// On a cancel, complete any open tool pairing, then write the stop record.
    /// See section 8.
    pub fn record_cancel(&mut self) -> Option<RecordId>;

    /// True when the log is ephemeral, or degraded to ephemeral.
    pub fn is_ephemeral(&self) -> bool;
}
```

A caller that wants persistence builds `SessionLog::File(writer)` and feeds every event
to `observe`. A caller that wants ephemeral mode builds `SessionLog::Off`. That is the
opt-out for F-53. The config key `ephemeral` and the absence of `session-file` both
select `Off`. See `SPEC-13`.

### 5a. The redaction function that a record depends on

The recorder masks a credential-shaped tool argument before it writes a record. A
message content block never holds a `Secret` type, but a `ToolCall.arguments` value is
free-form JSON, so it can carry a key. The recorder passes every argument value through a
new function in `rho-redact`. This is decision D-043.

```rust
/// Mask every value under a key that `looks_like_a_secret` flags, anywhere in a JSON
/// value. Recurse into every object and every array. Return a new value.
pub fn redact_json_secrets(value: &serde_json::Value) -> serde_json::Value;
```

- It masks a value under a flagged key. The masked value is the string `"***"`.
- It reads a key name only, never a value, because a value cannot be recognised
  reliably. This matches `looks_like_a_secret`, which already reads the name.
- It keeps every key name, every value under an unflagged key, and the whole tree shape.

**This function is new work for stage T5. It does not exist today.** `crates/rho-redact/src/lib.rs`
exports `looks_like_a_secret`, `sanitize_text`, and `sanitize_line` only. A reviewer
proved that a call to `redact_json_secrets` fails with `E0425`. This is the same family
as `confine`, the path boundary that stayed `todo!()` through a green stage, so it is
named here as required new surface. **No session record may be written before this
function exists.** `rho-redact` is the one home for redaction, per decision D-026, so no
second implementation may live outside it.

## 6. Storage cost, and a truncated last line

The append path cost is dominated by the write, not by the codec. One record is about
200 bytes. An encode of that record costs roughly 100 ns. A write costs microseconds.
So the codec is not the bottleneck. The real efficiency rules are about the write.

Four rules govern `store`.

- Use a buffered writer. Append one record as one write.
- Do not `fsync` per record. A per-record `fsync` costs a disk seek every record, and a
  session log does not need that durability.
- Cap the size of a tool result that reaches the file. See section 3.
- Keep a header record. So `list` stays cheap without a whole-file read. See section 8.

The append path never rewrites the file. This is F-50. An earlier record is already
written, so a later append never touches it.

A crash can lose a buffered tail, and can cut the last line in half. The reader handles
this.

- `SessionReader::read` reads line by line.
- It decodes every whole line into an entry.
- If the last line fails to decode, it drops that line only. It keeps every whole
  record before it.
- It sets `truncated_tail` to true.
- The resume path logs one warning when `truncated_tail` is true.

So a half-written tail never fails a resume, and never discards a whole record. A clean
`close` flushes the buffer, so a closed file has no buffered tail to lose.

### 6a. A bounded read

A reader must survive a file it did not write. The 64 KB write cap in section 3 bounds
what rho writes, but it does not protect the read path. A corrupt file, or a hostile
file, can hold a single multi-megabyte line. That repeats defect 7, where an unbounded
`bash` line reader turned 8 MB of output into 805 MB of memory. See decision D-016.

So `SessionReader::read` and `SessionStore::list` cap one line at `MAX_LINE_BYTES`.

- The reader reads at most `MAX_LINE_BYTES` for one line.
- A line that reaches the cap is a `SessionError::Decode`. It is a decode error, never an
  unbounded allocation.
- The reader never grows a buffer without a bound, whatever the file holds.

The read cap is larger than `MAX_RECORD_BYTES`, so a file written by rho always loads,
and a file written by another tool, for example a pi file, still loads unless one line
is pathological. So the bound protects memory without rejecting an honest file.

## 7. Branching

A branch is a new leaf on the tree.

- The user navigates to an earlier record.
- The next append names that record as its parent.
- The original branch stays on disk. rho deletes nothing.
- `branch_messages` rebuilds one branch by walking parent links from a chosen head to
  the root.

So two branches share their common prefix on disk. The file grows by the new records
only. This is F-52.

## 8. The session operations

Each operation below has a verbatim signature above. This section states its effect on
the file, its effect on a running turn, and its ACP method.

| Operation | Signature | Effect on the file | Effect on a turn | ACP method |
| --- | --- | --- | --- | --- |
| open, new | `SessionStore::create` | Write the header record. | Start a fresh `Session`. | `session/new` |
| store | `SessionLog::record`, `SessionWriter::append` | Append one record and flush. | None. It runs during a turn. | none, it is implicit |
| resume | `SessionReader::read`, `branch_messages`, `SessionStore::append_to` | Read every whole record. Append after them. | Rebuild the context, then continue. | `session/load`, `session/resume` |
| close | `SessionWriter::close` | Write the `Closed` record. | None. The turn is already done. | `session/close` |
| cancel | `CancelToken::cancel`, `SessionRecorder::record_cancel` | Complete open pairings, then write `Stop`. | Stop the turn. Keep the session usable. | `session/cancel` |
| list | `SessionStore::list` | Read the first line of each file. | None. | `session/list` |
| delete | `SessionStore::delete` | Remove the file and its branches. | None. | `session/delete` |
| fork | `SessionStore::fork` | Copy a branch to a new file. | None. Start from the copy. | none, load then new |

### The ACP methods are planned, and load differs from resume

The ACP methods in the table above are not all live yet. `SPEC-06` marks `session/load`,
`session/resume`, `session/list`, `session/delete`, and `session/close` as planned, after
the TUI. So this spec states the file effect of each operation now, and `rho-acp` wires
the method when the ACP frontend stage lands. The `session/new`, `session/prompt`, and
`session/cancel` methods are the sprint-1 set that already exists.

`session/load` and `session/resume` are two methods, not one. `session/load` rebuilds a
session's state from the file and reports it to the client, without continuing the turn.
`session/resume` loads the file and then continues the conversation with a new prompt. So
a load is a read, and a resume is a read plus a continuation. Both read through
`SessionReader::read` and `branch_messages`, and both obey the widening rule in section
8a.

### 8a. Resume must not widen a permission

A session that ran read-only must not resume with a wider permission by accident. The
header record holds the resolved `approval` and `sandbox` modes, per section 2. A resume
reads them and compares them against the modes the current run would use.

- The `approval` names order from strict to permissive: `read-only`, then `ask`, then
  `allow-all`. A resume refuses a mode more permissive than the header names.
- The `sandbox` names order from strict to permissive: `strict`, then `confined`, then
  `off`. A resume refuses a mode more permissive than the header names.
- A resume that would widen either mode stops, unless the user passes the explicit
  override flag `--allow-widen`. The flag is the one way to widen on purpose. Over ACP,
  the override is an explicit request field, not a default.
- A resume that keeps or narrows either mode runs with no flag.

The approval names form a total order here, so the comparison is representable. This is
not the trait-object comparison that `SPEC-11` section 3 proved impossible. It compares
two stored mode names, not two arbitrary `ApprovalPolicy` objects.

### Resume repairs an unmatched tool call after a crash

A crash can leave the file with an assistant `Message` that carries a `ToolCall`, and no
matching `ToolResult`. The cancel path in the next subsection prevents this on a clean
cancel, but a crash writes no cancel record, and a truncated tail can drop a `ToolResult`
line. A provider rejects a call that has no result, so a resume must repair the pairing.

- A resume that finds a trailing `ToolCall` with no matching `ToolResult` completes it
  with a synthetic error `ToolResult`. The result is an error, and it says the call did
  not finish before a crash.
- So the rebuilt message list holds a matched pairing, and the next provider request is
  valid. rho completes the call rather than dropping it, so the model sees that the call
  was attempted.

### Cancel keeps the session open

Cancel is not close. It reuses the existing `CancelToken` in
`crates/rho-core/src/cancel.rs`. It invents no second mechanism. It stops the running
turn and leaves the session open and usable for the next prompt.

A cancel writes an exact record set.

- It writes the assistant message built so far, if the turn produced one.
- For any tool call in that message with no result yet, it writes a synthetic tool
  result. The result is an error, and it says the call was cancelled.
- It writes one `Stop` record with `AgentStopReason::Canceled`.

So a cancelled turn leaves no half-written tool pairing in the file. Every `ToolCall` on
disk has a matching `ToolResult`. This mirrors the loop rule that pairs `TurnStart` with
`TurnEnd`, in `crates/rho-core/src/agent.rs`.

### List reads one line per file

`list` reads only the first line of each file. The summary comes from the header
record, plus the file name and the filesystem metadata. So a list of 500 sessions reads
500 first lines, not 500 whole files. A human title is out of scope for the cheap list,
because a title needs a deeper scan.

### Delete and fork

`delete` removes one file. Every branch lives in that one file, so every branch goes
with it. A fork is a separate file with its own id, so `delete` on the parent does not
touch a fork.

`fork` copies the branch that ends at a chosen record into a new file. The new file
gets a new session id. The original file is not changed. ACP has no fork method, so a
client forks by a load followed by a new. See `SPEC-06`.

### Resume when the model or the tool set changed

Resume rebuilds the context from the file. The model or the tool set can differ from the
file's day.

- A model change appends a `ModelChange` record. The old records stay valid. The new
  turns run under the new model.
- A tool that a record names, but that the current tool set lacks, is still read. Its
  past `ToolCall` and `ToolResult` records are history, so they load. rho does not call
  a missing tool again. It just cannot repeat a call the model never makes now.
- A tool present now, but absent when the file was written, is available for the next
  turn. The past does not restrict the future tool set.

## 9. The pi import path

Feature F-54 lives in the crate `rho-session-import-pi`. It converts a pi session file
to rho's format. The conversion is one-way. The pi file is not changed.

A pi session file is JSONL at `~/.pi/agent/sessions/<project>/<stamp>_<uuid>.jsonl`. The
shape below is confirmed against a real file on disk.

- The first line is `{"type":"session","version":3,"id":...,"cwd":...}`.
- A later line is a message:
  `{"type":"message","id":...,"parentId":...,"timestamp":...,"message":{"role":...,"content":[...]}}`.
- Other later lines are `model_change`, `thinking_level_change`, `session_info`, and
  `custom`.

The mapping to rho records:

| pi record | rho record |
| --- | --- |
| `session` | `Record::Session`. rho writes its own version, not pi's `3`. |
| `message`, role `user` | `Record::Message`, `Role::User`. |
| `message`, role `assistant` | `Record::Message`, `Role::Assistant`. |
| `message`, role `toolResult` | `Record::Message`, `Role::Tool`. |
| `model_change` | `Record::ModelChange`. |

The content block mapping, inside a message:

| pi block | rho `ContentBlock` |
| --- | --- |
| `text` | `Text { text }`. |
| `thinking`, `thinkingSignature` | `Thinking { thinking, signature }`. |
| `toolCall` | `ToolCall { id, name, arguments }`. |
| a `toolResult` text | `ToolResult { tool_call_id, content, is_error }`. |

The `id`, the `parentId`, and the `timestamp` carry across, so the tree shape is kept.

**The import holds one invariant.** Every pi record maps one to one to a rho record,
only the known droppable set drops, and the tree shape survives. The known droppable set
is `thinking_level_change`, `session_info`, and `custom`. So no unknown record is
invented, no known record is lost silently, and every kept record keeps its parent link.

**What rho drops.** rho drops the record types it has no model for.

- `thinking_level_change` drops. rho has no thinking-level concept.
- `session_info` drops. rho has no title record in sprint 2.
- `custom` drops. It is a pi extension payload.
- A `thinkingSignature` from another provider may be stale. rho keeps the field but
  does not promise it replays.

## 10. Test cases

Storage:
- `n_appends_yield_exactly_n_lines` — for any N, N appends yield exactly N lines. The
  invariant, not one example.
- `append_returns_a_new_id_each_time` — two appends return two different ids.
- `head_reports_the_last_written_id` — `SessionWriter::head` returns the id of the last
  append, and `None` before the first append.
- `path_returns_the_open_file_path` — `SessionWriter::path` returns the file the writer
  opened.
- `the_append_path_never_rewrites_an_earlier_byte` — the bytes before a new record are
  byte-identical after the append. This is the append-only proof.
- `no_written_record_of_any_kind_exceeds_the_cap` — for any input, no written record of
  any kind exceeds `MAX_RECORD_BYTES`. The invariant over every record kind.
- `an_oversize_record_of_every_kind_stays_under_the_cap` — an oversize `ToolResult`, an
  oversize assistant `Message`, and an oversize `ToolCall` each write under the cap, and
  the `ToolCall` stays a `ToolCall` with its `tool_call_id`.
- `a_non_tool_result_record_over_the_cap_is_capped` — an oversize assistant message text
  block is capped with a head and a note, the same as a tool result.
- `both_codecs_agree_byte_for_byte` — for every `Record` variant, `serde_json` and
  `sonic-rs` encode it to the identical line, and each reads the other's output in both
  directions. This is a property over every variant, not one example. CI runs it with
  `fast-json` on and off.

Resume:
- `resume_reads_every_whole_record` — a clean file loads every record.
- `resume_rebuilds_the_messages_in_file_order` — `branch_messages` returns the messages
  in the order the file records them.
- `resume_recovers_after_a_truncated_last_line` — a file with a half-written last line
  loads every whole record and sets `truncated_tail`.
- `resume_warns_on_a_truncated_last_line` — the resume path logs one warning.
- `resume_after_a_model_change_appends_a_model_change_record` — a new model adds a
  record and keeps the old ones.
- `resume_reopens_the_file_with_append_to` — `SessionStore::append_to` returns a writer
  on the existing file, and a later append lands after the earlier records.
- `a_giant_line_does_not_exhaust_memory` — a file with a multi-megabyte line over
  `MAX_LINE_BYTES` returns a decode error and never allocates the whole line.
- `resume_refuses_an_unknown_version` — a header with a version the reader does not know
  returns `SessionError::Version`, and the resume stops.
- `resume_does_not_widen_a_read_only_session` — a session whose header names `read-only`
  refuses to resume under `allow-all` without `--allow-widen`, and runs under
  `read-only` with no flag.
- `resume_repairs_an_unmatched_tool_call_after_a_crash` — a file that ends with a
  `ToolCall` and no matching `ToolResult` rebuilds a matched pairing with a synthetic
  error result, so the next provider request is valid.

Branch:
- `a_branch_keeps_the_original_records` — a branch appends and deletes nothing.
- `a_branch_links_the_new_record_to_its_parent` — the new record names the chosen parent.
- `branch_messages_walks_one_branch_only` — a head resolves to its own branch, not a sibling.

Ephemeral and degrade:
- `ephemeral_mode_writes_no_file` — `SessionLog::Off` creates no file.
- `a_write_failure_degrades_to_ephemeral_with_a_warning` — a failing writer switches to
  ephemeral and the run continues. It never ends the run. See defect 9.
- `session_log_is_ephemeral_reports_off_and_degraded` — `SessionLog::is_ephemeral` is
  true for `Off` and true after a write failure degrades the log.
- `recorder_is_ephemeral_follows_its_log` — `SessionRecorder::is_ephemeral` is true when
  its log is ephemeral or degraded.

Lifecycle:
- `close_writes_a_closed_record` — close appends the `Closed` record.
- `close_is_idempotent` — a second close writes nothing.
- `cancel_keeps_the_session_open` — after a cancel the session accepts a new prompt.
- `cancel_writes_a_stop_record` — a cancel appends one `Stop` with `Canceled`.
- `cancel_leaves_no_half_written_tool_pairing` — every `ToolCall` on disk has a matching
  `ToolResult` after a cancel, after a crash with an unmatched call, and after a
  truncated tail that dropped a result line. The invariant holds on all three paths.
- `a_usage_record_round_trips` — a `Record::Usage` encodes and decodes back to the same
  `Usage`, so the usage counters survive a resume.
- `list_reads_only_the_first_line` — `list` on many files reads one line each.
- `delete_removes_the_file_and_its_branches` — the file and every branch are gone.
- `delete_does_not_touch_a_fork` — a fork survives a delete of its parent.
- `fork_copies_the_branch_and_keeps_the_original` — the new file holds the branch, and
  the original is byte-identical.

Redaction, the security core:
- `no_credential_reaches_the_file` — a message with a secret-shaped tool argument writes
  a masked value, and the raw value never appears in the file.
- `a_redacted_tool_argument_is_masked_on_the_way_in` — `record_prompt` and `observe`
  redact arguments through `rho-redact` before the record is written.
- `redact_json_secrets_masks_a_secret_keyed_field` — `rho_redact::redact_json_secrets`
  masks the value under a key that `looks_like_a_secret` flags, and keeps every other
  value, every key name, and the tree shape. This test guards the new function in
  section 5a.

Pi import:
- `every_pi_record_maps_one_to_one_or_drops_from_the_known_set` — the invariant over any
  pi file: every pi record maps one to one to a rho record, only the known set
  (`thinking_level_change`, `session_info`, `custom`) drops, and the tree shape survives
  because every kept record keeps its `parentId`. This one property replaces the three
  example tests for a text message, a tool call, and a kept parent pointer.
- `drops_a_pi_thinking_level_change` — a `thinking_level_change` is not imported.
- `leaves_the_original_pi_file_unchanged` — the pi file is byte-identical after import.

Every test uses `tempfile`. No test uses `sleep`. No test reads a real user directory.

## 11. Out of scope for sprint 2

- A compaction or summary of an old branch. F-62 and F-63 own that.
- A search index over many sessions.
- A human-set session title, written or read cheaply. `list` gives no title.
- Adding `simd-json`. It lost on small records. See `ADR-005`.
- A two-way pi sync. The import is one-way.
- Encryption of the file at rest. The file holds no credential, so this is a later
  hardening, not a sprint-2 need.
