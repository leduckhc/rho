//! The tool interface.
//!
//! A tool is a typed unit the model can call. The core owns the trait, the
//! registry, path confinement, and the approval model. `rho-tools` ships the
//! built-in set.

use crate::{CancelToken, ContentBlock, ToolSpec};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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
pub fn confine(_root: &Path, _candidate: &Path) -> Result<PathBuf, ToolError> {
    todo!()
}

/// The outcome of an approval check.
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

#[async_trait]
impl ApprovalPolicy for ReadOnlyPolicy {
    async fn approve(&self, _tool: &str, _args: &serde_json::Value) -> ApprovalDecision {
        todo!()
    }
}

#[async_trait]
impl ApprovalPolicy for AllowAllPolicy {
    async fn approve(&self, _tool: &str, _args: &serde_json::Value) -> ApprovalDecision {
        todo!()
    }
}
