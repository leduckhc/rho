//! Tools that steer and stop a running subagent.
//!
//! A live child is addressable through the registry, so the model can redirect one
//! or stop one without ending the whole run. See `SPEC-subagents` section 7a and
//! `SPEC-steering`.
//!
//! Both tools take an id, and both treat an unknown id as a **result**, not a
//! fault. A child finishing between the model reading the list and acting on it is
//! ordinary, not an error.

use std::sync::Arc;

use async_trait::async_trait;
use rho_core::{AgentId, ContentBlock, Tool, ToolContext, ToolError, ToolKind, ToolOutput};
use serde::Deserialize;

use crate::args::parse_args;
use crate::subagent::SpawnEnv;

#[derive(Debug, Deserialize)]
struct SteerArgs {
    /// The subagent id, from a spawn event or the live list.
    id: u64,
    /// The message the child reads at its next turn boundary.
    message: String,
}

#[derive(Debug, Deserialize)]
struct CancelArgs {
    /// The subagent id to stop.
    id: u64,
}

/// Name the children that are live now, for a refusal that teaches.
fn live_summary(env: &SpawnEnv) -> String {
    // Only this session's own descendants. The registry is process-wide, so `live`
    // would name another session's children. See D-a-caller-addresses-only-its-own.
    let live = env.node.registry().live_under(&env.node);
    if live.is_empty() {
        return "No subagent is running now.".to_string();
    }
    let names: Vec<String> = live
        .iter()
        .map(|handle| format!("{} ({})", handle.id.0, handle.agent))
        .collect();
    format!("Running now: {}.", names.join(", "))
}

/// The `steer_agent` tool. It sends a message to one running subagent.
pub struct SteerAgentTool {
    env: Arc<SpawnEnv>,
}

impl SteerAgentTool {
    pub fn new(env: Arc<SpawnEnv>) -> Self {
        Self { env }
    }
}

#[async_trait]
impl Tool for SteerAgentTool {
    fn name(&self) -> &str {
        "steer_agent"
    }
    fn description(&self) -> &str {
        "Send a message to a subagent that is still running. The child reads it as \
         its next instruction, after its current tool calls finish. Use this to \
         redirect a child instead of cancelling it and starting again."
    }
    fn kind(&self) -> ToolKind {
        // Steering changes what a child does, and a child can change state. So a
        // read-only policy must deny it, exactly as it denies spawning.
        ToolKind::Execute
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "integer",
                    "description": "The subagent id, from the spawn event or the running list."
                },
                "message": {
                    "type": "string",
                    "description": "What the child should do next."
                }
            },
            "required": ["id", "message"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: SteerArgs = parse_args(args)?;
        let Some(handle) = self
            .env
            .node
            .registry()
            .descendant(&self.env.node, AgentId(args.id))
        else {
            return Ok(crate::subagent::error_result(format!(
                "no subagent with id {} is running, so it cannot be steered. It may have \
                 finished already. {}",
                args.id,
                live_summary(&self.env)
            )));
        };
        match handle.steer(vec![ContentBlock::Text { text: args.message }]) {
            Ok(position) => Ok(ToolOutput::text(format!(
                "queued for {} (id {}), at position {position}. The child reads it after \
                 its current tool calls finish.",
                handle.agent, args.id
            ))),
            // A full queue is a typed refusal, never a silent drop.
            Err(full) => Ok(crate::subagent::error_result(full.to_string())),
        }
    }
}

/// The `agent_status` tool. It polls a running or finished subagent.
pub struct AgentStatusTool {
    env: Arc<SpawnEnv>,
}

impl AgentStatusTool {
    pub fn new(env: Arc<SpawnEnv>) -> Self {
        Self { env }
    }
}

#[async_trait]
impl Tool for AgentStatusTool {
    fn name(&self) -> &str {
        "agent_status"
    }

    fn description(&self) -> &str {
        "Poll the status of a running or finished subagent. Returns current progress."
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "integer",
                    "description": "The subagent id from spawn_agent or spawn_agents."
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        #[derive(Deserialize)]
        struct Args {
            id: u64,
        }
        let args: Args = parse_args(args)?;

        // `status` answers for a live child **and** for one that finished. A background
        // child finishes while the parent is busy, and `ChildSlot::drop` removes the
        // live handle, so a lookup that only knew live children would lose every
        // result the parent asked for. jcode keeps a `latest_completion_report` for the
        // same reason.
        let Some(status) = self
            .env
            .node
            .registry()
            .status(&self.env.node, AgentId(args.id))
        else {
            return Ok(ToolOutput::text(format!(
                "no subagent with id {} is known in this session tree. It may belong to \
                 another session, or it finished long enough ago to be forgotten. {}",
                args.id,
                live_summary(&self.env)
            )));
        };

        let text = match status {
            rho_core::AgentStatus::Running {
                agent,
                progress,
                queued,
                ..
            } => {
                let tokens = progress.usage.input_tokens + progress.usage.output_tokens;
                let steering = if queued == 0 {
                    String::new()
                } else {
                    format!(", {queued} steering message(s) waiting")
                };
                format!(
                    "{agent} (id {}) is running: {} turn(s), {tokens} token(s){steering}.",
                    args.id, progress.turns
                )
            }
            // A finished child reports what a blocking spawn would have returned, plus
            // the transcript path so the parent can read the whole run if it wants to.
            rho_core::AgentStatus::Finished { report } => {
                let mut text = format!(
                    "{} (id {}) finished: {}. {} turn(s), {} token(s).",
                    report.agent,
                    args.id,
                    report.outcome.label(),
                    report.turns,
                    report.usage.input_tokens + report.usage.output_tokens
                );
                if !report.gate.passed() {
                    text.push_str(&format!(
                        " rho's checks failed: {}.",
                        report.gate.failed_labels().join(", ")
                    ));
                }
                if !report.summary.is_empty() {
                    text.push_str("\n\n");
                    text.push_str(&report.summary);
                }
                if let Some(path) = &report.transcript {
                    text.push_str(&format!("\n\n[full transcript: {}]", path.display()));
                }
                text
            }
        };

        Ok(ToolOutput::text(text))
    }
}

/// The `cancel_agent` tool. It stops one running subagent.
pub struct CancelAgentTool {
    env: Arc<SpawnEnv>,
}

impl CancelAgentTool {
    pub fn new(env: Arc<SpawnEnv>) -> Self {
        Self { env }
    }
}

#[async_trait]
impl Tool for CancelAgentTool {
    fn name(&self) -> &str {
        "cancel_agent"
    }
    fn description(&self) -> &str {
        "Stop one running subagent. Its siblings keep running, and this session \
         keeps running. Use it when a child is doing the wrong work."
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "integer",
                    "description": "The subagent id to stop."
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: CancelArgs = parse_args(args)?;
        if self
            .env
            .node
            .registry()
            .cancel_descendant(&self.env.node, AgentId(args.id))
        {
            Ok(ToolOutput::text(format!(
                "subagent {} was asked to stop. Its siblings keep running.",
                args.id
            )))
        } else {
            Ok(crate::subagent::error_result(format!(
                "no subagent with id {} is running, so nothing was cancelled. It may have \
                 finished already. {}",
                args.id,
                live_summary(&self.env)
            )))
        }
    }
}

/// Test that agent_status cannot reach another session's child.
#[cfg(test)]
mod agent_status_tests {
    use rho_core::ChildSpawn;

    #[tokio::test]
    async fn agent_status_cannot_reach_another_sessions_child() {
        // Two registries = two sessions. A child in one must not be visible from the other.
        let registry1 = rho_core::AgentRegistry::new(rho_core::SubagentLimits::new());
        let registry2 = rho_core::AgentRegistry::new(rho_core::SubagentLimits::new());

        let node1 = registry1.new_tree();
        let node2 = registry2.new_tree();

        let cancel = rho_core::CancelToken::new();
        let ChildSpawn { node: child, .. } = node1
            .spawn_child("scout", cancel.child())
            .expect("registry 1 has room");

        // Node 2's scope must not see node 1's child.
        let found = node2.registry().descendant(&node2, child.id());
        assert!(
            found.is_none(),
            "agent_status must not see a child from another session"
        );
    }
}
