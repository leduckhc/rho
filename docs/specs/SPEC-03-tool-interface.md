# SPEC-03 — Tool interface

Status: draft for sprint 1.
Owning crates: `rho-core` (the trait), `rho-tools` (the built-in set).

A tool is a typed unit the model can call. This spec defines the `Tool` trait,
JSON Schema generation, argument validation, the built-in tools, path confinement
to the session root, the approval model, and the `bash` timeout and streaming
rules. Sandbox and approval boundaries are part of the contract, not only the
signatures.

Features covered: F-20 (tool trait), F-21 to F-27 (the built-in tools), F-28
(path confinement), F-29 (approval gate). The tool `kind` and the async approval
policy exist so `rho-acp` can report ACP tool kinds and drive
`session/request_permission`; see `SPEC-06`.

## 1. The trait

```rust
use crate::{CancelToken, ContentBlock};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// The ACP tool category. The values mirror the ACP `ToolKind` set exactly, so
/// `rho-acp` forwards the value with no remap. See `SPEC-06`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    Other,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),
    #[error("path {0} escapes the session root")]
    PathEscape(PathBuf),
    #[error("permission denied by approval policy")]
    Denied,
    #[error("timed out after {0:?}")]
    Timeout(Duration),
    #[error("io error: {0}")]
    Io(String),
    #[error("canceled")]
    Canceled,
}

/// The result of a tool run. `content` holds only `Text` or `Image` blocks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolOutput {
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub is_error: bool,
}

impl ToolOutput {
    /// A plain-text success result.
    pub fn text(text: impl Into<String>) -> Self {
        Self { content: vec![ContentBlock::Text { text: text.into() }], is_error: false }
    }
}

/// Ambient data passed to every tool run.
pub struct ToolContext {
    /// The confinement root. All paths resolve under this directory.
    pub session_root: PathBuf,
    /// Cancels the run. `bash` and other long tools must select against it.
    pub cancel: CancelToken,
    /// A channel for streamed output lines. `bash` sends stdout and stderr here.
    pub updates: tokio::sync::mpsc::Sender<String>,
}

#[async_trait]
pub trait Tool: Send + Sync {
    /// The tool name the model calls, for example `read`.
    fn name(&self) -> &str;
    /// A short description sent to the model.
    fn description(&self) -> &str;
    /// The ACP tool category. Defaults to `Other`.
    fn kind(&self) -> ToolKind {
        ToolKind::Other
    }
    /// A JSON Schema object for the arguments.
    fn input_schema(&self) -> serde_json::Value;
    /// Run the tool. Validate `args` first. Confine every path to the root.
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError>;
}
```

## 2. The registry

The registry holds the tools. The agent loop reads `specs()` once at session
start and advertises the full list in the first request. This keeps the prompt
prefix stable, per `SPEC-01`.

```rust
use std::sync::Arc;
use crate::ToolSpec;

pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.push(tool);
    }
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.name() == name)
    }
    /// The advertised tool list, in registration order.
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools
            .iter()
            .map(|t| ToolSpec {
                name: t.name().to_string(),
                description: t.description().to_string(),
                kind: t.kind(),
                input_schema: t.input_schema(),
            })
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
```

## 3. JSON Schema generation and validation

Each built-in tool defines its arguments as a `serde` struct. The tool builds the
`input_schema` by hand as a small JSON object. Sprint 1 does not add a schema
derive crate. The schema stays small and explicit.

Validation rule: `execute` parses `args` into the argument struct with
`serde_json::from_value`. A parse failure returns
`ToolError::InvalidArguments`. A round-trip test proves each schema matches its
struct.

## 4. Path confinement

Every path argument resolves under the session root. A path that escapes the root
returns `ToolError::PathEscape`. This is a hard boundary. It is the sandbox.

Rules:
- Join a relative path onto the root. Reject an absolute path that is not already
  under the root.
- Resolve the longest existing ancestor with the filesystem, then keep any
  trailing part that does not exist yet. A `write` tool targets a file that does
  not exist, so this case must resolve.
- A symlink in the existing prefix resolves to its real target. A symlink that
  points outside the root is rejected after this step.
- Compare canonical forms. Canonicalise the root once. On macOS `/var` is a
  symlink to `/private/var`, and the temp directory lives under `/var/folders`,
  so a text prefix check on an uncanonical root fails. The canonical comparison
  must not.
- An empty candidate resolves to the root itself.
- A candidate with a `NUL` byte is rejected with `InvalidArguments`. It never
  reaches the filesystem.

```rust
use std::path::{Path, PathBuf};

/// Resolve `candidate` under `root`. Return an error when it escapes the root.
pub fn confine(root: &Path, candidate: &Path) -> Result<PathBuf, ToolError>;
```

## 5. The approval model

Some tools are safe to run without asking. Others need approval. The policy
decides. A denied call returns `ToolError::Denied`, which the agent loop turns
into an error tool result. The model sees the denial and can change course.

Default policy for sprint 1:
- Read-only tools run without approval: `read`, `list`, `glob`, `grep`.
- Mutating tools ask by default: `write`, `edit`, `bash`.
- A headless run uses a policy that denies every mutating call unless configured
  to allow.

```rust
use async_trait::async_trait;
use crate::ToolKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    Allow,
    Deny,
}

#[async_trait]
pub trait ApprovalPolicy: Send + Sync {
    /// Decide whether a tool call may run. `kind` states what the tool does, so
    /// a policy decides from the typed category, not from the tool name.
    async fn approve(
        &self,
        tool: &str,
        kind: ToolKind,
        args: &serde_json::Value,
    ) -> ApprovalDecision;
}

/// Allows read-only tools, asks nothing, denies mutating tools.
pub struct ReadOnlyPolicy;

/// Allows every call. For a trusted, non-interactive run.
pub struct AllowAllPolicy;
```

`ReadOnlyPolicy` decides from `ToolKind::is_read_only`. That function is an
**allowlist**, and the direction matters. Only `Read`, `Search`, `Think`, `Fetch`,
and `SwitchMode` are read-only. Every other kind is mutating, and that includes
`Other`.

The boundary fails closed on purpose. `Tool::kind` defaults to `Other`, so a tool
author who forgets to declare a kind gets `Other`. A denylist would then approve
that tool, even when it deletes files. A denied safe tool is an annoyance. An
approved destructive tool is a breach.

`is_read_only` matches every variant and carries no wildcard arm. So a new
`ToolKind` variant fails to compile until somebody classifies it on purpose.

This is the single source of truth. It replaces a fragile tool-name list.

The agent loop consults the policy before it runs a mutating tool. A hook may
also block a call; see `SPEC-04`. The policy runs after the hooks.

ACP mapping: `ApprovalPolicy` is async by design so a frontend can ask a human.
`rho-acp` implements `ApprovalPolicy` by issuing an ACP
`session/request_permission` request with the four permission options
(`allow_once`, `allow_always`, `reject_once`, `reject_always`) and mapping the
selected outcome back to `Allow` or `Deny`. See `SPEC-06`.

## 6. The built-in tool set

All eight ship in `rho-tools`. Each confines its paths to the session root. The
`kind` column is the ACP `ToolKind` the tool reports.

- `read` (kind `read`) — read a file. Args: `path`, optional `offset`, optional
  `limit`. Returns text. Read-only. F-21.
- `write` (kind `edit`) — write a whole file. Args: `path`, `content`. Creates
  parent directories. Mutating. Needs approval. F-22.
- `edit` (kind `edit`) — replace an exact text span in a file. Args: `path`,
  `old_text`, `new_text`. Fails when `old_text` is absent or not unique. Uses
  `similar` to build a diff for the result. Mutating. Needs approval. F-23.
- `list` (kind `read`) — list a directory. Args: `path`. Returns entries.
  Read-only. F-24.
- `glob` (kind `search`) — match files by a glob pattern under the root. Args:
  `pattern`. Uses `globset`. Read-only. F-25.
- `grep` (kind `search`) — search file contents by a regex. Args: `pattern`,
  optional `path`, optional `glob`. Uses `ignore` to walk and to honour
  `.gitignore`. Read-only. F-26.
- `bash` (kind `execute`) — run a shell command. Args: `command`, optional
  `timeout_ms`. Streams output. Mutating. Needs approval. See section 7. F-27.

## 7. The `bash` tool

`bash` runs a command in the session root. It streams output and enforces a
timeout.

Rules:
- The working directory is the session root.
- Default timeout is 120000 ms. The `timeout_ms` argument overrides it. The
  maximum is 600000 ms.
- On timeout, kill the process group and return `ToolError::Timeout`.
- Stream stdout and stderr line by line on `ctx.updates`. The final `ToolOutput`
  holds the combined output.
- Truncate the stored output at 100000 bytes. Note the truncation in the result.
- Select against `ctx.cancel`. On cancel, kill the process group and return
  `ToolError::Canceled`.
- Remove every variable whose name looks like a credential from the child's
  environment. See `scrub_environment` in `crates/rho-tools/src/bash.rs`. The filter
  reads the name, not the value, and removes the name too.

### What actually bounds `bash`, stated plainly

An earlier version of this section said that "path confinement and the approval
policy" bound `bash`. A security audit showed the first half was false, and a
false claim about a boundary is worse than no claim.

**Path confinement does not apply to `bash`.** A command reaches any path with `cd`
or with an absolute path. The session root only sets the working directory.

**So the approval policy is the only real boundary.** `bash` declares
`ToolKind::Execute`, which `ToolKind::is_read_only` treats as mutating. A read-only
policy therefore denies `bash` outright. Point rho at a repository you do not trust
with `--read-only`.

**Credential scrubbing is defence in depth, not a boundary.** It removes a key from
the child's environment, so a careless command cannot echo one. It does not stop a
determined command, because a shell can read `~/.aws/credentials` or a shell profile
from disk. Anything that runs shell commands can read files that the user can read.

Sprint 1 adds no container and no namespace sandbox. That is the honest limit.

## 8. Test cases

Security tests for `bash`, each from a real finding:
- `tool_bash_hides_a_credential_from_the_child` — a variable named like a key never
  reaches the child. A live run against a real model demonstrated the leak first.
- `tool_bash_keeps_the_variables_a_command_needs` — `PATH` and `HOME` survive, so
  scrubbing does not break ordinary work.
- `reader_splits_a_line_that_never_ends` — output with no newline cannot grow the
  host's memory without bound.


Decisions this stage pins, with a test each:
- `read` truncates stored output at 100000 bytes and notes the truncation. It
  reads UTF-8 text only; a non-UTF-8 file returns `InvalidArguments`.
- `write` creates parent directories and overwrites an existing file.
- `edit` fails when `old_text` appears more than once. A first-match replacement
  of an ambiguous span is a data-loss bug, so the tool refuses it and leaves the
  file unchanged.
- `bash` enforces its timeout with a `tokio::time::sleep` branch in a `select`,
  and on timeout or cancel it kills the whole process group with
  `kill(-pid, SIGKILL)`, so an orphaned grandchild cannot survive.

In `crates/rho-core/src/tool.rs` unit tests, for `confine` (F-28) and the
policies (F-29):
- `confine_allows_child_path` — a relative path under the root resolves to an
  absolute path.
- `confine_allows_the_root_itself` — an empty candidate resolves to the root.
- `confine_allows_new_file_that_does_not_exist_yet` — a not-yet-existing target
  under the root resolves. This is the `write` case.
- `confine_allows_traversal_that_returns_inside_root` — `sub/../file.txt`
  resolves and stays inside the root.
- `confine_rejects_parent_escape` — a `../` path returns `PathEscape`.
- `confine_rejects_absolute_outside_root` — an absolute path outside the root
  returns `PathEscape`.
- `confine_allows_absolute_inside_root` — an absolute path under the root
  resolves.
- `confine_rejects_symlink_that_points_outside_root` — a symlink inside the root
  that points outside is rejected.
- `confine_allows_symlink_that_points_inside_root` — a symlink that points inside
  the root resolves.
- `confine_handles_temp_dir_realpath_pair` — a path under the macOS temp root
  resolves without a false `PathEscape`.
- `confine_rejects_nul_byte_path` — a path with a `NUL` byte returns
  `InvalidArguments`.
- `confine_errors_when_root_does_not_exist` — a missing root returns `Io`.
- `read_only_policy_allows_a_reading_tool` — `ReadOnlyPolicy` allows a `Read`
  kind.
- `read_only_policy_denies_a_mutating_tool` — `ReadOnlyPolicy` denies `Edit`,
  `Delete`, `Move`, and `Execute`.
- `allow_all_policy_allows_a_mutating_tool` — `AllowAllPolicy` allows an
  `Execute` kind.
- `read_only_policy_denies_an_undeclared_kind` — `ReadOnlyPolicy` denies
  `ToolKind::Other`. This guards the fail-closed direction of the allowlist.
- `every_tool_kind_is_classified_on_purpose` — every `ToolKind` variant is either
  read-only or mutating, and the split matches this spec. A silent flip of one
  variant fails this test.

In `crates/rho-core/tests/approval.rs`, for approval wired into dispatch:
- `approval_denied_mutating_call_never_runs_the_tool` — a `ReadOnlyPolicy`
  denies a mutating tool, the tool never runs, and the result is an error result.
- `approval_allowed_reading_call_runs_the_tool` — a `ReadOnlyPolicy` allows a
  reading tool and the tool runs.

In `crates/rho-tools/tests/`:
- `tool_read_returns_file_contents` — `read` returns the file text.
- `tool_read_offset_limit_slices_lines` — `read` with `offset` and `limit`
  returns the requested line range.
- `tool_write_creates_file_and_parents` — `write` creates a nested file.
- `tool_edit_replaces_unique_span` — `edit` replaces a unique `old_text`.
- `tool_edit_fails_on_absent_span` — `edit` returns `InvalidArguments` when
  `old_text` is absent.
- `tool_edit_fails_on_ambiguous_span` — `edit` fails when `old_text` matches more
  than once.
- `tool_list_returns_entries` — `list` returns directory entries.
- `tool_glob_matches_pattern` — `glob` returns files that match the pattern.
- `tool_grep_finds_matches` — `grep` returns lines that match the regex.
- `tool_grep_honours_gitignore` — `grep` skips a path in `.gitignore`.
- `tool_bash_streams_output_lines` — `bash` sends output lines on `updates`
  before it finishes.
- `tool_bash_enforces_timeout` — a slow command returns `Timeout` after
  `timeout_ms`.
- `tool_bash_cancel_kills_process` — a cancel token kills the command and returns
  `Canceled`.
- `tool_bash_truncates_large_output` — output over the cap is truncated and the
  result notes it.
- `tool_bash_reports_nonzero_exit` — a non-zero exit sets `is_error` and notes
  the exit code.
- `tool_read_missing_file_is_io_error` — a missing file returns `Io`.
- `tool_read_directory_is_invalid_arguments` — a directory path returns
  `InvalidArguments`.
- `tool_read_binary_file_is_invalid_arguments` — a non-UTF-8 file returns
  `InvalidArguments`.
- `tool_read_offset_and_limit_beyond_end_returns_empty` — an offset past the end
  returns empty text.
- `tool_read_truncates_a_very_large_file` — a file over the byte cap is truncated
  and the result notes it.
- `tool_write_overwrites_existing_file` — `write` overwrites an existing file.
- `tool_write_rejects_path_escape` — `write` rejects a `../` path.
- `tool_glob_does_not_follow_symlink_escape` — `glob` does not return a file
  reached through a symlink out of the root.
- `tool_grep_does_not_follow_symlink_escape` — `grep` does not search a file
  reached through a symlink out of the root.
- `tool_result_feeds_back_into_a_provider_turn_end_to_end` — a scripted fake
  provider asks for a real `read` call, the agent loop runs it, and the tool
  result appears in the next provider request. This is the S7 end-to-end proof.
- `path_confine_allows_child_path` — a path under the root resolves.
- `path_confine_rejects_parent_escape` — a `../` path returns `PathEscape`.
- `path_confine_rejects_absolute_outside_root` — an absolute path outside the
  root returns `PathEscape`.
- `path_confine_rejects_symlink_escape` — a symlink that points outside the root
  is rejected.
- `tool_schema_roundtrips_arguments` — each tool's `input_schema` accepts a valid
  argument object and the struct parses it back.
- `tool_kind_matches_acp_category` — each built-in tool reports the expected
  `ToolKind`, for example `bash` reports `Execute` and `read` reports `Read`.
- `tool_kind_serialises_snake_case` — each `ToolKind` value serialises to its ACP
  `snake_case` name, for example `switch_mode`.
- `approval_read_only_policy_allows_read` — `ReadOnlyPolicy` allows `read`.
- `approval_read_only_policy_denies_write` — `ReadOnlyPolicy` denies `write`.
- `approval_denied_call_returns_denied_error` — a denied call returns
  `ToolError::Denied`.

## 9. Out of scope for sprint 1

- A container or OS-namespace sandbox for `bash`. The boundary is path
  confinement and the approval policy.
- A network allow-list for `bash`.
- A schema derive crate. Schemas are written by hand.
- Interactive per-call approval UI. Sprint 1 uses a fixed policy. The TUI adds a
  prompt in a later sprint.
- Binary file handling in `read` and `grep`. Sprint 1 treats files as UTF-8 text.
- A patch or multi-edit tool. Sprint 1 ships single-span `edit` only.
