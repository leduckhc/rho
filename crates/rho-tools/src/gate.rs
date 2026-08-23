//! The gate's command runner, and the task arguments a caller sends.
//!
//! `rho-core` owns the gate but must hold no sandbox and no process, so it takes a
//! [`CommandRunner`](rho_core::CommandRunner) trait object. This module supplies
//! the real one, wrapping the same `build_command` path that `bash` uses. So an
//! acceptance check obeys the parent's sandbox mode.
//!
//! **A check command must come from a trusted author.** See decision
//! D-an-acceptance-check-has-a-trusted-author. The tool schema below accepts a
//! file artifact from the model, and it does **not** accept a command. A command
//! check comes from the agent definition or from a Rust caller.

use std::path::Path;

use async_trait::async_trait;
use rho_core::{Acceptance, AgentTask, ArtifactSpec, CancelToken, CommandRunner, SandboxMode};
use serde::Deserialize;

/// Runs a gate command under the parent's sandbox.
pub struct SandboxedRunner {
    sandbox: SandboxMode,
}

impl SandboxedRunner {
    /// A runner confined the same way the parent's `bash` is.
    pub fn new(sandbox: SandboxMode) -> Self {
        Self { sandbox }
    }
}

#[async_trait]
impl CommandRunner for SandboxedRunner {
    async fn run(&self, command: &str, root: &Path, cancel: &CancelToken) -> std::io::Result<i32> {
        let mut built = crate::bash::build_gate_command(command, root, self.sandbox)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let mut child = built.spawn()?;

        // A gate check must stop with its parent. Otherwise a cancelled run leaves
        // a test suite running.
        tokio::select! {
            () = cancel.cancelled() => {
                let _ = child.start_kill();
                Err(std::io::Error::other("the gate check was cancelled"))
            }
            status = child.wait() => {
                // A signal death has no code. Treat it as a failure, never a pass.
                Ok(status?.code().unwrap_or(-1))
            }
        }
    }
}

/// What the model may declare about a task, in the tool arguments.
///
/// Only a **file** artifact is accepted here. A command is executable, and a model
/// writing its own acceptance command would let a prompt injection choose the
/// command that judges it. See decision D-an-acceptance-check-has-a-trusted-author.
#[derive(Debug, Default, Deserialize)]
pub struct TaskArgs {
    /// Files the child must deliver. Each must exist and hold bytes, under the root.
    #[serde(default)]
    pub artifacts: Vec<String>,
}

impl TaskArgs {
    /// Build the task rho will verify.
    ///
    /// `checks` come from the definition or the caller, never from the model.
    pub fn into_task(self, agent: &str, goal: &str, trusted_checks: Vec<Acceptance>) -> AgentTask {
        AgentTask::new(agent, goal)
            .with_artifacts(
                self.artifacts
                    .into_iter()
                    .map(|path| ArtifactSpec::File { path: path.into() })
                    .collect(),
            )
            .with_acceptance(trusted_checks)
    }
}

/// The JSON Schema fragment for the task fields, shared by both spawn tools.
pub fn task_schema_fields() -> serde_json::Value {
    serde_json::json!({
        "artifacts": {
            "type": "array",
            "items": { "type": "string" },
            "description":
                "Files the child must deliver, relative to the session root. rho checks each \
                 one after the child stops, and reports the task rejected when one is missing \
                 or empty. Declare a file only when the child really must write it."
        }
    })
}
