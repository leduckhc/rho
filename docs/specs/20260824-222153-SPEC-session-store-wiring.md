# SPEC-session-store-wiring — the session store, reached from the command line

Status: delivered by the wiring lane.
Owning crate: `rho-core`, module `session`. Callers in `rho-cli` and `rho-tui`.

> **Read section 15 first.** The wiring lane reviewed this contract again before it wrote
> any code, because sections 3, 3a, 3b, 3c and 4 had already shipped. Section 15 lists every
> correction, and it names the tests this lane deferred and who owns each one.
Features: F-session-store, F-session-resume, F-session-list, F-session-delete,
F-session-fork, F-session-branching, F-ephemeral-mode, F-session-title,
F-session-crash-continue, F-append-only-session-log, F-slash-commands.

Supersedes nothing. It extends `SPEC-sessions`, which built the library.

## 1. What this spec is for

`SPEC-sessions` built a session store that no production caller reaches. See
`D-no-caller-writes-a-session-file`. The library can create, append, read, list, delete,
fork, and branch. The `rho` command does none of it.

This spec wires it. It adds the store location, the session id, the picker row, the
command surface, and four defect fixes.

It does not add a rewind, a replay, or a retention policy. See section 10.

## 2. The sides, and the contract kinds

A side is any two places that must agree. This change has five.

| Side | Owner | What it must agree on |
| --- | --- | --- |
| The library and the command line | `rho-core`, `rho-cli` | the store root, the id, the selector, the errors |
| The library and the terminal | `rho-core`, `rho-tui` | the row fields, the record ids, the fork target |
| Today's rho and a later rho | the persisted format | the version rule in section 6 |
| rho and the user | `rho-config`, `rho-cli` | the config keys and the flags |
| rho and a third party | `rho-core` | the store root and the project key are injected |

The contract kinds this change touches: the public API, the data model, the error
taxonomy, the persisted format, the configuration, and the behaviour rules. It touches no
wire format.

## 3. Where a session lives

See `D-session-store-layout`.

**Sections 3, 3a, 3b, 3c and 4 are built.** They live in
`crates/rho-core/src/session/key.rs`, and `crates/rho-core/tests/session_key_id.rs` holds
their tests. The prose below is the contract they were built from. One rule arrived with
the code and is stated here too: `sanitize_name` strips every leading dot, so a project
named `.config` does not become a hidden store directory. Its test is
`a_dot_named_project_does_not_hide_the_store`. `PrefixMatch` in section 4 was **not** built
with them, because it needs the store.

```rust
use std::path::{Path, PathBuf};

/// The identity of one project. Every worktree of one repository shares it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectKey(String);

impl ProjectKey {
    /// Resolve the key for a project root.
    ///
    /// It reads the `.git` entry in `root`. A `.git` file holds one line, such as
    /// `gitdir: /path/to/main/.git/worktrees/name`. The function parses that line and
    /// walks up to the repository. So every worktree of one repository returns one key.
    ///
    /// It spawns no process, and it never fails. An unreadable `.git` falls back to the
    /// physical path of `root`.
    ///
    /// **A `.git` file is untrusted input.** A checked-out repository can ship any bytes
    /// in it. So the read is bounded, and the key is sanitized. See section 3a.
    pub fn resolve(root: &Path) -> Self;

    /// The directory name for this key, `<directory-name>-<8 hex characters>`.
    ///
    /// It is always exactly one path segment. See section 3a.
    pub fn as_str(&self) -> &str;
}
```

### 3a. A hostile repository must not choose the path

A security review found this, and it is the `confine` family. That path boundary sat at
`todo!()` through a stage that reported green.

A repository can ship a `.git` file holding `gitdir: ../../../../etc`. The last component
of the resolved path becomes the directory name. So a separator or a `..` inside it would
make `root.join(key.as_str())` escape the store.

The rules, each testable:

- `resolve` reads at most one line, and at most `GIT_ENTRY_MAX_BYTES` of it.
- The directory-name part of the key is exactly one path segment. It holds no `/`, no
  `\`, and no `..`. Every byte outside `[A-Za-z0-9._-]` is replaced.
- An empty name after the replacement becomes a fixed placeholder, never an empty segment.
- The 8 hex characters come from a digest of the full identity path, so two sanitized names
  that collide still get two directories.
- **The digest is stable for ever.** It is FNV-1a over the path bytes, and section 3b states
  it exactly. A digest that changes would rename a directory a user already has.
- `store_root.join(key.as_str())` can never leave `store_root`. That is the invariant, and
  the test drives a malicious `gitdir:` line at it.

```rust
/// The most of a `.git` entry that `ProjectKey::resolve` reads.
///
/// A one-line file of ten megabytes is a denial of service, not a git directory.
pub const GIT_ENTRY_MAX_BYTES: usize = 4096;

/// The default store root, `<home>/.rho/sessions`.
///
/// The caller passes `home`, so a test never reads the real home directory.
pub fn default_store_root(home: &Path) -> PathBuf;

impl ProjectKey {
    /// Resolve a key from an already-open `.git` entry.
    ///
    /// **The bound needs this seam, or its test is theatre.** `resolve` opens the file
    /// itself, so a test on the returned key cannot see how much was read. An
    /// implementation that reads ten megabytes and then keeps 4096 bytes returns the right
    /// key. So the bound takes its input through a `BufRead`, exactly as
    /// `SessionReader::read_from` does.
    ///
    /// A test passes a source that counts the bytes it hands out. That source returns a
    /// small chunk per `fill_buf`, as a real `BufReader` does, so no implementation can
    /// borrow the whole file uncounted. The tester found that bypass in its own first draft.
    ///
    /// `root` gives the name and the fallback.
    pub fn resolve_from<R: std::io::BufRead>(source: R, root: &Path) -> Self;
}
```

### 3b. The digest is stable, and the algorithm is named

A review found the first implementation used `std::hash::DefaultHasher`. Rust documents that
hasher as unspecified, and says its hashes must not be relied on across releases.

The digest names a **persistent directory**. So a toolchain upgrade would change the key, and
every session a user already has would sit under a name rho no longer computes. `--continue`
would report an empty project, and nothing would look broken.

So the algorithm is part of the contract, not an implementation choice.

```rust
/// The digest of an identity path, as eight lowercase hex digits.
///
/// FNV-1a, 64 bit, over the path bytes. The low 32 bits are printed.
///
/// - offset basis `0xcbf2_9ce4_8422_2325`
/// - prime `0x0000_0100_0000_01b3`
/// - the input is `identity.as_os_str().as_encoded_bytes()`, never `Path::hash`
///
/// `Path::hash` is also wrong here, because it normalizes and so can differ per platform.
/// The bytes are hashed directly instead.
///
/// The algorithm is fixed for ever. A change renames a directory a user already has, so a
/// change is a migration, never a refactor.
fn digest_hex(identity: &Path) -> String;
```

Two values are stated here, so a test can pin them and a reader can check them by hand:

| identity path | digest |
| --- | --- |
| `/tmp/example-project` | `783befb6` |
| `/Users/le/Work/Vibe/rho` | `2ddec135` |

### 3c. A relative gitdir must resolve before it is hashed

A review found this, and it breaks the feature this section exists for.

git writes a relative `gitdir:` when `worktree.useRelativePaths` is set, and after a worktree
moves. So a `.git` file can hold `gitdir: ../../main/.git/worktrees/name`.

The first implementation walked to the `.git` component and returned its parent unchanged.
For that input it returned `../../main`, whose digest differs from the main checkout's
absolute path. **So two worktrees of one repository got two keys, which is the opposite of
what this section promises.**

- A relative gitdir resolves against the project root before anything hashes it.
- The resolved path is then normalized, so `a/b/../c` and `a/c` give one digest.
- A test drives a relative gitdir, and it must fail against the walk that returns it as is.

## 4. The session id

See `D-a-session-id-sorts-by-time`.

```rust
/// A session id, `<YYYYMMDD-HHMMSS>-<4 hex characters>`.
///
/// A sort of the file names gives newest first, so an ordered list reads no file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionId(String);

impl SessionId {
    /// Mint an id from a time and four random hex characters.
    ///
    /// Both inputs are parameters, so a test mints a known id and never sleeps.
    pub fn mint(epoch_millis: u64, suffix: u16) -> Self;

    /// Parse a whole id. A malformed id is a `SessionError::Decode`.
    pub fn parse(text: &str) -> Result<Self, SessionError>;

    pub fn as_str(&self) -> &str;
}

/// What a prefix resolved to.
///
/// `Many` carries every match, so the error can list them. A prefix never picks one.
#[derive(Clone, Debug, PartialEq)]
pub enum PrefixMatch {
    One(SessionId),
    None,
    Many(Vec<SessionId>),
}
```

## 5. A picker row is built from a head read and a tail read

See `D-no-list-cache-until-a-budget-fails`. No cache ships in this lane.

**A full decode is too slow, and the measurement says so.** `ADR-jsonl-codec` measured a
typed decode of one 1848-record session at 2.01 milliseconds. Five hundred of those cost
about one second. The budget in section 8 is 100 milliseconds. So a row must never decode
a whole file.

A row comes from two bounded reads.

- **The head.** The first `ROW_HEAD_LINES` lines. It gives the header, the model, and the
  first user message.
- **The tail.** The last `ROW_TAIL_BYTES` bytes. It gives the newest name, the last
  cumulative usage, and whether the file closed.

A tail read can start inside a line. The first partial line is dropped, always.

```rust
use crate::Usage;

/// The lines read from the head of a file to build one row.
pub const ROW_HEAD_LINES: usize = 8;

/// The bytes read from the tail of a file to build one row.
pub const ROW_TAIL_BYTES: u64 = 64 * 1024;

/// One row of a session list.
///
/// A file rho cannot read is a row, never a failed list. See `D-a-bad-session-file-is-one-row`.
#[derive(Clone, Debug)]
pub enum SessionRow {
    Session(Box<SessionSummary>),
    Unreadable { path: PathBuf, reason: String },
}

/// The summary of one session, for a list or a picker.
///
/// Every field here has a renderer or a test that reads it. A field that no reader wants
/// is dead surface, and this project has a defect class for that. See
/// `D-dead-surface-is-a-defect-class`.
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

/// The origin of a forked session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForkOrigin {
    pub session_id: String,
    pub record_id: RecordId,
}
```

**Three fields the review cut.** A first draft carried `provider`, `approval`, and
`sandbox` on the row. No renderer reads any of them, and no test asserted them. A field on
a contract is a promise every side keeps, so all three are gone. The header still records
all three, and a resume still reads them from the header.

### 5a. The row builder must not be able to open a file

The review found this, and it is the worst finding in the set. A first draft passed both a
counting `source` **and** a `path` to the row builder. An implementation could ignore the
source, open the path, and read the whole file. The counting source would then report almost
nothing, and the byte assertion would pass.

So the seam existed and proved nothing. That is the memory-cap defect of `D-bash-line-cap`,
rebuilt by the very fix meant to prevent it.

**The row builder takes no openable path.** It takes the source and the metadata a source
cannot know.

```rust
use std::io::{BufRead, Seek};

/// The facts about a file that a byte source cannot carry.
///
/// `display_path` is data. The row builder never opens it, and a test proves that by passing
/// a path that does not exist.
#[derive(Clone, Debug)]
pub struct RowMeta {
    pub display_path: PathBuf,
    pub size_bytes: u64,
    pub last_active_millis: u64,
}

/// Build one row from one open source.
///
/// This is the seam the budget test drives. A test passes a source that counts the bytes it
/// hands out, and asserts the count stays within `ROW_HEAD_LINES` lines plus
/// `ROW_TAIL_BYTES` bytes, plus one buffered chunk.
///
/// The counting source sits **under** any buffered reader, so a full forward scan is counted
/// rather than hidden by a buffer.
pub fn row_from<R: BufRead + Seek>(source: R, meta: RowMeta) -> SessionRow;
```

`SessionStore::rows` opens each file, reads its metadata, and calls `row_from`. So the store
path and the tested path are the same code.

The test passes a `display_path` that does not exist, and content that lives only in the
source. An implementation that opens the path fails, because there is nothing to open.

**A row carries no turn count.** A count needs every record, so it needs a full decode.
The row reports tokens and cost instead, and both are exact, because a `Usage` record is
cumulative. A turn count arrives with the cache, or not at all. **rho shows no number it
did not read.**

`SessionStore::list` is replaced by `SessionStore::rows`. The old method returned four
fields that no picker wants, and two methods for one job would leave dead surface.

**The existing list tests are ported, not preserved.** The review corrected a false claim
here. `list_reads_only_the_first_line` asserts `summary.session_id`, `.cwd`, `.size_bytes`,
and `.path`. The return type becomes an enum, and `session_id: String` becomes
`id: SessionId`. So those assertions are rewritten, and the ported test is named
`rows_read_only_the_head_and_the_tail`. Under `AGENTS.md` that makes this change not a
pure refactor, and the spec says so rather than claiming otherwise.

## 6. The version rule

See `D-chain-records-are-frozen`.

A **chain record** may be a parent. The frozen set is `Session`, `ModelChange`, `Message`,
`Usage`, `Stop`, `Closed`, and `Reopened`. Every version must decode all seven. A new
chain record needs a version bump.

A **leaf record** is never a parent. `Name` is the first one. An unknown leaf is skipped,
counted, and warned about, as `D-a-bad-middle-record-is-skipped-and-counted` states.

### 6a. The reader cannot see the class, so the check is referential

The review found the hole in the first draft. `Record` is a `#[serde(tag = "type")]` enum.
An unknown tag fails to decode, so the reader **cannot tell an unknown leaf from an unknown
chain record**. Both land in the same dropped count. So a rule written as "an unknown leaf
is safe" would be exactly `ToolKind::Other`: an unknown thing assumed harmless.

A second idea also fails. The reader cannot record the id of a line it could not decode,
because the id is inside the line it could not decode.

**So the check runs from the child side, and it is referential integrity.**

- Every non-root `parent_id` must resolve to a record the reader decoded.
- A `parent_id` that resolves to nothing is `SessionError::Orphan`, and it names both ids.
- The check runs at read time, before any branch walk.

That detects a skipped chain record, because its children point at nothing. It ignores a
skipped leaf, because nothing points at a leaf. So the class is **derived from the data**,
and never declared by a writer. A future rho needs no cooperation from this build.

The same read also refuses two records that share an id. See section 7a.

**And it refuses a cycle.** Referential integrity alone cannot see one: a cycle resolves every
parent, and no record on it is a leaf. So a three-line file whose `a` names `b` and whose `b` names
`a` passed every check, and then every walk ran for ever and cloned an entry per turn of the loop.
That is a denial of service reachable from a resume and from `sessions fork`.

A reviewer found it, and this project already guards the same class for subagents in
`check_no_cycle`, "to stop an infinite loop inside a lock". The session reader had forgotten it.

- `SessionError::CyclicChain` names a record on the cycle.
- The check runs at read time, with the other two, and it is linear because it memoises the
  records it has already settled.
- `walk_chain` keeps a visited set as well, so a caller with hand-built entries cannot spin.
  Defence in depth, because one guard on one path is how `confine` stayed unproven.
- `rho sessions show` walks parents too, in `depths`. It memoises, so a long session costs O(N)
  and not O(N squared).

### 6b. A silent early stop must become an error

`branch_messages` and `fork` both walk parent links with `None => break`. See lines 1084
and 1030. So a hole in the chain ends the walk in silence, and a resume loses the end of a
conversation.

The refusal in section 6a fixes the store path, because `read_from` runs first. It does not
fix a caller that builds entries by hand. So both walkers change too:

- A walk that meets a missing parent returns an error. It never returns a short list.
- A test drives hand-built entries with a hole, and asserts the error.

Defence in depth, because one guard on one path is how `confine` stayed unproven.

```rust
/// Rebuild the messages along one branch.
///
/// A missing parent is an error, never a short list. A short list would drop the end of a
/// conversation, and the provider request would look valid.
///
/// `root` names the header record id, which is not in `entries`, because `read_from`
/// consumes the first line before its loop. A caller that read a file passes
/// `Some(&read.header_id)`. A caller with hand-built entries passes `None`, and then every
/// parent must resolve inside `entries`. Without this parameter the walk cannot tell the
/// root from a hole, so it would refuse every real file. See section 15, finding 10.
pub fn branch_messages(
    entries: &[Entry],
    head: &RecordId,
    root: Option<&RecordId>,
) -> Result<Vec<Message>, SessionError>;
```

`ReadResult` gains `header_id: RecordId` for the same reason. A fork needs it too, to
re-parent its first copied record.

### 6c. The records

```rust
/// One new leaf record. It is never a parent.
///
/// The newest `Name` record wins. See `D-a-session-title-costs-nothing`.
Name { title: String },
```

The header record gains two optional fields. No type in this module sets
`deny_unknown_fields`, so an older reader ignores a field it does not know.

```rust
Session {
    version: u32,
    cwd: PathBuf,
    approval: String,
    sandbox: String,
    /// The session id. It was implicit in the file name before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    /// Set when this file came from a fork.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    forked_from: Option<ForkOrigin>,
},
```

### 6d. The recorder writes the assistant turn

See `D-a-recorder-writes-the-assistant-turn`. The wiring lane found this before it wrote
any code, and the first draft of this spec did not name it.

`SessionRecorder` is the only thing that turns a live run into records. It writes a tool
result, a usage record, and a stop record. It writes **no assistant message and no tool
call**, because its match has no `TurnEnd` arm and it reads no `StreamEvent` text or
tool-call event. Its own doc comment claims otherwise.

So a recorded session holds the prompts and the results, and nothing else. A resume then
builds a message list with a `ToolResult` that matches no `ToolCall`, and every provider
refuses that request. This defect makes the whole feature unusable, so it is fixed here.

```rust
impl SessionRecorder {
    /// Fold one agent event.
    ///
    /// The recorder holds the parts of the current assistant turn. `TextDelta` appends
    /// text. `ThinkingEnd` closes a reasoning block. `ToolCallEnd` completes a call with
    /// its parsed arguments. `TurnEnd` writes one `Message` record with role `Assistant`,
    /// in provider block order. An empty turn writes nothing.
    ///
    /// So the file order is always the call, then its result. A `ToolCall` on disk with no
    /// `ToolResult` is then impossible on the run path.
    pub fn observe(&mut self, event: &AgentEvent) -> Option<RecordId>;

    /// Write an explicit title as a `Name` leaf record.
    ///
    /// An empty or blank title is refused, so a row never shows a blank name. See
    /// `D-a-session-title-costs-nothing`.
    pub fn record_name(&mut self, title: &str) -> Result<Option<RecordId>, SessionError>;
}
```

**A tool result was not wrapped either.** `ToolEnd` wrote the raw output blocks as the
content of the tool message, so the record carried no `tool_call_id`. `Agent::finish_tool`
wraps the same output in a `ContentBlock::ToolResult`, so the recorded conversation had a
different shape from the one the model saw. A resume then sent a tool message no provider can
match to a call, and `branch_messages` invented a synthetic error result beside the real one.
The recorder now writes the same shape the live context holds.

- A reasoning payload is kept verbatim. A rewritten payload cannot replay.
- Redaction still runs through `redact_block` before anything reaches the file.
- A cancel writes the partial assistant message with the real arguments it holds, instead
  of the empty object it invents today.

## 7. The store operations

`create` takes one struct, not six arguments. A four-argument constructor already hid a
fake model id and an approve-all policy in this project. See
`D-no-four-argument-session-new`.

```rust
/// What a new session needs, when the caller has no id yet.
///
/// A review found `create_minted` naming a type the spec never defined. It is stated here,
/// because a retry mints a second id and so the id cannot be a field of the request. See
/// section 15, finding 2.
#[derive(Clone, Debug)]
pub struct NewSessionWithoutId<'a> {
    pub cwd: &'a Path,
    pub approval: &'a str,
    pub sandbox: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    pub forked_from: Option<ForkOrigin>,
}

/// What a new session needs. One struct, so a later field breaks no caller.
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

impl SessionStore {
    /// Create a session file. Write the header, then one `ModelChange` record.
    ///
    /// The model record is written here, not by a caller. So every file states its model
    /// on the second line, and a row reads it from the head. Nothing can forget it.
    ///
    /// **The file is created exclusively.** An existing path is an error, never a truncation.
    /// See section 7c.
    ///
    /// The file is created with mode `0o600`, and every directory rho creates under the
    /// store root with mode `0o700`. See section 9.
    pub fn create(&self, new: NewSession<'_>) -> Result<SessionWriter, SessionError>;

    /// Create a session, and mint a fresh id when the first one is taken.
    ///
    /// This is what a run calls. It retries `MINT_ATTEMPTS` times, so a collision costs one
    /// more mint rather than a lost session.
    ///
    /// It returns the id it used beside the writer. A caller must print the id, and it
    /// cannot recompute one that a retry replaced.
    pub fn create_minted(
        &self,
        now_millis: u64,
        new: NewSessionWithoutId<'_>,
    ) -> Result<(SessionId, SessionWriter), SessionError>;

    /// Create a session, taking each id suffix from `suffixes`.
    ///
    /// **The retry needs this seam, or its test is theatre.** `create_minted` draws its own
    /// suffix, so two calls in one millisecond get two ids and no collision ever happens.
    /// A test could then never reach the retry. So the suffix source is a parameter here,
    /// exactly as `SessionReader::read_from` and `ProjectKey::resolve_from` take their
    /// input.
    ///
    /// A test passes `[0x1234, 0x1234, 0x5678]`, so the first two attempts collide and the
    /// third wins. A test that passes one repeated suffix reaches `MINT_ATTEMPTS`.
    ///
    /// `create_minted` calls this with a real suffix source, so the store path and the
    /// tested path are the same code. See section 15, finding 8.
    pub fn create_minted_from<I: Iterator<Item = u16>>(
        &self,
        now_millis: u64,
        suffixes: I,
        new: NewSessionWithoutId<'_>,
    ) -> Result<(SessionId, SessionWriter), SessionError>;

    /// Every session in the store, newest first. A file rho cannot read is one row.
    ///
    /// It reads `ROW_HEAD_LINES` lines and `ROW_TAIL_BYTES` bytes per file. It never
    /// decodes a whole file.
    pub fn rows(&self) -> Result<Vec<SessionRow>, SessionError>;

    /// Resolve a prefix to one id, to none, or to every match.
    pub fn resolve_prefix(&self, prefix: &str) -> Result<PrefixMatch, SessionError>;

    /// The newest session in this store that holds no `Closed` record.
    ///
    /// This is what a crash offers, and it is what `--continue` takes.
    pub fn newest_open(&self) -> Result<Option<SessionId>, SessionError>;
}
```

### 7a. A record id is minted against the set

See `D-a-record-id-is-minted-against-the-set`. This fixes a defect that mints a duplicate
id in two places.

**There is no public method for it.** A first draft added `seed_ids`, and then section 11
forbade any test from calling it. A public method that the spec forbids is a hazard, because
a caller can seed the wrong ids and force a collision. So the writer seeds itself from the
file it opens, inside `append_to` and inside `fork`. No caller can forget it, and no caller
can misuse it.

`append_to` used `entries.len() + 2`, and the reader drops records, so two drops made a
duplicate. `fork` added one per copied record, and a branch is not contiguous, so the
chain `r1`, `r2`, `r4` made a duplicate `r4`.

### 7b. A fork re-parents its first record

A review claimed that `fork` copies the source header, so a forked file holds two `Session`
records. **That is wrong, and I checked it.** `read_from` consumes the first line before its
loop, so `read.entries` never holds the header. The parent walk stops at the header and never
copies it.

**The real defect is a coincidence.** The first copied record keeps `parent_id = Some(r0)`.
That resolves only because every header is minted first, and so is always `r0`. An imported
pi file has an eight character hex header id. Then the first copied record points at an id
the new file does not hold, and the orphan check in section 6a refuses the fork.

So `fork` re-parents its first copied record onto the new header id. The invariant is stated
as a test, and it does not depend on any id being `r0`.

### 7c. An exclusive create, because a collision truncates

`SessionStore::create` calls `File::create` today, and its own comment says "Create or
truncate". See `crates/rho-core/src/session/mod.rs` line 924.

The id is a one second stamp plus four hex characters, which is 16 bits and 65536 values. For
N sessions minting in one second the collision chance is about `N * (N - 1) / 2 / 65536`. At
50 concurrent sessions that is 1225 in 65536, which is 1.87 percent.

So about one run in 53 would erase another session's file, in silence. An operations review
found it and did the arithmetic.

```rust
/// How many times a create re-mints an id before it gives up.
pub const MINT_ATTEMPTS: usize = 8;
```

- `create` opens the file with `create_new`, so an existing path is an error.
- `create_minted` catches that error and mints again, up to `MINT_ATTEMPTS`.
- A run therefore never truncates a session that already exists.
- The retry is bounded, so a full store cannot spin.

### 7d. A live session holds an advisory lock

Two worktrees share one project key, so `--continue` in both can open one file. Both would
seed their ids from the same read, both would mint the same ids, and the lines would
interleave. An architecture review found it.

**The prior art was read before this rule was written.** Only fx faced the same race, and it
solved it with a lock rather than by partitioning the key space.

| Tool | Key | Session lock | Two writers possible |
| --- | --- | --- | --- |
| pi | the absolute working directory | none | yes, unguarded |
| jcode | the session id only | none, but one daemon owns writes | no, by architecture |
| fx | the session id | yes, a per-session advisory lock | no |

pi shows the cost of no lock. Two pi processes in one directory append to one file, and
nothing stops them. So a per-directory key does not remove the race. It only makes it rarer.

fx holds `session.lock` per session, refuses a second writer with a busy error, and reports
it as "open elsewhere". When the filesystem cannot lock, fx refuses instead of continuing.
rho copies that shape.

```rust
/// A held advisory lock on one session.
///
/// It is released on drop, and by the operating system when the process dies. So a crash
/// never leaves a session locked forever, which a plain lock file would.
pub struct SessionLock { /* private */ }

impl SessionStore {
    /// Take the advisory lock for one session.
    ///
    /// `SessionError::Busy` names the session when another process holds it.
    /// `SessionError::LockUnsupported` names the path when the filesystem cannot lock.
    pub fn lock(&self, id: &SessionId) -> Result<SessionLock, SessionError>;
}
```

The rules:

- Every write path takes the lock: a create, a resume, and a fork of the target it writes.
- `newest_open` skips a session another process holds. So `--continue` never picks a live
  session, and it moves to the next one instead.
- A read-only path takes no lock. So `sessions list` and `sessions show` always work.
- **A filesystem that cannot lock is a refusal, not a warning.** A network filesystem that
  silently ignores a lock would fail open, and that is the shape of
  `D-plugin-does-not-classify-itself`.
- `--ephemeral` needs no lock, because it writes no file.

### 7e. The error taxonomy

**A record id must print itself.** `rho-core` has no `Display` impl for `RecordId`, so an
error that names a record id does not compile. A cold compile of this contract in a scratch
crate outside the repository found it. So this spec adds the impl.

```rust
/// A record id prints as its own text, so an error message can name one.
impl std::fmt::Display for RecordId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
```

```rust
/// The new variants. Every earlier variant keeps its meaning.
pub enum SessionError {
    // Io, Encode, Decode, Version, Widen stay exactly as they are.
    /// A prefix matched more than one session. The message lists every match.
    #[error("the id prefix {prefix} matches {} sessions: {}", matches.len(), matches.join(", "))]
    AmbiguousPrefix { prefix: String, matches: Vec<String> },
    /// A prefix matched no session in this project.
    #[error("no session in this project starts with {prefix}")]
    NoSuchSession { prefix: String },
    /// `--continue` found no session to continue.
    #[error("no session to continue in {project}; start one without --continue")]
    NoSessionToContinue { project: String },
    /// A record names a parent the reader skipped, so the chain has a hole.
    #[error("record {child} names parent {parent}, which this build could not read")]
    Orphan { child: RecordId, parent: RecordId },
    /// A record names a leaf record as its parent. A leaf is never a parent.
    ///
    /// A reviewer asked for the name. Section 11 names the test and the first draft had no
    /// error for it, so the refusal would have arrived as a bare decode message. See section
    /// 15, finding 9a.
    #[error("record {child} names parent {parent}, which is a leaf record and never a parent")]
    LeafParent { child: RecordId, parent: RecordId },
    /// Two records in one file share an id.
    #[error("record id {id} appears twice in the file")]
    DuplicateId { id: RecordId },
    /// Another process holds this session.
    #[error("session {id} is open in another process. Use another session, or close that one.")]
    Busy { id: String },
    /// The filesystem cannot hold an advisory lock.
    ///
    /// The message calls `path.display()`, because `PathBuf` does not implement `Display`.
    /// A cold compile of this contract caught that. It is the second error the compile found.
    #[error("the filesystem at {} cannot lock a session. Set a store on a local disk.", path.display())]
    LockUnsupported { path: PathBuf },
}
```

## 8. The command surface, and the budget

See `D-resume-is-a-flag-on-run` and `D-continue-is-scoped-to-the-project`.

```rust
/// Which session a run uses. The command line resolves exactly one of these.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionSelector {
    /// A fresh session. The default.
    New,
    /// The newest session for this project key.
    Newest,
    /// One session, named by an id or a prefix of one.
    Named(String),
}
```

**One flag, two spellings, an optional value.** `--resume` is an alias of `--continue`.
They are the same argument, so they can never disagree.

| Command | Selector |
| --- | --- |
| `rho run "..."` | `New` |
| `rho run "..." --continue` | `Newest` |
| `rho run "..." -c` | `Newest` |
| `rho run "..." --resume` | `Newest` |
| `rho run "..." --continue=<id>` | `Named` |
| `rho run "..." --resume=<id>` | `Named` |
| `rho run "..." -c=<id>` | `Named` |

Other flags: `--allow-widen` and `--ephemeral`. `--allow-widen` alone is an error.
Subcommands: `rho sessions list`, `show`, `delete`, `fork --at <record-id>`, and `name`.
Terminal commands: `/sessions` opens the picker.

**`/tree` and `/fork` are not in this lane.** An earlier draft named both in this list. They
were specified nowhere and tested nowhere, which is `D-dead-surface-is-a-defect-class`. A
review named it as the `/guide` defect repeated, where the first frame advertised a command
that answered an error. So the terminal keeps one new command, and the record navigator gets
its own lane and its own spec. See `D-the-record-id-is-visible-on-the-command-line`.

### 8a. The discovery path for a record id

A fork needs a record id. So a user must be able to see one. This section is the whole
reason the fork command is usable.

`rho sessions show <id-prefix>` prints one line per record. The format is fixed:

```
session  20260825-094512-a3f9  "fix the parser"  claude-sonnet-4  2 turns  closed
  r1  09:45:12  user       fix the parser
  r2  09:45:14  assistant  I will read the file first.
  r3  09:45:14  tool_call  read  path=src/parse.rs
  r4  09:45:15  tool_result  read  1.2 KiB
  r5  09:45:19  assistant  The bug is on line 42. Shall I fix it?
  r6  09:46:02  user       yes
  r7  09:46:20  assistant  Done. I changed one line.
```

Rules for the format:

- The record id is the first column, so a user copies it into `--at`.
- One record is one line. A long text is cut at the terminal width, and never wrapped.
- The whole listing fits 80 columns. A test asserts that.
- **The close state is the last thing cut.** It is one word. It is also the only header field a
  user cannot read again from a record line. The title gives way first, and then the model, which
  keeps at most half of the room that is left. A live drive found a long title pushing both the model and
  the state off the line, and a reviewer found a real 41 character Bedrock model id doing the same
  to the state alone.
- A tool call names the tool and its short arguments. **A tool message names the tool and a byte
  count, never the content.** The rule is the **role**, and not the block shape: a first version
  keyed on a `ToolResult` block, so a tool message holding a bare `Text` block fell through and
  printed the body. A crafted or an imported file holds exactly that. `--full` does not turn the
  rule off, or the flag would be a way to read every secret a session recorded.
- `--full` prints the whole text of each record instead of one line.
- A branch is shown by indentation, and a sibling branch is marked. So a user can see that
  two answers came from one question.

Then the fork is two commands a user can actually type:

```sh
rho sessions show 20260825-09
rho sessions fork 20260825-09 --at r5
```

So the owner's ask is reachable in this lane, on the command line. The terminal navigator
is nicer, and it is not the only way.

### 8b. What `rho sessions list` prints

The list is a user-facing surface, so its columns are part of the contract.

```
ID                    LAST ACTIVE  TITLE                          MODEL            TOKENS   COST
20260825-094512-a3f9  2 min ago    fix the parser                 claude-sonnet-4   14.2k  $0.08
20260824-171003-77b2  yesterday    add the retry test             claude-haiku-4     3.1k  $0.01
20260823-092211-0c41  2 days ago   * unreadable: bad header       -                    -      -
```

- Six columns, and they fit 80 columns. A test asserts the width.
- The title column is cut with an ellipsis, never wrapped.
- An unreadable row keeps its id and its reason, and shows a dash for every unknown field.
- `--long` adds the working directory and the fork origin.
- A row shows no turn count, because a turn count needs a whole file. See section 5.
- The order is newest first.

The row carries `cwd` for `--long`. So the wider project scope stays visible. See
`D-continue-is-scoped-to-the-project`.

### 8c. The value needs an equals sign, and the reason is measured

The prompt is a positional argument. See `crates/rho-cli/src/cli.rs` line 211. An optional
flag value beside a positional argument is a known trap. So I drove clap 4 to find out what
it really does.

Without `require_equals`, the natural command fails:

```
rho run --continue "fix the bug"
error: the following required arguments were not provided:
```

clap takes the prompt as the flag's value, so the prompt goes missing. With
`require_equals = true` that command works, and so does `--resume=<id>`.

**One case stays silently wrong, so it needs a guard.** This was measured, not guessed:

```
rho run --resume 20260825-09
  selector = Newest        (the flag was bare)
  prompt   = "20260825-09" (the id became the prompt)
```

A space instead of an equals sign continues the wrong session. It also sends the id to the
model as a question. No error appears. That is a fail-open shape, so the contract refuses it.

- The flag sets `require_equals = true`, **and** `num_args = 0..=1` with a
  `default_missing_value`. A review found the first draft named the missing value and not
  the argument count, and without the count a bare `--continue` yields no value at all. See
  section 15, finding 5.
- A prompt that matches the session id shape is refused. The message names `--resume=<id>`.
- The refusal is the whole rule. rho never guesses which one the user meant.

**The alias makes one refusal free.** `--continue --resume=<id>` is already an error, because
clap sees one argument used twice. The first draft needed a custom conflict rule for that.
The alias deletes the rule and its code.

### 8d. What the picker costs

A bare `--resume` now means the newest session. So it no longer opens a picker.

The picker lives in two places instead:

- `/sessions` inside the terminal.
- `rho sessions list` on the command line, whose columns are fixed in section 8b.

A review asked what a user with 40 sessions does, since they remember no ids. The answer is
two commands. `rho sessions list` shows the titles and the times. Then `--resume=<prefix>`
takes the one they want, and four characters of the stamp are usually enough.

A command-line picker would be a third surface, and nobody has asked for one. If it
arrives, it arrives as its own flag. It never arrives as a second meaning for this one.

### 8e. A user can look without prompting

`run` needs a prompt, so a resume needs one too. A review found that a user cannot simply
look at yesterday's session, because they must invent a question first.

`rho sessions show <id-prefix>` is that read-only view. It sends nothing to a model, and it
starts no session. So looking costs nothing.

**The budget.** A list of 500 sessions completes under 100 milliseconds. The spec records
the command and the measured number in `docs/benchmarks.md`. If the budget fails, the
cache in `D-no-list-cache-until-a-budget-fails` ships, and not before.

## 9. Security rules

A security review produced every rule here. Before this change rho wrote no session file,
so all of this surface is new.

### The store is private, by construction

A default umask would make the directory `0o755` and the file `0o644`. Then any local user,
or a synced backup folder, reads every conversation.

rho already solved this for a subagent transcript. `crates/rho-core/src/transcript.rs` sets
a file to `0o600` at line 135, and walks the ancestors to `0o700` at line 126. Its test is
`permissions_are_0o600_on_unix`, at line 226.

- A session file is `0o600` on unix.
- Every directory rho creates under the store root is `0o700` on unix.
- A sidecar spill file gets the same mode as the session file.
- A test asserts each mode, and it mirrors the transcript test.

### What redaction does not cover, stated plainly

`redact_block` masks `ToolCall.arguments` through `rho_redact::redact_json_secrets`. That
function matches a **key name**. Every other content block passes through unchanged.

So the file holds these verbatim, and this spec does not change that:

- A secret pasted into a prompt.
- A secret inside a tool **result**, such as the output of `cat .env` or `env`.
- A secret on a `bash` command line, because the key is `command` and not a secret name.
- A token inside a URL, such as a git remote.

Two consequences for this contract:

- The test `no_credential_reaches_the_file_on_the_run_path` is renamed
  `a_secret_named_tool_argument_is_masked_on_the_run_path`. The old name claimed a
  guarantee that the code does not give. **A test name is a claim.**
- A value-shaped scan is named as required follow-up work. It belongs to the lane that
  records tool results by default, and it is out of scope here.

`docs/guide/sessions.md` states the gap in the same words. A user who records a secret must
delete that session. rho applies no encryption at rest, and no prune.

### A session file is untrusted input

A resume reads a file. An attacker who can write into the store chooses what rho replays.

- The run's working directory and its modes come from live config, never from the file.
- The stored `approval` and `sandbox` may only **tighten** the run. They can never widen it,
  and they never stand in for `--allow-widen`. A forged header can therefore remove a
  warning, and it can never grant a permission the live config withheld.
- `forked_from` is shown and never trusted. It opens no file and grants nothing.
- Every record's content is untrusted text that reaches the model. The rebuilt context is
  model-visible input, exactly like a tool result.
- The mode names still parse to the strictest value when unknown. That behaviour exists,
  and a test pins it.

### Delete says what it does

`SessionStore::delete` removes the session file, and it scans for `<id>.*.sidecar` and
removes each one. I verified both against the implementation.

It does not overwrite the bytes, so a recovery tool may still find them. It does not remove
a session forked from this one, because a fork is its own file. The guide says both.

### The id suffix is not an access control

Four hex characters defend against two sessions in the same second. They are not a secret.
Access is enforced by `0o700` on the store, and by nothing else.

## 10. Out of scope

- The rewind, and any file snapshot. That is the next lane.
- The replay of a session, and any cross-model diff.
- Retention, pruning, and any bound on the store. See
  `D-retention-is-not-in-the-wiring-lane`. The store grows, and the guide says so.
- A turn count in a row. It needs a full decode, so it waits for the cache.
- Compaction and a branch summary. `F-context-compaction` and `F-branch-summary` own them.
- Search across sessions, export to HTML, and grafting a turn between sessions.
- A session file per subagent. A subagent keeps its transcript.
- An ACP method. `F-session-commands-over-acp` stays planned, and this contract does not
  block it.
- A second storage backend. `SessionStore` stays a struct, not a trait. A caller injects
  the store root and the project key, and that is the extension point. A sqlite backend is
  a fork, and the reason is written here rather than left to a guess.

## 11. Test cases

Every test uses `tempfile`. No test sleeps. No test reads the real home directory. No test
uses the network.

**The project key.**
- `every_worktree_of_one_repository_shares_a_key` — a `.git` file that names a worktree
  gitdir resolves to the same key as the main checkout.
- `a_plain_directory_keys_on_its_own_path` — a root with no `.git` keys on the physical path.
- `an_unreadable_git_entry_falls_back_and_never_fails` — a `.git` file of junk still
  returns a key.
- `two_directories_of_one_name_get_two_keys` — the digest keeps them apart.
- `a_hostile_gitdir_cannot_escape_the_store` — a `.git` file holding
  `gitdir: ../../../../etc` yields a key whose joined path stays under the store root. The
  invariant, over a table of hostile lines, not one example.
- `a_key_is_always_one_path_segment` — for any `.git` content, the key holds no separator
  and no `..`.
- `a_giant_git_entry_stops_at_the_cap` — a one-line `.git` of ten megabytes reads at most
  `GIT_ENTRY_MAX_BYTES`. **The test drives `ProjectKey::resolve_from` with a counting
  source**, so it
  fails against an implementation that reads the file and then truncates the string.
- `a_key_that_sanitizes_to_nothing_gets_a_placeholder` — a name of only illegal bytes never
  yields an empty segment.
- `the_default_store_root_sits_under_the_passed_home` — `default_store_root` never reads the
  real home directory, and it returns `<home>/.rho/sessions`.

**The id.**
- `an_id_sorts_by_time` — for any two mint times, the newer id sorts after the older.
- `two_sessions_in_one_second_get_two_ids` — the same stamp with two suffixes differs.
- `a_malformed_id_is_refused` — `SessionId::parse` refuses a name that is not the shape.
**The prefix, which needs the store.** These four sit with the store slice, not with the id,
because `resolve_prefix` reads a directory. The tester found the misplacement.
- `a_unique_prefix_resolves_to_one_session` — a prefix of one id returns `One`.
- `an_ambiguous_prefix_lists_every_match` — a shared prefix returns `Many` with each id,
  and the error message names them all.
- `an_unknown_prefix_resolves_to_none` — a prefix that matches nothing returns `None`.
- `an_unknown_prefix_on_the_command_line_names_the_project` — the command turns `None` into
  `SessionError::NoSuchSession`, and the message names the prefix.

**The row.**
- `a_row_never_decodes_the_whole_file` — a counting source is driven through `row_from`, and
  the test asserts the bytes handed out stay within `ROW_HEAD_LINES` lines plus
  `ROW_TAIL_BYTES` plus one buffered chunk, on a file far larger than both. The
  `display_path` in `RowMeta` does not exist, so an implementation that opens a path fails.
  **It must fail against a full-file implementation, and against one that ignores the source.**
- `rows_read_only_the_head_and_the_tail` — the ported list test. It keeps the assertions on
  the path, the cwd, and the size, and it reads the id as a `SessionId`.
- `a_row_reports_the_cumulative_usage` — the tokens and the cost come from the last `Usage`
  record.
- `a_row_reports_no_usage_for_a_session_with_no_turn` — the field is `None`, and the row
  shows no number it did not read.
- `a_row_states_the_model_from_the_second_line` — `create` writes the `ModelChange` record,
  so the row shows the model with no full read.
- `a_row_states_its_start_time_and_its_last_activity` — the start time comes from the header,
  and the last activity comes from the file metadata.
- `a_row_states_its_size` — `size_bytes` matches the file length.
- `a_row_prefers_an_explicit_name` — with a `Name` record the title is the name, and the row
  says the title is explicit.
- `a_row_falls_back_to_the_first_prompt` — with no `Name` record the title is the first line of
  the first prompt, cut at 60 bytes, and the row says the title is not explicit.
- `a_tail_read_drops_a_partial_first_line` — a tail that starts inside a line yields no
  broken record.
- `a_row_marks_a_closed_session` — a file that ends with `Closed` reports closed.
- `one_unreadable_file_is_one_row` — for any directory of N files, `rows` returns N rows,
  and an unreadable file is an `Unreadable` row with a reason. The invariant, not one case.
- `rows_come_back_newest_first` — the order follows the id. A second assertion counts the
  files opened during the sort, and it is zero.
- `a_row_shows_its_fork_origin` — a forked file states the source session and record.

**The four defects.**

Each test below drives the **real store path**, and never seeds a writer by hand. A test
that calls `seed_ids` itself proves nothing about whether `append_to` remembered to call it,
and a caller that forgot to wire a guard is this project's signature defect.

- `two_dropped_records_do_not_mint_a_duplicate_id` — a file whose reader drops two records
  is reopened through `SessionStore::append_to`, and the new record's id is one no earlier
  record holds. It must fail against the `entries.len() + 2` seed.
- `a_fork_of_a_branch_does_not_mint_a_duplicate_id` — a fork of the chain `r1`, `r2`, `r4`
  runs through `SessionStore::fork`, and the next append is not `r4`. It must fail against
  the count-based seed.
- `every_record_id_in_a_file_is_unique` — the invariant over any sequence of appends,
  forks, and reopens, all through the store.
- `an_imported_file_mints_a_fresh_id` — after a pi import, an append through the store mints
  an id that no imported record holds.
- `one_unreadable_file_is_one_row` — named again here, because it is this defect's own test.
- `a_header_states_its_own_session_id` — a file written by `create` names its id in the
  header, and `parse_header` returns it rather than an empty string.

**The version rule.**
- `an_unknown_leaf_record_is_skipped_and_counted` — a file with a leaf record this build
  does not know loads every other record.
- `an_orphan_refuses_the_file` — a record whose `parent_id` resolves to nothing returns
  `SessionError::Orphan`, and the message names both ids. **The test drives
  `SessionReader::read_from`**, never a helper, so a checker with no caller fails it.
- `an_unknown_chain_record_is_caught_by_its_children` — a file with an unknown record that
  has children is refused, because the children orphan. This is the test that proves
  section 6a, and it must fail against a reader that only counts drops.
- `the_chain_record_set_is_frozen_and_the_version_gates_it` — the runtime half asserts that a
  file naming a higher version returns `SessionError::Version`. The compile half is an
  exhaustive match with no wildcard, so an eighth chain variant breaks the build. **The first
  draft named only the match, which fails at compile time and therefore asserts nothing at
  run time.**
- `a_hand_built_leaf_parent_is_refused` — a leaf record named as a parent is refused. The
  test builds the entries by hand, because no writer can produce that file, so a test over
  written files could never fail.
- `an_old_reader_ignores_a_new_header_field` — a header with an unknown field still parses.
- `a_duplicate_id_in_a_file_is_refused` — a hand-edited file with two `r4` records returns
  `SessionError::DuplicateId` at read time, before any branch walk.
- `a_branch_walk_with_a_hole_is_an_error` — `branch_messages` on hand-built entries with a
  missing parent returns an error, and never a short list. Same for `fork`.

**The title.**
- `a_new_session_titles_itself_from_the_first_prompt` — the title is the first line, capped
  at 60 characters.
- `the_newest_name_record_wins` — two `Name` records resolve to the later one.
- `an_empty_name_is_refused_by_the_recorder` — `sessions name` with an empty string is an
  error, and the error has its own name. A live drive showed the first version reporting
  `cannot decode a record`, which reads like file corruption.
- `a_title_costs_no_model_call` — the title path calls no provider. The recorder holds no
  provider at all, which is the structural proof.

**Recording, and the default.**
- `a_run_writes_a_session_file_by_default` — the store holds one file after a run.
- `the_ephemeral_flag_writes_no_file` — with `--ephemeral` the store stays empty.
- `the_ephemeral_config_key_writes_no_file` — the key does what the flag does.
- `the_session_file_key_overrides_the_store` — `session-file` writes the named path.
- `a_write_failure_degrades_and_the_run_finishes` — the run completes with a warning.
- `a_secret_named_tool_argument_is_masked_on_the_run_path` — a secret under a flagged
  argument key is masked in the file a real run wrote. It proves the wiring, not the
  function. **The name states its own limit**, because redaction matches a key name only.
  See section 9.

**The store is private.**
- `a_session_file_is_0o600_on_unix` — the mode of a created file, mirroring
  `permissions_are_0o600_on_unix` in `transcript.rs`.
- `a_store_directory_is_0o700_on_unix` — the mode of every directory rho creates.
- `a_sidecar_spill_file_is_0o600_on_unix` — the spill path gets the same care as the file.

**Resume is a trust boundary.**
- `a_resume_takes_its_cwd_from_config_not_from_the_file` — a forged `cwd` in a header
  changes no path the run uses.
- `a_forged_header_cannot_widen_a_run` — a header claiming `allow-all` never grants more
  than the live config grants, and it never stands in for `--allow-widen`.
- `an_unknown_mode_name_still_parses_to_the_strictest_mode` — the existing rule, pinned
  again from the command-line path.
- `a_forged_fork_origin_opens_no_file` — `forked_from` names a sentinel path, and the test
  asserts that path is never opened. An assertion that the run succeeds would pass against an
  implementation that follows the origin.

**The discovery path, and the printed surface.**
- `show_prints_one_line_per_record_with_its_id` — every record appears once, and the id is
  the first column. This is what makes `--at` usable.
- `show_fits_eighty_columns` — for any record content, no printed line exceeds 80 columns.
  The invariant, over long text and over wide unicode.
- `show_cuts_a_long_text_and_never_wraps` — a 5000 character message prints as one line.
- `show_names_a_tool_and_never_prints_a_result_body` — a tool result row shows the tool and a
  byte count. A secret inside a result never reaches the terminal by accident.
- `show_marks_a_sibling_branch` — two answers to one question are shown as two branches.
- `show_sends_nothing_to_a_model` — the real binary runs `sessions show` with a provider name
  that does not exist and no credential in the environment. It still prints the records, so
  looking is free.
- `list_prints_six_columns_inside_eighty` — the header and every row fit 80 columns.
- `list_shows_an_unreadable_row_with_its_reason` — the id and the reason survive, and every
  unknown field is a dash.
- `list_shows_no_turn_count` — no column reports a number that needs a whole file.
- `the_fork_flow_works_from_the_two_printed_commands` — the end-to-end proof, driven through
  the real binary. Write a session, run `show`, take a record id from its output, run
  `fork --at <that id>`, and the new file holds the branch. **The test reads the id from the
  real output, so it fails if the id is not printed.** This is the owner's ask, pinned as one
  test.

**Delete, which had no test at all.**
- `delete_removes_the_session_and_its_sidecars` — the file and every `<id>.*.sidecar` are
  gone.
- `delete_keeps_a_session_forked_from_this_one` — the fork survives, because it is its own
  file.
- `delete_of_an_absent_session_names_the_path` — the error names the file, not just a reason.

**The twice cases, because twice has caught two defects here.**
- `two_appends_mint_two_ids` — the plain case, stated so the harder ones have a baseline.
- `a_fork_of_a_fork_keeps_every_id_unique` — fork twice, then append.
- `a_resume_of_a_resume_keeps_every_id_unique` — reopen twice, then append.
- `a_reopen_of_a_reopened_file_writes_one_more_reopened_record` — the invariant from
  `SPEC-sessions` still holds after two reopens.
- `create_twice_with_one_id_never_truncates` — the second create fails, and the first file
  keeps every byte. It must fail against `File::create`.

**Boundaries the row builder must survive.**
- `a_file_smaller_than_the_tail_window_builds_a_row` — a two-line file needs no seek.
- `a_first_prompt_beyond_the_head_lines_leaves_the_title_empty` — the row states no title
  rather than a wrong one.
- `an_empty_file_is_an_unreadable_row` — a zero-byte file never panics.
- `a_torn_last_line_does_not_produce_a_wrong_closed_flag` — an append with no `fsync` can
  leave half a line, and the row must not read it as a close.

**The new header fields round-trip.**
- `the_new_header_fields_survive_a_write_and_a_read` — `session_id` and `forked_from` come
  back as they went in.
- `a_record_id_prints_as_its_own_text` — `Display for RecordId`, which every new error
  message needs.

**The lock, and two live processes.**
- `a_second_process_cannot_open_a_live_session` — a second `lock` on one session returns
  `SessionError::Busy`, and the message names the session.
- `two_worktrees_continuing_at_once_never_share_a_file` — the defect from the review, driven
  end to end. Two worktrees of one repository share a project key. Two runs open at the same
  moment, and each ends with its own file whose record ids are unique. Its partner
  `a_second_process_cannot_continue_a_live_session` is what fails against an implementation
  with no lock.
- `newest_open_skips_a_locked_session` — `--continue` moves past a live session, and takes
  the next one.
- `a_lock_is_released_when_the_process_ends` — dropping the lock frees the session, so a
  crash cannot lock a session forever.
- `a_read_only_command_needs_no_lock` — `list` and `show` both work on a locked session.
- `a_filesystem_that_cannot_lock_is_refused` — a stubbed lock failure returns
  `LockUnsupported`, and the run stops. It must fail against an implementation that warns
  and continues.

**Continue, resume, and fork.**
- `resume_is_an_alias_for_continue` — `--resume` and `--continue` resolve to the same
  selector, for the bare form and for the value form. One argument, two spellings.
- `a_bare_flag_takes_the_newest_session` — `--continue` with no value resolves to `Newest`.
- `a_flag_with_a_value_takes_that_session` — `--resume=<id>` resolves to `Named`.
- `the_flag_does_not_swallow_the_prompt` — `rho run --continue "fix the bug"` keeps the
  prompt and resolves to `Newest`. It must fail without `require_equals`.
- `a_session_id_as_the_prompt_is_refused_with_a_hint` — `--resume 20260825-09`, with a space,
  is refused, and the message names `--resume=<id>`. Measured today as a silent wrong run.
- `the_session_flag_given_twice_is_an_error` — `--continue --resume=<id>` is refused by the
  alias itself, with no custom conflict rule.
- `continue_takes_the_newest_session_for_the_project` — two sessions in two worktrees of
  one repository, and the bare flag takes the newer.
- `continue_states_which_session_it_took` — the output names the id, the title, and the
  directory.
- `continue_with_no_session_is_an_error_that_says_what_to_do` —
  `NoSessionToContinue` names the project and the way forward.
- `allow_widen_alone_is_an_error` — the flag with no session flag is refused.
- `a_resume_that_would_widen_is_refused_on_the_command_line` — the run stops, and the
  message names `--allow-widen`. It closes the error that names a flag rho lacks.
- `allow_widen_permits_the_wider_resume_on_the_command_line` — with the flag the run starts.
- `a_resumed_context_holds_no_live_result_handle` — every stale preview is rewritten to say
  the evidence expired, and the byte count survives. See
  `D-a-stale-result-handle-expires-on-resume`.
- `fork_copies_the_branch_and_keeps_the_original` — the original file is byte-identical, and
  `a_row_shows_its_fork_origin` proves the new file names its origin.
- `a_crash_offers_the_unclosed_session` — a store with an unclosed session offers it once.
- `a_closed_session_is_never_offered` — a store of closed sessions offers nothing.

**The recorder, which wrote no assistant turn.**
- `a_run_records_the_assistant_text_of_a_turn` — a scripted stream of text deltas leaves one
  `Message` record with role `Assistant` and the joined text. It must fail against a
  recorder with no `TurnEnd` arm.
- `a_run_records_a_tool_call_before_its_result` — the file holds the `ToolCall` block, and
  it appears on an earlier line than its `ToolResult`.
- `every_tool_call_on_disk_has_a_result_on_disk` — the invariant over a scripted run with
  three tool calls. It is the pairing rule, checked on the file the run wrote.
- `a_recorded_run_replays_as_a_valid_message_list` — read the file back, rebuild the branch,
  and assert the pairing is complete and the assistant text survives. This is the resume
  path, so it is the test that proves the feature works.
- `an_empty_turn_writes_no_assistant_record` — a turn with no text and no call writes
  nothing, so a file gains no blank message.
- `a_reasoning_payload_survives_the_recorder_verbatim` — a provider replay payload reaches
  the file unchanged.
- `a_cancel_records_the_real_tool_arguments` — a cancel after a completed `ToolCallEnd`
  writes the arguments the provider sent, not an empty object.
- `an_empty_name_is_refused_by_the_recorder` — `record_name` with a blank string is an
  error.

**The budget.**
- `a_list_of_five_hundred_sessions_reads_only_the_head_and_the_tail` — the deterministic
  test. It returns 500 rows, and one of the 500 files is a **sentinel**: it carries a `Name`
  record after `ROW_HEAD_LINES` lines and more than `ROW_TAIL_BYTES` before the end. A
  bounded `rows` cannot see that record, so the sentinel row reports
  `title_is_explicit == false`. A `rows` that decodes the whole file reports `true`. So the
  test fails against a full decode with no seam under `rows`.

  A review found that the first draft asserted a byte bound over a method that opens its own
  files, so no counting source could see the defect. That is `D-bash-line-cap` rebuilt one
  level up. See `D-the-budget-test-needs-an-observable-difference`.
- `create_minted_remints_after_a_collision` — the suffix source hands out one taken suffix
  and then a free one, through `create_minted_from`. The second id differs from the first,
  and the existing file keeps every byte. It must fail against a `create_minted` that
  propagates the collision.
- `create_minted_gives_up_after_mint_attempts` — a suffix source that repeats one value
  forever returns an error after `MINT_ATTEMPTS` tries, and never spins.
- The wall-clock number is **measured and printed, and it asserts nothing**. A shared CI
  runner makes a 100 millisecond assertion flaky, and on a fast machine it would pass against
  a full decode of small files. The number goes into `docs/benchmarks.md` with its command.
  The byte bound is what proves the budget holds.

## 12. The contract compiles

Every signature above was pasted into a scratch crate outside this repository, with
`serde`, `thiserror`, and a stand-in for each `rho-core` type. It builds.

The check found two errors, and both are recorded where they belong.

1. `RecordId` had no `Display` impl, so two error messages did not compile. Section 7e adds
   the impl.
2. `PathBuf` has no `Display` impl either, so `LockUnsupported` did not compile. It calls
   `path.display()` instead.

A reviewer reading prose would have missed both.

## 13. What the review changed

The contract went through two reviewers before any code, per step 9 of `AGENTS.md`. One
read it for correctness, one for security. Both were told the defect history and asked to
assume another defect of the same family was present. Both found one.

**Three blocking findings.**

1. **The row test could not fail.** The first draft named
   `a_row_never_decodes_the_whole_file` and gave it no seam. It could only assert the
   finished row, which a full-file implementation produces identically. That is the
   memory-cap defect of `D-bash-line-cap` repeated. Section 5a adds `row_from`.
2. **The version rule was fail-open.** A `#[serde(tag = "type")]` reader cannot tell an
   unknown leaf from an unknown chain record. So "an unknown leaf is safe" was
   `ToolKind::Other` again. Section 6a replaces the declared class with referential
   integrity, checked from the child side.
3. **A hostile repository chose the path.** A `.git` file can hold
   `gitdir: ../../../../etc`, and the last component became a directory name. Section 3a
   bounds the read and sanitizes the key to one segment. That is the `confine` family.

**What else changed.**

- The store sets `0o600` and `0o700`, following `transcript.rs`. The first draft said
  nothing about mode bits, and a default umask would have made every conversation readable.
- `no_credential_reaches_the_file_on_the_run_path` is renamed. Redaction matches a key name
  only, so the old name claimed a guarantee the code does not give. A test name is a claim.
- `branch_messages` returns a `Result`. Its `None => break` was a silent early stop.
- Three row fields are cut: `provider`, `approval`, and `sandbox`. No renderer read them.
- The four defect tests must drive the real store path, never `seed_ids` by hand.
- The claim that the list tests keep every assertion was false. They are ported, and the
  ported test is named.
- Six untested fields gained tests: the start time, the last activity, the size, and the
  absent-usage case.

## 14. What a reviewer must answer

1. Does a new case need an edit to shared code? Name the case.
2. Which public item in this spec has no test?
3. Is any default fail-open? Look at the version rule and at the row fallbacks.
4. Can `a_row_never_decodes_the_whole_file` fail against a full-file implementation?
5. Does any test here pass against the defect it names?

## 15. What the second review changed, before the wiring lane wrote code

The wiring lane reviewed this contract again, because sections 3, 3a, 3b, 3c and 4 had
already shipped between the first review and the start of the work. A reviewer that did not
write the spec answered the five questions of section 14 against the real tree.

**Nine findings. Each one is corrected above.**

1. **Sections 3, 3a, 3b, 3c and 4 are built.** Section 3 now says so and names the file.
   One rule arrived with the code and was missing here: a leading dot is stripped from a
   key, so a project named `.config` does not hide the store. `PrefixMatch` was not built
   with them, because it needs the store to resolve anything.
2. **`NewSessionWithoutId` was named and never defined.** Section 7 defines it now.
   `create_minted` also returns the id it used, because a retry replaces the first one and
   a caller cannot recompute it.
3. **The new `SessionSummary` replaces the old one.** It is not a second type of the same
   name. `SessionStore::list` and the four-field `SessionSummary` both go, in the same
   change, or the crate does not compile.
4. **`Record::Session` gaining two fields breaks three sites.** `parse_header` matches the
   variant with no rest pattern, and `create` and `fork` both build it as a literal. All
   three are edits this lane must make, and none of them is optional.
5. **Section 8c named `require_equals` and no argument count.** A bare `--continue` yields
   no value without `num_args = 0..=1`. Section 8c states both now.
6. **The recorder writes no assistant turn.** This is the worst finding, and the lane found
   it, not the reviewer. Section 6d states the fix, and
   `D-a-recorder-writes-the-assistant-turn` records the decision.
7. **The budget test could not fail.** It asserted a byte bound over a method that opens
   its own files, so no counting source could observe a full decode. Section 11 replaces it
   with a sentinel row. See `D-the-budget-test-needs-an-observable-difference`.
8. **`create_minted` had no test, and no seam.** Section 11 names two tests, and section 7
   adds `create_minted_from`. Without the seam a test cannot force a collision, because two
   calls in one millisecond draw two different suffixes.
9. **Two named tests need a provider stub, and two need config keys.** The config keys
   `ephemeral` and `session-file` already exist in `rho-config`, so this lane reads them and
   edits nothing there. `rho-provider-testkit` supplies the stub, so no provider crate
   changes either.

### 15a. The extension point, restated

A reviewer asked which new case needs an edit to shared code. One does: a second storage
backend. Section 10 states it, keeps `SessionStore` a struct, and says a sqlite store is a
fork. That is a deliberate choice, and it is written down rather than discovered.

Everything else arrives without an edit to shared code. A new frontend calls the same store.
A new record arrives as a leaf, and the referential check of section 6a catches a chain
record that a build cannot read.

### 15c. What the final review changed

A reviewer that did not write the code read the whole diff, with the same defect history. It found
six things, and each one is fixed.

1. **A fail-open on the new-session path.** `open_recording` degraded **every** failure to
   ephemeral, including `LockUnsupported`. So a filesystem that cannot lock would have warned and
   continued, which section 7d forbids. A lock refusal now stops the run. Test:
   `a_filesystem_that_cannot_lock_stops_a_new_run`.
2. **`a_forged_header_cannot_widen_a_run` was theatre.** Its only runtime assertion was that a
   narrower run succeeded, which passes whether or not the header is trusted. It drives a table of
   eight stored-and-live mode pairs now, so a build that read the run's mode from the file breaks a
   row. The code-shape half moved to `the_run_never_takes_its_permission_from_a_session_file`.
3. **`a_forged_fork_origin_opens_no_file` was vacuous.** The sentinel did not exist, so nothing
   could read it. The sentinel is a real file with a marker inside now, and the test asserts the
   marker reaches neither the model nor a printed row.
4. **A second mint loop.** `rho-cli` minted a fork id with its own bounded retry, and its give-up
   branch had no test. `SessionStore::fork_minted` and `fork_minted_from` replace it, so one rule
   has one spelling. Tests: `a_fork_mints_a_free_id_through_the_store`,
   `a_fork_gives_up_after_mint_attempts`.
5. **A delete could break the lock.** `delete` unlinks `<id>.lock`, and `flock` binds to an inode.
   So a delete during a live session would let the next writer lock a **new** inode, and two
   writers would both believe they held the session. A delete takes the lock first now, and a live
   session refuses it. Test: `delete_refuses_a_live_session`.
6. **`SessionError::NoSuchRecord` had no test**, and a user reaches it by typing `--at r99`. Test:
   `a_fork_at_a_record_the_file_does_not_hold_is_refused`.

**One finding is accepted and not fixed.** `is_locked_elsewhere` probes a lock and releases it, so
`newest_resumable` can name a session that another process takes first. The caller then gets
`SessionError::Busy` and stops. The invariant holds, because the real lock is taken before any
write, so two processes never write one file. A reservation would make a read-only query return a
resource a caller must remember to drop, and that cost is worse than one error message. The doc
comment on `is_locked_elsewhere` states it.

### 15b. Three more items needed a name

Section 11 names `a_hand_built_leaf_parent_is_refused`, and section 7e listed no error for
it. So the refusal would have arrived as a bare decode message, and a caller could not match
on it. Section 7e adds `SessionError::LeafParent`, which names both ids.

A fork at a record the file does not hold needed a name too. A user typing
`--at r99` reaches it, so section 7e adds `SessionError::NoSuchRecord`.
`SessionError::MintExhausted` names the bounded retry of section 7c.

10. **`branch_messages` could not tell the root from a hole.** The header record is not in
    `entries`, so the first entry of every real file names a parent the walk cannot see.
    Section 6b adds the `root` parameter, and `ReadResult` gains `header_id`.

### 15d. What the review fleet and codex found

The lane ran a second review phase after the suite was green: four subagents with one lens each,
and `codex review` from outside this harness with no sight of their findings. Ten findings were
real. Each is fixed, and each has a mutation proof in
`docs/verification/session-store-wiring.md` section 12.

**One critical.**

1. **A cyclic parent chain looped for ever.** Section 6a states the rule now. Found independently by
   the correctness lens and the security lens, which is the strongest signal in this review.

**Two that bypassed a rule through a config key.** Codex found both.

2. **A `session-file` took no lock.** Two runs with the key set appended to one file, and both
   seeded their record ids from one read.
3. **A `session-file` skipped the permission check.** A file written under `read-only` came back
   under `allow-all` in silence, which is `D-resume-never-widens` reached by a different door.

Both are settled by `D-a-named-session-file-is-a-session-like-any-other`: **an existing named file
is a resume.** It locks, it checks the stored modes, and it replays. `SessionStore::lock_file`
locks any path and `lock` calls it, and `rebuild` holds the shared half of both resume paths, so
one rule has one spelling.

**Four more.**

4. **A fork at a leaf record left a leaf as the head**, so the next append through the returned
   writer named a leaf as its parent. `SessionWriter::append` already knew the rule, and the fork's
   own copy loop was a second spelling of it. Codex found it.
5. **A tool message with a bare text block printed its body.** Section 8a states the role rule now.
   The security lens found it, and it was a real gap against a promise `docs/guide/sessions.md`
   makes.
6. **A long model id pushed the close state off the `show` header.** Section 8a states the budget
   now. The test lens reproduced it with a real Bedrock id.
7. **`expire_stale_result_handles` was string surgery with three holes:** it rewrote a `Text` block
   only, it rewrote one preview per block, and it kept a nested tag inside the head it kept. The
   security lens found all three. The impact is a wasted turn and not a leak, because the store
   behind the handle is dead, and a promise rho makes must still hold.

**Two about a test rather than the code.**

8. **The grep of `cli.rs` passes against `if false`.** The test lens proved it by running the break.
   `record_and_print` now holds the whole recording lifecycle in one function, and two tests drive
   it on a real file: `the_whole_lifecycle_runs_in_order` and
   `the_prompt_is_recorded_before_the_answer_even_when_the_run_fails`. The grep is a backstop for
   one call now, and not the guard for the lifecycle. **The residual limit is stated in section 16.**
9. **`two_worktrees_continuing_at_once_never_share_a_file` opens no concurrent run.** It proves the
   shared project key through the real binary. The concurrency is proved by
   `a_second_process_cannot_continue_a_live_session`, which really opens two recordings, and by
   `a_second_process_cannot_open_a_live_session`, which locks in a child process. The test's own
   comment says so now, rather than leaving its name to overstate.

**Three findings were accepted and not fixed.** Each is stated where a reader will meet it:

- `is_locked_elsewhere` probes a lock and releases it, so `newest_resumable` can name a session
  another process takes first. The caller then gets `Busy`. The invariant holds, because the real
  lock is taken before any write.
- A project key has no length cap, so a four kilobyte `gitdir:` line yields a long directory
  component and `ENAMETOOLONG`. That is a degrade and not a traversal.
- The prior-art table in section 7d is a reading of pi, jcode and fx. It is editorial context about
  other projects, and no rho test can back it.

## 16. What no test covers

`AGENTS.md` step 8 asks for this list, and a reader deserves it in the contract and not only in a
report.

- **`run_headless` calling `record_and_print`.** The lifecycle is behaviour, and this last hop is a
  grep. `crates/rho-cli/src/provider.rs` belongs to another lane, so this crate cannot inject a stub
  provider and drive `run_headless` in process. A break that wraps the call in `if false` still
  passes the suite. `docs/verification/session-store-wiring.md` drives it against live Bedrock
  instead, and that is the only guard.
- **The terminal.** `rho-tui` records nothing and `/sessions` opens no picker.
- **A `flock` failure from a real filesystem.** `classify_lock_failure` is pure and every code is
  tested, and no test makes a real network filesystem refuse a lock.
- **`SessionStore::create_file`, `SessionSelector::resumes`, `SessionsAction`, and
  `sessions_command::run`** are each reached only through a caller. Each has a test that fails when
  it breaks, and none has a test that names it.
