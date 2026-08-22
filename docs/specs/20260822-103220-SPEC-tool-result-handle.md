# SPEC-tool-result-handle — A capped tool result, and a handle back to it

Status: delivered. Verified live; see `docs/verification/tool-result-handle.md`.
Owning crates: `rho-core` (the cap and the store), `rho-tools` (the tool).
Features: F-tool-result-handle.

Decisions this spec implements: D-tool-result-handle, D-cap-at-one-choke-point,
D-stored-result-inherits-session-trust. It completes D-cap-a-large-tool-result.

## 1. The problem, stated twice

**The context is unbounded.** `Session::finish_tool` appends a whole tool output to the
context with no cap. `bash` bounds its own output at 100,000 bytes, and no other tool does.
An MCP server or a plugin that returns ten megabytes puts ten megabytes into the window, and
the user pays for it on every later turn. No tool in `rho-tools` behaves that way, which is
why no test caught it. The defect needs a peer rho does not ship.

**The tail is unreachable.** Where a cap does exist, the dropped text is gone as far as the
model is concerned. `bash` appends `[truncated: output over 100000 bytes.]` and the model's
only recovery is to run the command again. A second run costs a second process, a second
wait, and it may not even be repeatable.

The cap and the read-back are two halves of one feature. rho shipped the first half.

**One limit, stated up front.** `bash` cuts its own output at 100,000 bytes before it returns,
so a store never holds more than that for a `bash` result. This feature does not recover the
rest, and no document here claims it does. See D-bash-cap-limits-the-store and
`F-bash-streams-to-the-store`.

## 2. The cap always runs. The store is what makes it kind

This is the central rule of this spec, and an earlier draft got it wrong.

**The cap is not optional.** Every tool result meets `max_result_bytes` before it reaches the
context, whatever the caller configured. D-cap-at-one-choke-point rules out trusting a peer
to bound itself, and a cap a caller can leave off is the same thing with an extra step. An
earlier draft made the cap opt-in, so the default build still had the ten-megabyte defect.
That is the `ToolKind::Other` shape: the safe path existed and the default did not take it.

**The store is optional, and it changes what the cap costs.** With no store, an oversize
result is cut at `max_result_bytes` and the tail is lost, exactly as `bash` behaves today.
With a store, the context keeps a much smaller preview and a handle, because the tail is
reachable. So configuring a store makes the context **smaller**, never larger.

| | Context keeps | Tail |
| --- | --- | --- |
| No store | `max_result_bytes` (64 KiB) plus a note | Lost |
| Store | `preview_bytes` (4 KiB) plus a handle | Readable by handle |

## 3. What the model sees

A result under the threshold is unchanged. rho adds nothing, so a small result costs no extra
byte and the common case is untouched.

With a store, an oversize result becomes this and nothing else:

```text
<tool_result_preview handle="tr-4f2a91c8e0b3d756-000001" stored_bytes="412990" preview_bytes="4096">
…the first 4096 bytes, cut on a character boundary…
</tool_result_preview>
The full result is stored outside the context. Use read_tool_result with this exact handle
to read a byte range, or to search it for a literal string.
```

With no store, it becomes the first `max_result_bytes` plus:

```text
[rho cut this result at 65536 bytes of 412990. The rest is not available, because this
session has no result store.]
```

The note states the loss. A cut that says nothing lets the model treat a partial result as
whole, which is the defect `F-project-instructions` already had to fix once.

The preview is the head, because a command's first lines say what happened. A caller that
needs the tail instead replaces the preview strategy; see section 5.

## 4. The store, and the handle

The store is a trait, so a caller chooses where a payload lives. rho ships one implementation
that writes files beside the session file.

**A handle is a name, not a path.** It matches `^tr-[0-9a-f]{16}-[0-9]{6,}$` exactly. rho
validates the shape before it touches the filesystem, and it joins the name to the store
directory. A handle carrying a separator, a parent reference, or any other character is
refused. This is the rule `SPEC-project-instructions` section 3 applies to a filename, for
the same reason: an untrusted string must never become a path.

**The nonce is why the handle is not a sequence.** The first sixteen hex digits are a
per-store nonce, fresh on every open. An earlier draft used a bare sequence and argued that
guessing a handle was harmless, because the model had seen a preview of every result in its
own session. A review broke that argument on resume: a resumed session continues over the
same directory, the model never saw the earlier run's previews, and enumerating `tr-000001`
upward would pull a previous run's evidence into the context. That evidence can hold a
credential a tool read, per D-stored-result-inherits-session-trust.

The nonce fixes three things at once:

1. A resumed session cannot enumerate an earlier run's handles.
2. A new run cannot clobber an earlier run's files, because the names differ.
3. Two stores over one directory, such as a subagent sharing a session directory, cannot
   collide on a sequence number.

`read_tool_result` still answers `NotFound` for an absent handle, so it remains an existence
oracle for a handle the caller already holds. With a 64-bit nonce the model cannot reach a
handle it was never given, so the oracle answers only about its own results.

**Two puts must not race.** Tool dispatch can run in parallel, so two calls to `put` can ask
for a sequence number at the same time. The sequence comes from an atomic counter, and the
file is created with `create_new`, so a collision is detected rather than silently
overwriting. A detected collision retries with the next number.

**A reader never sees half a file.** `put` writes a temporary file and renames it into place.
A rename within one directory is atomic, so a concurrent `read_range` sees either nothing or
the whole payload, never a prefix with a wrong `total_bytes`.

## 5. Bounds

| Bound | Default | Why this number |
| --- | --- | --- |
| `max_result_bytes` | 64 KiB | Matches `MAX_RECORD_BYTES`, so the context and the record agree. |
| `store_threshold_bytes` | 16 KiB | Below this, a file costs more than it saves. |
| `preview_bytes` | 4 KiB | Enough to see what happened, small enough to ignore. |
| `read_default_bytes` | 8 KiB | A read the model can afford to repeat. |
| `read_max_bytes` | 64 KiB | One read never exceeds one whole result cap. |
| `max_matches` | 20 | A search that returns everything is the payload again. |

Every read is bounded, so no sequence of tool calls can pull a ten-megabyte payload into the
context in one step. A model that spends twenty turns reading slices has chosen to, and the
turn cap and the tool-call budget already bound that.

A cut for a preview or a range lands on a UTF-8 character boundary. A cut through a
multi-byte character produces invalid UTF-8 and a provider error.

## 6. Public API

This is the contract. It compiles as written.

```rust
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

/// Why a store operation failed.
#[derive(Debug, thiserror::Error)]
pub enum ResultStoreError {
    /// The handle does not match the documented shape. It was never used as a path.
    #[error("handle {0:?} is not a valid result handle")]
    MalformedHandle(String),
    /// No stored result has that handle in this store.
    #[error("no stored result has handle {0}")]
    NotFound(String),
    /// The store could not be read or written.
    #[error("result store input or output failed: {0}")]
    Io(String),
}

/// One slice of a stored result.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredSlice {
    /// The text in the requested range, cut on a character boundary.
    pub text: String,
    /// The zero-based byte offset the slice starts at.
    pub start_byte: usize,
    /// The byte offset one past the end of the slice.
    pub end_byte: usize,
    /// The whole payload size, so the model can plan the next read. This is always the
    /// whole size, even for an empty slice past the end, or the model cannot learn it is
    /// done.
    pub total_bytes: usize,
}

/// One literal match inside a stored result.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredMatch {
    /// The one-based line number the match is on.
    pub line_number: usize,
    /// The byte offset of the match within the payload.
    pub start_byte: usize,
    /// The whole line, bounded. A very long line is cut on a character boundary.
    pub line: String,
}

/// Where a large tool result is kept so the model can read it later.
///
/// A store belongs to one session. An implementation must refuse a malformed handle before
/// it builds any path from it.
#[async_trait]
pub trait ResultStore: Send + Sync {
    /// Store `text` and return its handle. The handle is unique within this store, even
    /// when two callers put at the same time.
    async fn put(&self, text: &str) -> Result<String, ResultStoreError>;

    /// Read at most `max_bytes` from `start_byte`.
    ///
    /// A range past the end returns an empty slice with the true `total_bytes`, not an
    /// error, so a model learns it has read everything.
    async fn read_range(
        &self,
        handle: &str,
        start_byte: usize,
        max_bytes: usize,
    ) -> Result<StoredSlice, ResultStoreError>;

    /// Find a literal string. Returns at most `max_matches` matches.
    ///
    /// The default scans the payload through `read_range`, in this process. It never puts a
    /// slice into the context, so the scan costs no tokens. A store whose backend can search
    /// on the server overrides this and saves the transfer.
    async fn search(
        &self,
        handle: &str,
        needle: &str,
        max_matches: usize,
    ) -> Result<Vec<StoredMatch>, ResultStoreError> {
        let _ = (handle, needle, max_matches);
        Ok(Vec::new())
    }
}

/// Chooses what part of an oversize result the context keeps, and how it reads.
///
/// This is the extension point for the shape of the cap. A caller that wants the tail, or
/// both ends, or a different block format, supplies an impl. It does not edit `rho-core`.
pub trait ResultPreview: Send + Sync {
    /// The bytes to keep from `text`, at most `max_bytes`, cut on a character boundary.
    fn select(&self, text: &str, max_bytes: usize) -> String;

    /// Render the block the context keeps for a stored result.
    ///
    /// Must be byte-identical for the same input, because a tool result joins the
    /// append-only log and is never rewritten.
    fn render_stored(&self, handle: &str, preview: &str, stored_bytes: usize) -> String;

    /// Render the block the context keeps when there is no store and the tail is lost.
    fn render_cut(&self, kept: &str, kept_bytes: usize, whole_bytes: usize) -> String;
}

/// Keeps the head of a result. The default, because a command's first lines say what
/// happened.
pub struct HeadPreview;

impl ResultPreview for HeadPreview {
    fn select(&self, text: &str, max_bytes: usize) -> String {
        let _ = (text, max_bytes);
        String::new()
    }
    fn render_stored(&self, handle: &str, preview: &str, stored_bytes: usize) -> String {
        let _ = (handle, preview, stored_bytes);
        String::new()
    }
    fn render_cut(&self, kept: &str, kept_bytes: usize, whole_bytes: usize) -> String {
        let _ = (kept, kept_bytes, whole_bytes);
        String::new()
    }
}

/// The bounds from section 5.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResultLimits {
    /// The most bytes one tool result may contribute to the context. Always enforced.
    pub max_result_bytes: usize,
    /// A result at or above this size is stored, when a store exists.
    pub store_threshold_bytes: usize,
    /// The preview kept in the context when the result is stored.
    pub preview_bytes: usize,
    /// The default size of one `read_tool_result` read.
    pub read_default_bytes: usize,
    /// The largest size one `read_tool_result` read may ask for.
    pub read_max_bytes: usize,
    /// The most matches one search returns.
    pub max_matches: usize,
}

impl Default for ResultLimits {
    fn default() -> Self {
        Self {
            max_result_bytes: 64 * 1024,
            store_threshold_bytes: 16 * 1024,
            preview_bytes: 4 * 1024,
            read_default_bytes: 8 * 1024,
            read_max_bytes: 64 * 1024,
            max_matches: 20,
        }
    }
}

/// A store that writes one file per result, beside the session file.
///
/// See D-stored-result-inherits-session-trust. The directory inherits the session file's
/// privacy and its lifetime.
pub struct FileResultStore {
    _directory: PathBuf,
}

impl FileResultStore {
    /// Open or create a store in `directory`.
    ///
    /// Each open takes a fresh nonce, so a resumed session can neither read nor overwrite
    /// an earlier run's results. See section 4.
    pub async fn open(directory: impl Into<PathBuf>) -> Result<Self, ResultStoreError> {
        Ok(Self {
            _directory: directory.into(),
        })
    }

    /// The nonce this store stamps into every handle it makes.
    pub fn nonce(&self) -> &str {
        ""
    }
}

/// True when `handle` matches `^tr-[0-9a-f]{16}-[0-9]{6,}$`.
///
/// Every store calls this before it builds a path.
pub fn is_valid_handle(handle: &str) -> bool {
    let Some(rest) = handle.strip_prefix("tr-") else {
        return false;
    };
    let Some((nonce, sequence)) = rest.split_once('-') else {
        return false;
    };
    nonce.len() == 16
        && nonce.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        && sequence.len() >= 6
        && sequence.bytes().all(|b| b.is_ascii_digit())
}

/// How a session caps a tool result, and where the tail goes.
///
/// `limits` and `preview` always apply. `store` is what makes the tail readable.
pub struct ResultPolicy {
    pub limits: ResultLimits,
    pub preview: Arc<dyn ResultPreview>,
    pub store: Option<Arc<dyn ResultStore>>,
}

impl Default for ResultPolicy {
    /// The cap runs with no store. So a default session is bounded, and its tail is lost.
    fn default() -> Self {
        Self {
            limits: ResultLimits::default(),
            preview: Arc::new(HeadPreview),
            store: None,
        }
    }
}
```

`SessionConfig` gains one field with a real default, and one builder method. It gains no
constructor argument, because D-no-four-argument-session-new forbids widening the
constructor.

```rust
# use std::sync::Arc;
# pub struct ResultPolicy;
# impl Default for ResultPolicy { fn default() -> Self { Self } }
# pub struct SessionConfig { pub results: ResultPolicy }
impl SessionConfig {
    /// Replace the result policy. The default already caps; this adds a store or a
    /// different preview.
    pub fn with_result_policy(mut self, policy: ResultPolicy) -> Self {
        self.results = policy;
        self
    }
}
```

## 7. The tool

`read_tool_result` is a `Tool` in `rho-tools`. Its `kind()` is `ToolKind::Read`, because it
reads evidence rho already gathered and changes nothing.

| Argument | Type | Meaning |
| --- | --- | --- |
| `handle` | string, required | Copied exactly from a preview block. |
| `start_byte` | integer, optional | Defaults to 0. |
| `byte_count` | integer, optional | Defaults to `read_default_bytes`, clamped to `read_max_bytes`. |
| `query` | string, optional | A literal search. When present, `start_byte` and `byte_count` are ignored. |

A failure names the reason and what to do. A wrong handle is the most likely mistake, so its
message says a handle must be copied exactly from a preview and is scoped to this session.

The tool is registered only when the session has a store. Advertising a tool that always
fails would spend schema bytes on every turn and teach the model a false capability.

## 8. Test cases

**The cap always runs, in `rho-core`**

- `a_small_result_is_unchanged` — a result under the threshold reaches the context byte for
  byte, and nothing is stored.
- `a_large_result_is_cut_with_no_store` — the default policy bounds the result to
  the result cap. This is the regression test for the ten-megabyte defect.
- `the_cut_note_states_the_whole_size` — the note names the bytes kept and the bytes there
  were, so the model knows the result is partial.
- `the_cap_applies_to_a_tool_that_does_not_bound_itself` — a stub tool returning ten
  megabytes cannot put ten megabytes in the context. This is D-cap-at-one-choke-point.
- `an_error_result_is_capped_too` — a huge failure message is bounded like any other result.
- `a_result_below_the_store_threshold_is_not_stored` — below the threshold a file costs more
  than it saves, so the result is unchanged.

**The store path, in `rho-core`**

- `a_large_result_is_replaced_by_a_preview` — the context holds the preview block, and a
  distinctive string from the middle of the payload is **absent** from the context.
- `a_large_result_keeps_the_head` — the preview is the first bytes, not the last.
- `the_preview_states_the_stored_size` — the block names the full byte count.
- `a_capped_preview_cuts_on_a_character_boundary` — a cut inside a multi-byte character does
  not split it.
- `a_store_keeps_less_context_than_no_store` — with a store the context is smaller, so
  configuring one is never a cost.
- `a_store_failure_keeps_the_cap` — when the store cannot write, the context still gets a
  bounded result and the user gets a warning. It never gets the whole payload, because the
  failure that fills a disk is the large write itself.

**The store, in `rho-core`**

- `put_then_read_returns_the_text` — a round trip.
- `handles_are_unique_within_a_store` — two sequential puts return different handles.
- `concurrent_puts_never_share_a_handle` — many puts at once yield distinct handles and
  distinct payloads. A sequence read without an atomic counter fails this.
- `a_partial_write_is_never_read` — a reader sees the whole payload or nothing, because `put`
  renames into place.
- `read_range_reports_the_total_size` — `total_bytes` is the whole payload.
- `read_range_past_the_end_returns_empty_with_the_true_total` — an empty slice still reports
  the real size, so a model can tell it is done rather than loop.
- `read_range_reports_its_own_offsets` — `start_byte` and `end_byte` describe the slice
  returned, not the slice asked for.
- `read_range_clamps_to_the_maximum` — a request over the read ceiling is clamped.
- `read_range_cuts_on_a_character_boundary` — a range inside a character does not split it.
- `search_finds_a_literal_with_its_line_number` — the match names its line and its offset.
- `search_returns_nothing_for_no_match` — an empty result, not an error.
- `search_caps_the_match_count` — a needle on every line returns `max_matches`.
- `the_default_search_scans_through_read_range` — a store that implements only `put` and
  `read_range` still searches, so the required surface really is two methods.
- `an_unknown_handle_is_not_found` — the error is `NotFound`.
- `opening_a_store_creates_its_directory` — `open` on a missing directory succeeds.

**Security, in `rho-core`**

- `a_handle_with_a_separator_is_refused` — a handle containing `/` is `MalformedHandle`, and
  no path is built.
- `a_handle_with_a_parent_reference_is_refused` — a handle containing `..` is
  `MalformedHandle`.
- `a_handle_of_the_wrong_shape_is_refused` — an empty handle, a bare number, a handle with an
  uppercase nonce, and a short nonce are each refused.
- `is_valid_handle_accepts_only_the_documented_shape` — the predicate matches section 4.
- `a_malformed_handle_never_reaches_the_filesystem` — a store whose directory has been
  deleted still returns `MalformedHandle`, which proves the check runs first.
- `a_reopened_store_cannot_read_an_earlier_handle` — a second open takes a new nonce, so a
  resumed session cannot reach the previous run's results. This is the review finding in
  section 4.
- `a_reopened_store_does_not_clobber` — an earlier run's file still exists and still holds its
  own payload after a second store writes.

**The wiring, in `rho-cli`**

- `the_result_store_opens_and_is_owner_only` — the store really opens, and its directory is
  owner-only, because a stored result holds whatever a tool read.
- `the_result_store_is_removed_with_its_session` — dropping the guard removes the directory, so
  a stored result never outlives its session.
- `read_tool_result_is_advertised_only_with_a_store` — the tool reaches the model when a store
  exists, and not otherwise. Without this the feature would be a library nobody calls.

**Pinned invariants, in `rho-core`**

- `default_result_limits_match_the_spec` — the section 5 numbers are the shipped numbers.
- `head_preview_renders_byte_identically` — the same input renders the same bytes, twice.
- `head_preview_selects_the_head` — `select` takes the first bytes.

**The tool, in `rho-tools`**

- `read_tool_result_returns_a_range` — the tool reads the bytes it asked for.
- `read_tool_result_searches_for_a_literal` — a query returns matches, not a range.
- `read_tool_result_defaults_its_range` — no `start_byte` or `byte_count` reads from zero.
- `read_tool_result_rejects_a_missing_handle` — a call with no handle is an error result.
- `read_tool_result_explains_an_unknown_handle` — the message says a handle is
  session-scoped and must be copied exactly.
- `read_tool_result_is_a_read_kind` — `kind()` is `ToolKind::Read`, so a read-only policy
  allows it.
- `read_tool_result_refuses_a_malformed_handle` — the tool returns an error result rather
  than panicking.
- `read_tool_result_reports_reading_past_the_end` — the model is told the true size, so it
  learns it is done rather than looping.

## 9. Out of scope

- **Redacting the stored payload.** D-stored-result-inherits-session-trust names this and
  leaves it open. It needs a free-text detector, not a JSON key match, and it belongs with
  `F-no-secrets-in-logs`.
- **Pruning a store.** A store dies with its session directory. A retention policy needs a
  decision about what a user may delete.
- **A summary instead of a preview.** A summarising model call would spend tokens guessing
  which part matters, and a wrong guess destroys the evidence. D-tool-result-handle refuses
  it. A caller who wants one implements `ResultPreview`.
- **A regular expression search.** `search` takes a literal. Adding a pattern means adding a
  trait method, which widens the contract for every implementation, so it needs its own
  decision. An earlier draft claimed it could arrive behind the existing method, and that was
  wrong.
- **Storing a non-text result.** `put` takes `&str`. An image or a binary payload would need
  a second method and a content type, so it is a contract change, not a later detail.
- **A store for a subagent.** A child inherits the parent's bounds and gets no store, so an
  oversize child result is cut with a note. A child has no `read_tool_result` tool, because the
  tool is registered per session, and a store the child cannot read from would swallow the tail
  silently. Giving a child its own store means giving it the tool, and that is its own change.
  `a_child_inherits_the_parent_result_bounds` pins the inheritance.

## 10. What driving it for real changed

Two things, and the first is the more useful.

**A run passed for the wrong reason.** A 2.6 MB command output with a marker at byte 1,308,924
was answered correctly by the model, using `grep` on the file rather than the stored result.
`bash` had cut its output at 100,000 bytes, so the store never held the marker. The run was
about to be recorded as a pass. Checking the byte offset afterwards is the only thing that
caught it. The limit is now stated in section 1, D-bash-cap-limits-the-store records it, and
`F-bash-streams-to-the-store` carries the fix.

**Two tests passed against broken code.** Step 7 broke the implementation eight ways, and two
guards did not trip.

`a_capped_preview_cuts_on_a_character_boundary` used a two-byte character, and the preview
bound is 4,096 bytes. 4,096 is even, so the cut never fell inside a character, and the test
passed with the whole boundary walk deleted. It now uses a three-byte character and asserts
that the bound is not a multiple of three.

`a_partial_write_is_never_read` only checked that no temporary file was left behind, which is
also true when `put` writes the payload directly. It now reads the handle in a loop while a
large put runs, and asserts every size a reader observes is the whole size.

Both are the family D-bash-line-cap already named: a test that passes against the bug it was
written for buys false confidence.
