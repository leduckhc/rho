//! The built-in tool set for the rho harness.
//!
//! This crate ships `read`, `write`, `edit`, `list`, `glob`, `grep`, and `bash`.
//! Each tool implements the `rho_core::Tool` trait. Each tool declares a real
//! `ToolKind`, because a read-only approval policy denies an undeclared kind.
//! Each tool routes every path through `rho_core::confine`, so a path from the
//! model never escapes the session root.

mod args;
mod bash;
mod edit;
mod glob;
mod grep;
mod list;
mod progress;
mod read;
mod sandbox;
mod subagent;
mod task;
mod write;

pub use bash::BashTool;
pub use edit::EditTool;
pub use glob::GlobTool;
pub use grep::GrepTool;
pub use list::ListTool;
pub use progress::{PROGRESS_PREFIX, ProgressScan, scan_line};
pub use read::ReadTool;
pub use sandbox::{Backend, CommandPlan, SandboxUnavailable, detect_backend, plan};
pub mod gate;
pub use gate::{SandboxedRunner, TaskArgs};
pub use subagent::{ChildToolFactory, SpawnAgentTool, SpawnAgentsTool, SpawnEnv};
pub use task::{TaskCancelTool, TaskTool};
pub use write::WriteTool;

use std::sync::Arc;

use rho_core::{SandboxMode, TaskRegistry, Tool, ToolRegistry};
/// Build a registry with every built-in tool, in a stable order.
///
/// The order is the advertised tool order, so it stays stable across sessions to
/// keep the provider prompt prefix warm. See `SPEC-core-runtime` section 7.
pub fn builtin_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    for tool in builtin_tools() {
        registry.register(tool);
    }
    registry
}

/// The built-in tools as trait objects, in a stable order.
///
/// This set has a foreground-only `bash`. It has no `task` or `task_cancel`
/// tool, because those need a task registry. Use [`builtin_tools_with_tasks`] to
/// get the full set with background support.
pub fn builtin_tools() -> Vec<Arc<dyn Tool>> {
    let mut tools = shared_tools();
    tools.push(Arc::new(BashTool::new()));
    tools
}

/// Every core tool that does not need a task registry.
///
/// One list, because there were two. The six file and search tools were written out in
/// both `builtin_tools` and `builtin_tools_with_tasks`, so adding a core tool to one
/// silently omitted it from the other. A controller found this by deleting `grep` from
/// one list and watching the set-membership test still pass. See decision D-three-tiers.
fn shared_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ReadTool),
        Arc::new(ListTool),
        Arc::new(GlobTool),
        Arc::new(GrepTool),
        Arc::new(WriteTool),
        Arc::new(EditTool),
    ]
}

/// Build a registry with every built-in tool plus background-task support.
///
/// The `bash`, `task`, and `task_cancel` tools all share `tasks`, so a background
/// `bash` call creates a task the `task` tool can probe. See `SPEC-background-tasks` section 7.
pub fn builtin_registry_with_tasks(tasks: Arc<TaskRegistry>) -> ToolRegistry {
    builtin_registry_with_tasks_and_sandbox(tasks, SandboxMode::Off)
}

/// Build a registry with background-task support and a `bash` confinement mode.
///
/// The `bash` tool runs under `sandbox`. `SandboxMode::Off` is today's behaviour.
/// See `SPEC-bash-sandbox`.
pub fn builtin_registry_with_tasks_and_sandbox(
    tasks: Arc<TaskRegistry>,
    sandbox: SandboxMode,
) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    for tool in builtin_tools_with_tasks_and_sandbox(tasks, sandbox) {
        registry.register(tool);
    }
    registry
}

/// The built-in tools with background-task support, in a stable order.
pub fn builtin_tools_with_tasks(tasks: Arc<TaskRegistry>) -> Vec<Arc<dyn Tool>> {
    builtin_tools_with_tasks_and_sandbox(tasks, SandboxMode::Off)
}

/// The built-in tools with background-task support and a `bash` confinement mode.
pub fn builtin_tools_with_tasks_and_sandbox(
    tasks: Arc<TaskRegistry>,
    sandbox: SandboxMode,
) -> Vec<Arc<dyn Tool>> {
    let mut tools = shared_tools();
    tools.push(Arc::new(
        BashTool::with_tasks(Arc::clone(&tasks)).sandbox(sandbox),
    ));
    tools.push(Arc::new(TaskTool::new(Arc::clone(&tasks))));
    tools.push(Arc::new(TaskCancelTool::new(tasks)));
    tools
}
