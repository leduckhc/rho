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
mod task;
mod write;

pub use bash::BashTool;
pub use edit::EditTool;
pub use glob::GlobTool;
pub use grep::GrepTool;
pub use list::ListTool;
pub use progress::{PROGRESS_PREFIX, ProgressScan, scan_line};
pub use read::ReadTool;
pub use task::{TaskCancelTool, TaskTool};
pub use write::WriteTool;

use std::sync::Arc;

use rho_core::{TaskRegistry, Tool, ToolRegistry};

/// Build a registry with every built-in tool, in a stable order.
///
/// The order is the advertised tool order, so it stays stable across sessions to
/// keep the provider prompt prefix warm. See `SPEC-01` section 7.
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
    vec![
        Arc::new(ReadTool),
        Arc::new(ListTool),
        Arc::new(GlobTool),
        Arc::new(GrepTool),
        Arc::new(WriteTool),
        Arc::new(EditTool),
        Arc::new(BashTool::new()),
    ]
}

/// Build a registry with every built-in tool plus background-task support.
///
/// The `bash`, `task`, and `task_cancel` tools all share `tasks`, so a background
/// `bash` call creates a task the `task` tool can probe. See `SPEC-07` section 7.
pub fn builtin_registry_with_tasks(tasks: Arc<TaskRegistry>) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    for tool in builtin_tools_with_tasks(tasks) {
        registry.register(tool);
    }
    registry
}

/// The built-in tools with background-task support, in a stable order.
pub fn builtin_tools_with_tasks(tasks: Arc<TaskRegistry>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ReadTool),
        Arc::new(ListTool),
        Arc::new(GlobTool),
        Arc::new(GrepTool),
        Arc::new(WriteTool),
        Arc::new(EditTool),
        Arc::new(BashTool::with_tasks(Arc::clone(&tasks))),
        Arc::new(TaskTool::new(Arc::clone(&tasks))),
        Arc::new(TaskCancelTool::new(tasks)),
    ]
}
