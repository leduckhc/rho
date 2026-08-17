//! The `spawn_agent` tool.
//!
//! A subagent is another `Session` on the same runtime. This tool spawns one, so
//! the model can delegate work that would otherwise fill its own context. Only a
//! summary comes back; the child's transcript is written to disk and never enters
//! the parent's context. See `docs/specs/SPEC-11-subagents.md`.
//!
//! The security core is enforced here by composition. The child's approval
//! policy is `BothPolicies(parent, child)`, so a child can only be more
//! restrictive. The child's tool set is the parent's set intersected with the
//! definition's list. The session root is inherited and never overridable. The
//! sandbox mode may only narrow. See decision D-036.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use rho_core::AgentNode;
use rho_core::{
    AllowAllPolicy, ApprovalPolicy, BothPolicies, Context, HookChain, Provider, Session,
    SessionConfig, Tool, ToolContext, ToolError, ToolKind, ToolOutput, ToolRegistry,
    collect_report, narrow_sandbox,
};
use rho_skills::{AgentDefinition, load_agent_body};
use serde::Deserialize;

use crate::args::parse_args;

/// Builds a child's tool registry, limited to a set of tool names.
///
/// The child gets only the tools the intersection allows. The factory owns how a
/// tool name maps to a `Tool`, so the tool crate stays the single place that
/// knows the built-in set.
pub trait ChildToolFactory: Send + Sync {
    /// Build a registry that holds only the named tools.
    fn build(&self, allowed: &[String]) -> ToolRegistry;
    /// The parent's advertised tool names. The intersection filters the child's
    /// request against these.
    fn parent_tool_names(&self) -> Vec<String>;
}

/// Everything the `spawn_agent` tool needs to run a child.
pub struct SpawnEnv {
    /// This session's place in the spawn tree.
    pub node: AgentNode,
    /// The agent definitions this session may spawn, keyed by name.
    pub definitions: HashMap<String, AgentDefinition>,
    /// The parent session config. The child inherits the root, and composes the
    /// policy and the sandbox from it.
    pub parent_config: SessionConfig,
    /// The provider the child talks to. Inherited, never changed.
    pub provider: Arc<dyn Provider>,
    /// The hooks the child runs under.
    pub hooks: Arc<HookChain>,
    /// Builds the child's tool registry from the allowed names.
    pub tools: Arc<dyn ChildToolFactory>,
    /// The directory that holds child transcripts. One file per child.
    pub transcript_dir: PathBuf,
}

/// Arguments for the `spawn_agent` tool.
#[derive(Debug, Deserialize)]
struct SpawnArgs {
    /// The name of the agent definition to run.
    agent: String,
    /// The prompt for the child. This is the work to delegate.
    prompt: String,
}

/// The `spawn_agent` tool. It spawns a subagent and returns its summary.
pub struct SpawnAgentTool {
    env: Arc<SpawnEnv>,
}

impl SpawnAgentTool {
    /// Build the tool from its environment.
    pub fn new(env: Arc<SpawnEnv>) -> Self {
        Self { env }
    }
}

#[async_trait]
impl Tool for SpawnAgentTool {
    fn name(&self) -> &str {
        "spawn_agent"
    }
    fn description(&self) -> &str {
        "Delegate a task to a subagent. The child does the work in a fresh \
         conversation and returns only a summary, so the work stays out of your \
         context. Name an agent definition and give it a prompt."
    }
    fn kind(&self) -> ToolKind {
        // Spawning runs code and can change state through the child's tools. It
        // is mutating, so a read-only policy denies it. The child is then
        // confined further by the composed policy.
        ToolKind::Execute
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent": { "type": "string", "description": "The agent definition name." },
                "prompt": { "type": "string", "description": "The work to delegate to the child." }
            },
            "required": ["agent", "prompt"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: SpawnArgs = parse_args(args)?;
        let env = &self.env;

        // Find the definition. A missing agent is a result, not a fault.
        let Some(def) = env.definitions.get(&args.agent) else {
            return Ok(error_result(format!(
                "no agent named \"{}\" is defined. Check the agent name.",
                args.agent
            )));
        };

        // Reserve a slot in the tree. A refusal names the limit and what to do.
        // The slot frees when it drops at the end of this call. A failed spawn is
        // a result, so the model can choose again. See decision D-032.
        let (child_node, _slot) = match env.node.spawn_child() {
            Ok(pair) => pair,
            Err(refusal) => return Ok(error_result(refusal.to_string())),
        };

        // The sandbox may only narrow. A weaker request is refused.
        let sandbox = match narrow_sandbox(env.parent_config.sandbox, def.sandbox) {
            Ok(mode) => mode,
            Err(refusal) => return Ok(error_result(refusal.to_string())),
        };

        // Intersect the child's tool request with the parent's set. A dropped
        // name is reported to the caller, so a bad definition is visible.
        let parent_tools = env.tools.parent_tool_names();
        let intersection = def.resolve_tools(&parent_tools);
        let child_registry = env.tools.build(&intersection.allowed);

        // Compose the policy. The parent is always one conjunct, so the child can
        // only be more restrictive. The definition names no policy, so the child
        // conjunct allows all and the parent policy binds unchanged.
        let approval: Arc<dyn ApprovalPolicy> = Arc::new(BothPolicies::new(
            Arc::clone(&env.parent_config.approval),
            Arc::new(AllowAllPolicy),
        ));

        // The model may be overridden by the definition. The root is inherited
        // and never overridable. The turn cap is capped by the parent's.
        let model = def
            .model
            .clone()
            .unwrap_or_else(|| env.parent_config.model.clone());
        let max_turns = def
            .max_turns
            .map(|requested| requested.min(env.parent_config.max_turns))
            .unwrap_or(env.parent_config.max_turns);

        let child_config =
            SessionConfig::new(model, env.parent_config.session_root.clone(), approval)
                .with_sandbox(sandbox)
                .with_max_turns(max_turns);

        // The body is the child's system prompt. It never enters the parent.
        let body = load_agent_body(def).await.unwrap_or_default();

        let child = Session::with_config(
            child_config,
            Arc::clone(&env.provider),
            Arc::new(child_registry),
            Arc::clone(&env.hooks),
            Context::new(Some(body), Vec::new()),
        );

        // The child runs under the parent's cancel token, so cancelling the
        // parent cancels every descendant. See SPEC-11 section 8.
        let cancel = ctx.cancel.clone();
        let transcript = env.transcript_dir.join(format!("{}.log", child_node.id()));
        let timeout = env.node.limits().child_timeout;

        let events = child.prompt(
            vec![rho_core::ContentBlock::Text { text: args.prompt }],
            cancel.clone(),
        );
        let report =
            collect_report(def.name.clone(), events, cancel, timeout, Some(transcript)).await;

        // The parent's context receives the summary and nothing else. A dropped
        // tool name is added as a short note, so a bad definition is visible.
        let mut text = report.summary;
        if !intersection.dropped.is_empty() {
            text.push_str(&format!(
                "\n\n[note: these requested tools were dropped because the parent does not hold \
                 them: {}]",
                intersection.dropped.join(", ")
            ));
        }
        Ok(ToolOutput::text(text))
    }
}

/// A tool result that carries a plain-text reason for the model. A subagent
/// failure is a result, not the end of the parent's run. See decision D-032.
fn error_result(reason: impl Into<String>) -> ToolOutput {
    ToolOutput {
        content: vec![rho_core::ContentBlock::Text {
            text: reason.into(),
        }],
        is_error: true,
    }
}
