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
    let live = env.node.registry().live();
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
        let Some(handle) = self.env.node.registry().handle(AgentId(args.id)) else {
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
        if self.env.node.registry().cancel(AgentId(args.id)) {
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
