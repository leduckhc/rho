//! The tool interface.
//!
//! A tool is a typed unit the model can call. The core owns the trait, the
//! registry, path confinement, and the approval model. `rho-tools` ships the
//! built-in set.

use crate::{CancelToken, ContentBlock, ToolSpec};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// The ACP tool category. The values mirror the ACP `ToolKind` set exactly, so
/// `rho-acp` forwards the value with no remap.
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

impl ToolKind {
    /// True when a tool of this kind cannot change state.
    ///
    /// The approval model reads this. A read-only policy allows only a kind that
    /// this function names. This is the single source of truth, and it replaces a
    /// fragile tool-name list.
    ///
    /// The list is an allowlist, not a denylist. That choice makes the boundary
    /// fail closed. `Tool::kind` defaults to `Other`, so a tool author who forgets
    /// to declare a kind gets `Other`. A denylist would then approve that tool,
    /// even when it deletes files. An allowlist denies it instead. A denied safe
    /// tool is an annoyance. An approved destructive tool is a breach.
    ///
    /// So a new `ToolKind` variant is denied until somebody adds it here on
    /// purpose. Do not add a wildcard arm to this match.
    pub fn is_read_only(self) -> bool {
        match self {
            ToolKind::Read
            | ToolKind::Search
            | ToolKind::Think
            | ToolKind::Fetch
            | ToolKind::SwitchMode => true,
            ToolKind::Edit
            | ToolKind::Delete
            | ToolKind::Move
            | ToolKind::Execute
            | ToolKind::Other => false,
        }
    }

    /// True when a tool of this kind may change state, or when its kind is not
    /// declared. See [`ToolKind::is_read_only`] for why an undeclared kind counts
    /// as mutating.
    pub fn is_mutating(self) -> bool {
        !self.is_read_only()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),
    #[error("path {0} escapes the session root")]
    PathEscape(PathBuf),
    #[error(
        "the approval policy denied this tool call. Change the policy to allow it, or call a read-only tool."
    )]
    Denied,
    #[error("{}", timeout_message(*_0))]
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
        Self {
            content: vec![ContentBlock::Text { text: text.into() }],
            is_error: false,
        }
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
    ///
    /// Declare a real kind. A read-only approval policy denies `Other`, because
    /// the policy cannot know whether an undeclared tool changes state. So a tool
    /// that omits this method works only under a permissive policy.
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

/// The tool registry. It holds the tools and advertises their specs.
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

/// Resolve `candidate` under `root`. Return an error when it escapes the root.
///
/// This is the sandbox boundary for feature F-path-confinement. Read it as security code.
///
/// Rules:
/// - Join a relative `candidate` onto the root. Keep an absolute `candidate` as
///   given.
/// - Resolve the longest existing ancestor with the filesystem, so a symlink
///   that points outside the root is caught. You cannot resolve a path that does
///   not exist yet, so the `write` case of a new file still works.
/// - Canonicalise every existing component. This replaces the typed letter case
///   with the on-disk case. So a case-insensitive filesystem, the macOS default,
///   does not falsely reject a valid path that differs only in case.
/// - Compare canonical forms. On macOS `/var` is a symlink to `/private/var`, so
///   a text prefix check fails. Canonicalise the root once, then compare.
/// - An empty `candidate` resolves to the root itself.
/// - A path with a `NUL` byte is rejected. It never reaches the filesystem.
pub fn confine(root: &Path, candidate: &Path) -> Result<PathBuf, ToolError> {
    // A `NUL` byte cannot be part of a real path. Reject it before any syscall.
    if candidate.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(ToolError::InvalidArguments(
            "the path holds a NUL byte. Remove the NUL byte.".to_string(),
        ));
    }

    // Canonicalise the root once. All comparisons use this canonical form.
    let canonical_root = root.canonicalize().map_err(|error| {
        ToolError::Io(format!(
            "cannot resolve the session root {}: {error}. Create the directory first.",
            root.display()
        ))
    })?;

    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        canonical_root.join(candidate)
    };

    let resolved = resolve_existing_ancestor(&joined)?;

    if resolved.starts_with(&canonical_root) {
        Ok(resolved)
    } else {
        Err(ToolError::PathEscape(candidate.to_path_buf()))
    }
}

/// Resolve `path` component by component. Canonicalise each existing part, so a
/// symlink resolves to its real target. Keep a trailing part that does not exist
/// yet, so a new-file target still resolves. The result holds no symlink and no
/// `.` or `..` in its existing prefix.
fn resolve_existing_ancestor(path: &Path) -> Result<PathBuf, ToolError> {
    let mut real = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => real.push(prefix.as_os_str()),
            Component::RootDir => real.push(Component::RootDir.as_os_str()),
            Component::CurDir => {}
            // `real` holds a canonical path with no symlink, so a lexical pop is
            // correct. This keeps `sub/../file.txt` inside the root.
            Component::ParentDir => {
                real.pop();
            }
            Component::Normal(name) => {
                real.push(name);
                // Canonicalise an existing component. This resolves a symlink to
                // its real target, so a link that points outside the root is
                // caught by the later prefix check. It also replaces the typed
                // letter case with the on-disk case. So a case-insensitive
                // filesystem does not cause a false escape. A component that does
                // not exist yet keeps its typed name, so the new-file case works.
                if real.symlink_metadata().is_ok() {
                    real = real.canonicalize().map_err(|error| {
                        ToolError::Io(format!(
                            "cannot resolve the path {}: {error}.",
                            real.display()
                        ))
                    })?;
                }
            }
        }
    }
    Ok(real)
}

/// The outcome of an approval check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    Allow,
    Deny,
}

#[async_trait]
pub trait ApprovalPolicy: Send + Sync {
    /// Decide whether a tool call may run. `kind` states what the tool does, so a
    /// policy decides from the typed category, not from the tool name.
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

#[async_trait]
impl ApprovalPolicy for ReadOnlyPolicy {
    /// Deny a mutating tool. Allow a read-only tool. The decision reads
    /// `ToolKind::is_mutating`, so it never matches a tool name.
    async fn approve(
        &self,
        _tool: &str,
        kind: ToolKind,
        _args: &serde_json::Value,
    ) -> ApprovalDecision {
        if kind.is_mutating() {
            ApprovalDecision::Deny
        } else {
            ApprovalDecision::Allow
        }
    }
}

#[async_trait]
impl ApprovalPolicy for AllowAllPolicy {
    async fn approve(
        &self,
        _tool: &str,
        _kind: ToolKind,
        _args: &serde_json::Value,
    ) -> ApprovalDecision {
        ApprovalDecision::Allow
    }
}

/// Build a timeout message that names the unit.
///
/// Adopted from jcode. A model often passes a millisecond timeout while meaning seconds,
/// then repeats the mistake, because an error that only echoes the number back teaches
/// nothing. So the message states the seconds too, and it warns about the unit when the
/// value is small enough to look like a mistake.
///
/// The message lives here, next to the error, so there is one definition. See decision
/// D-one-redaction-home for why a message that guides a user does not get copied.
fn timeout_message(elapsed: Duration) -> String {
    let millis = elapsed.as_millis();
    let seconds = elapsed.as_secs_f64();
    let mut message = format!("timed out after {millis} ms ({seconds:.1} s)");
    if millis <= 5_000 {
        message.push_str(
            ". Note the unit: the timeout is in milliseconds, not seconds. \
             For a longer limit pass a larger value, for example 600000 for ten minutes",
        );
    }
    message.push('.');
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    // --- Path confinement (F-path-confinement) ---

    #[test]
    fn confine_allows_child_path() {
        let root = tempdir().unwrap();
        let resolved = confine(root.path(), Path::new("file.txt")).unwrap();
        assert!(resolved.is_absolute(), "the result must be absolute");
        assert!(resolved.starts_with(root.path().canonicalize().unwrap()));
    }

    #[test]
    fn confine_allows_the_root_itself() {
        let root = tempdir().unwrap();
        let resolved = confine(root.path(), Path::new("")).unwrap();
        assert_eq!(resolved, root.path().canonicalize().unwrap());
    }

    #[test]
    fn confine_allows_new_file_that_does_not_exist_yet() {
        // This is the real `write` case. The target file has no entry on disk.
        let root = tempdir().unwrap();
        let resolved = confine(root.path(), Path::new("nested/dir/new.txt")).unwrap();
        assert!(resolved.starts_with(root.path().canonicalize().unwrap()));
        assert!(resolved.ends_with("nested/dir/new.txt"));
    }

    #[test]
    fn confine_allows_traversal_that_returns_inside_root() {
        // `sub/../file.txt` stays inside the root. Rejecting it is a false
        // positive. `sub` must exist so the walk resolves it.
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("sub")).unwrap();
        let resolved = confine(root.path(), Path::new("sub/../file.txt")).unwrap();
        assert_eq!(
            resolved,
            root.path().canonicalize().unwrap().join("file.txt")
        );
    }

    #[test]
    fn confine_rejects_parent_escape() {
        let root = tempdir().unwrap();
        let error = confine(root.path(), Path::new("../escape.txt")).unwrap_err();
        assert!(matches!(error, ToolError::PathEscape(_)));
    }

    #[test]
    fn confine_rejects_absolute_outside_root() {
        let root = tempdir().unwrap();
        let error = confine(root.path(), Path::new("/etc/passwd")).unwrap_err();
        assert!(matches!(error, ToolError::PathEscape(_)));
    }

    #[test]
    fn confine_allows_absolute_inside_root() {
        let root = tempdir().unwrap();
        let inside = root.path().canonicalize().unwrap().join("in.txt");
        let resolved = confine(root.path(), &inside).unwrap();
        assert_eq!(resolved, inside);
    }

    #[test]
    fn confine_rejects_symlink_that_points_outside_root() {
        // The case a naive lexical check misses. A link inside the root points
        // at a directory outside the root.
        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        let error = confine(root.path(), Path::new("link/secret.txt")).unwrap_err();
        assert!(matches!(error, ToolError::PathEscape(_)));
    }

    #[test]
    fn confine_allows_symlink_that_points_inside_root() {
        let root = tempdir().unwrap();
        let real_root = root.path().canonicalize().unwrap();
        fs::create_dir(real_root.join("target")).unwrap();
        let link = real_root.join("link");
        std::os::unix::fs::symlink(real_root.join("target"), &link).unwrap();
        let resolved = confine(root.path(), Path::new("link/file.txt")).unwrap();
        assert_eq!(resolved, real_root.join("target").join("file.txt"));
    }

    #[test]
    fn confine_handles_temp_dir_realpath_pair() {
        // On macOS `std::env::temp_dir()` lives under `/var/folders`, and `/var`
        // is a symlink to `/private/var`. A naive prefix check on the uncanonical
        // root fails here. The canonical comparison must not.
        let root = tempdir().unwrap();
        let resolved = confine(root.path(), Path::new("a/b/c.txt")).unwrap();
        assert!(resolved.starts_with(root.path().canonicalize().unwrap()));
    }

    #[test]
    fn confine_allows_a_path_that_differs_only_in_case() {
        // On a case-insensitive filesystem, the macOS default, a valid path that
        // differs only in case must resolve. A case-sensitive `starts_with` on
        // the root prefix falsely rejects it. This is an availability bug, not an
        // escape. The fix canonicalises the existing prefix, so the on-disk case
        // replaces the typed case.
        let parent = tempdir().unwrap();
        let real_parent = parent.path().canonicalize().unwrap();
        let root = real_parent.join("RootDir");
        fs::create_dir(&root).unwrap();

        let lower = real_parent.join("rootdir");
        if !lower.exists() {
            // The filesystem is case-sensitive. The lowercase directory is a
            // different, missing path, so the false-rejection bug cannot occur.
            return;
        }
        // The candidate names the same directory with a different case.
        let candidate = lower.join("file.txt");
        let resolved = confine(&root, &candidate).expect("a case variant of the root must resolve");
        assert!(
            resolved.starts_with(&root),
            "the result stays inside the root"
        );
    }

    #[test]
    fn confine_rejects_nul_byte_path() {
        let root = tempdir().unwrap();
        let error = confine(root.path(), Path::new("bad\0name")).unwrap_err();
        assert!(matches!(error, ToolError::InvalidArguments(_)));
    }

    #[test]
    fn confine_errors_when_root_does_not_exist() {
        let missing = Path::new("/no/such/root/anywhere/xyz");
        let error = confine(missing, Path::new("file.txt")).unwrap_err();
        assert!(matches!(error, ToolError::Io(_)));
    }

    // --- Approval policies (F-tool-approval-gate) ---

    #[tokio::test]
    async fn read_only_policy_allows_a_reading_tool() {
        let policy = ReadOnlyPolicy;
        let decision = policy
            .approve("read", ToolKind::Read, &serde_json::json!({}))
            .await;
        assert_eq!(decision, ApprovalDecision::Allow);
    }

    #[tokio::test]
    async fn read_only_policy_denies_a_mutating_tool() {
        let policy = ReadOnlyPolicy;
        for kind in [
            ToolKind::Edit,
            ToolKind::Delete,
            ToolKind::Move,
            ToolKind::Execute,
        ] {
            let decision = policy.approve("write", kind, &serde_json::json!({})).await;
            assert_eq!(decision, ApprovalDecision::Deny, "{kind:?} must be denied");
        }
    }

    #[tokio::test]
    async fn read_only_policy_denies_an_undeclared_kind() {
        // The boundary must fail closed. `Tool::kind` defaults to `Other`, so a
        // tool author who forgets to declare a kind gets `Other`. If the policy
        // approved `Other`, a tool that deletes files would run under a read-only
        // policy. That is a breach, not an annoyance. So `Other` is denied.
        let policy = ReadOnlyPolicy;
        let decision = policy
            .approve("mystery", ToolKind::Other, &serde_json::json!({}))
            .await;
        assert_eq!(decision, ApprovalDecision::Deny);
    }

    #[test]
    fn every_tool_kind_is_classified_on_purpose() {
        // This test exists so a new `ToolKind` variant cannot slip through as
        // read-only by accident. `is_read_only` matches every variant with no
        // wildcard arm, so a new variant fails to compile until somebody classifies
        // it. This test pins the current classification, so a silent flip of an
        // existing variant fails here.
        let read_only = [
            ToolKind::Read,
            ToolKind::Search,
            ToolKind::Think,
            ToolKind::Fetch,
            ToolKind::SwitchMode,
        ];
        let mutating = [
            ToolKind::Edit,
            ToolKind::Delete,
            ToolKind::Move,
            ToolKind::Execute,
            ToolKind::Other,
        ];
        for kind in read_only {
            assert!(kind.is_read_only(), "{kind:?} must stay read only");
            assert!(!kind.is_mutating(), "{kind:?} must not be mutating");
        }
        for kind in mutating {
            assert!(kind.is_mutating(), "{kind:?} must stay mutating");
            assert!(!kind.is_read_only(), "{kind:?} must not be read only");
        }
        assert_eq!(
            read_only.len() + mutating.len(),
            10,
            "classify every variant"
        );
    }

    #[test]
    fn tool_error_denied_matches_and_reads_as_a_denial() {
        // The denial path constructs this variant. A caller can match on it.
        let error = ToolError::Denied;
        assert!(matches!(error, ToolError::Denied));
        assert!(ToolError::Denied.to_string().contains("denied"));
    }

    #[tokio::test]
    async fn allow_all_policy_allows_a_mutating_tool() {
        let policy = AllowAllPolicy;
        let decision = policy
            .approve("bash", ToolKind::Execute, &serde_json::json!({}))
            .await;
        assert_eq!(decision, ApprovalDecision::Allow);
    }

    #[test]
    fn tool_kind_is_mutating_matches_the_spec() {
        assert!(ToolKind::Edit.is_mutating());
        assert!(ToolKind::Execute.is_mutating());
        assert!(!ToolKind::Read.is_mutating());
        assert!(!ToolKind::Search.is_mutating());
    }
}
