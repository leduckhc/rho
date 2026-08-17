# SPEC-03 — Tool interface

Status: draft for sprint 1.
Owning crates: `rho-core` (the trait), `rho-tools` (the built-in set).

A tool is a typed unit the model can call. This spec defines the `Tool` trait,
JSON Schema generation, argument validation, the built-in tools, path confinement
to the session root, the approval model, and the `bash` timeout and streaming
rules. Sandbox and approval boundaries are part of the contract, not only the
signatures.

## 1. The trait

```rust
use crate::{CancelToken, ContentBlock};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

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
- Normalise `.` and `..` without touching the filesystem, then check the result
  starts with the root.
- Reject a path that resolves outside the root, including through `..`.
- A symlink that points outside the root is rejected after canonicalisation for a
  path that exists.

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    Allow,
    Deny,
}

#[async_trait]
pub trait ApprovalPolicy: Send + Sync {
    /// Decide whether a tool call may run.
    async fn approve(&self, tool: &str, args: &serde_json::Value) -> ApprovalDecision;
}

/// Allows read-only tools, asks nothing, denies mutating tools.
pub struct ReadOnlyPolicy;

/// Allows every call. For a trusted, non-interactive run.
pub struct AllowAllPolicy;
```

The agent loop consults the policy before it runs a mutating tool. A hook may
also block a call; see `SPEC-04`. The policy runs after the hooks.

## 6. The built-in tool set

All eight ship in `rho-tools`. Each confines its paths to the session root.

- `read` — read a file. Args: `path`, optional `offset`, optional `limit`.
  Returns text. Read-only.
- `write` — write a whole file. Args: `path`, `content`. Creates parent
  directories. Mutating. Needs approval.
- `edit` — replace an exact text span in a file. Args: `path`, `old_text`,
  `new_text`. Fails when `old_text` is absent or not unique. Uses `similar` to
  build a diff for the result. Mutating. Needs approval.
- `list` — list a directory. Args: `path`. Returns entries. Read-only.
- `glob` — match files by a glob pattern under the root. Args: `pattern`. Uses
  `globset`. Read-only.
- `grep` — search file contents by a regex. Args: `pattern`, optional `path`,
  optional `glob`. Uses `ignore` to walk and to honour `.gitignore`. Read-only.
- `bash` — run a shell command. Args: `command`, optional `timeout_ms`. Streams
  output. Mutating. Needs approval. See section 7.

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
- The command runs with the caller's environment. Sprint 1 does not add a
  container or a namespace sandbox. Path confinement and the approval policy are
  the only boundaries for `bash`.

## 8. Test cases

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
- `path_confine_allows_child_path` — a path under the root resolves.
- `path_confine_rejects_parent_escape` — a `../` path returns `PathEscape`.
- `path_confine_rejects_absolute_outside_root` — an absolute path outside the
  root returns `PathEscape`.
- `path_confine_rejects_symlink_escape` — a symlink that points outside the root
  is rejected.
- `tool_schema_roundtrips_arguments` — each tool's `input_schema` accepts a valid
  argument object and the struct parses it back.
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
