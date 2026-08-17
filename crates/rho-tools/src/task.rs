//! The `task` and `task_cancel` tools.
//!
//! A background command becomes a task the model can list, probe, or wait on.
//! The read actions live in the `task` tool, which declares `ToolKind::Read`, so
//! a read-only policy allows a probe. The kill action lives in a separate
//! `task_cancel` tool, which declares `ToolKind::Execute`, so a read-only policy
//! denies a kill. A single tool cannot vary its kind per call, so the split is
//! required. See `SPEC-07` section 7 and decision D-012.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rho_core::{
    TaskId, TaskRegistry, TaskSnapshot, Tool, ToolContext, ToolError, ToolKind, ToolOutput,
    WaitUntil,
};
use serde::{Deserialize, Serialize};

use crate::args::parse_args;

/// The default wait budget in milliseconds, when the model gives none.
const DEFAULT_WAIT_BUDGET_MS: u64 = 30_000;

/// Arguments for the `task` tool.
#[derive(Debug, Serialize, Deserialize)]
struct TaskArgs {
    /// One of `list`, `get`, or `wait`.
    action: String,
    /// The task id. Required for `get` and `wait`.
    #[serde(default)]
    id: Option<String>,
    /// The wait budget in milliseconds. Used by `wait`.
    #[serde(default)]
    budget_ms: Option<u64>,
    /// What `wait` wakes on: `finished` or `next_progress`. The default is
    /// `finished`.
    #[serde(default)]
    until: Option<String>,
}

/// Arguments for the `task_cancel` tool.
#[derive(Debug, Serialize, Deserialize)]
struct TaskCancelArgs {
    /// The task id to cancel.
    id: String,
}

/// The read-only task tool. It lists, reads, and waits. It never changes state.
pub struct TaskTool {
    tasks: Arc<TaskRegistry>,
}

impl TaskTool {
    pub fn new(tasks: Arc<TaskRegistry>) -> Self {
        Self { tasks }
    }
}

#[async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }
    fn description(&self) -> &str {
        "Inspect background tasks. Actions: list every task, get one task with \
         its output, or wait until a task finishes or reports progress."
    }
    fn kind(&self) -> ToolKind {
        // Read only. A read-only policy allows a probe.
        ToolKind::Read
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "get", "wait"], "description": "The action to run." },
                "id": { "type": "string", "description": "The task id. Required for get and wait." },
                "budget_ms": { "type": "integer", "minimum": 1, "description": "The wait budget in milliseconds." },
                "until": { "type": "string", "enum": ["finished", "next_progress"], "description": "What wait wakes on. Default finished." }
            },
            "required": ["action"]
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: TaskArgs = parse_args(args)?;
        match args.action.as_str() {
            "list" => {
                let snapshots = self.tasks.list().await;
                Ok(snapshots_output(&snapshots))
            }
            "get" => {
                let id = require_id(args.id)?;
                match self.tasks.get(&id).await {
                    Some(snapshot) => Ok(snapshot_output(&snapshot)),
                    None => Err(unknown_task(&id)),
                }
            }
            "wait" => {
                let id = require_id(args.id)?;
                let budget =
                    Duration::from_millis(args.budget_ms.unwrap_or(DEFAULT_WAIT_BUDGET_MS));
                let until = parse_until(args.until.as_deref())?;
                let snapshot = self
                    .tasks
                    .wait(&id, budget, until)
                    .await
                    .map_err(|error| ToolError::Io(error.to_string()))?;
                Ok(snapshot_output(&snapshot))
            }
            other => Err(ToolError::InvalidArguments(format!(
                "the action {other} is not valid. Use list, get, or wait."
            ))),
        }
    }
}

/// The task cancel tool. It kills a task's process group. Mutating, so a
/// read-only policy denies it.
pub struct TaskCancelTool {
    tasks: Arc<TaskRegistry>,
}

impl TaskCancelTool {
    pub fn new(tasks: Arc<TaskRegistry>) -> Self {
        Self { tasks }
    }
}

#[async_trait]
impl Tool for TaskCancelTool {
    fn name(&self) -> &str {
        "task_cancel"
    }
    fn description(&self) -> &str {
        "Cancel a background task. It kills the task's process group, so no \
         grandchild survives."
    }
    fn kind(&self) -> ToolKind {
        // Mutating. A read-only policy denies it. This is why cancel is its own
        // tool, apart from the read-only task tool.
        ToolKind::Execute
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "The task id to cancel." }
            },
            "required": ["id"]
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: TaskCancelArgs = parse_args(args)?;
        let id = TaskId(args.id);
        self.tasks
            .cancel(&id)
            .await
            .map_err(|error| ToolError::Io(error.to_string()))?;
        Ok(ToolOutput::text(format!("Cancelled task {id}.")))
    }
}

/// Require a task id. Return a helpful error when it is missing.
fn require_id(id: Option<String>) -> Result<TaskId, ToolError> {
    id.map(TaskId).ok_or_else(|| {
        ToolError::InvalidArguments("this action needs a task id. Pass the id field.".to_string())
    })
}

/// Parse the `until` argument into a `WaitUntil`. Default to `Finished`.
fn parse_until(until: Option<&str>) -> Result<WaitUntil, ToolError> {
    match until {
        None | Some("finished") => Ok(WaitUntil::Finished),
        Some("next_progress") => Ok(WaitUntil::NextProgress),
        Some(other) => Err(ToolError::InvalidArguments(format!(
            "the until value {other} is not valid. Use finished or next_progress."
        ))),
    }
}

/// A tool error for an unknown task id.
fn unknown_task(id: &TaskId) -> ToolError {
    ToolError::Io(format!(
        "no task has the id {id}. Run the task list action to see the valid ids."
    ))
}

/// Render one snapshot as a JSON tool result, so the model reads it reliably.
fn snapshot_output(snapshot: &TaskSnapshot) -> ToolOutput {
    let text = serde_json::to_string_pretty(snapshot)
        .unwrap_or_else(|_| "cannot render the task".to_string());
    ToolOutput::text(text)
}

/// Render every snapshot as a JSON array.
fn snapshots_output(snapshots: &[TaskSnapshot]) -> ToolOutput {
    let text = serde_json::to_string_pretty(snapshots)
        .unwrap_or_else(|_| "cannot render the tasks".to_string());
    ToolOutput::text(text)
}
