//! The `spawn_agent` tool.
//!
//! A subagent is another `Session` on the same runtime. This tool spawns one, so
//! the model can delegate work that would otherwise fill its own context. Only a
//! summary comes back; the child's transcript is written to disk and never enters
//! the parent's context. See `docs/specs/20260818-000223-SPEC-subagents.md`.
//!
//! The security core is enforced here by composition. The child's approval
//! policy is `BothPolicies(parent, child)`, so a child can only be more
//! restrictive. The child's tool set is the parent's set intersected with the
//! definition's list. The session root is inherited and never overridable. The
//! sandbox mode may only narrow. See decision D-child-confined-by-composition.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use rho_core::AgentNode;
use rho_core::{
    AgentOutcome, AllowAllPolicy, ApprovalPolicy, BothPolicies, Context, Gate, HookChain, Provider,
    RetryLedger, Session, SessionConfig, Tool, ToolContext, ToolError, ToolKind, ToolOutput,
    ToolRegistry, collect_report, narrow_sandbox,
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
    /// Runs a gate command under the parent's sandbox. `rho-core` holds no
    /// sandbox, so the tool crate supplies it.
    pub runner: Arc<dyn rho_core::CommandRunner>,
    /// Counts how often the same work has died, so a poisoned task stops.
    ///
    /// jcode's reclaim cap. Without a caller this guard does nothing, and it had
    /// no caller until a live sweep looked for one. See `SPEC-subagents` section 8.
    pub retries: Arc<RetryLedger>,
}

/// Arguments for the `spawn_agent` tool.
#[derive(Debug, Deserialize)]
struct SpawnArgs {
    /// The name of the agent definition to run.
    agent: String,
    /// The prompt for the child. This is the work to delegate.
    prompt: String,
    /// Files the child must deliver. rho checks each one after the child stops.
    ///
    /// Only file names. A command check would let a prompt-injected child choose
    /// the command that judges it. See decision D-an-acceptance-check-has-a-trusted-author.
    #[serde(default)]
    artifacts: Vec<String>,
}

/// The `spawn_agent` tool. It spawns a subagent and returns its summary.
pub struct SpawnAgentTool {
    env: Arc<SpawnEnv>,
    /// The tool description, with every agent and its purpose listed.
    ///
    /// Built once, because `Tool::description` returns a borrow. A definition
    /// requires a `description` field so the model can choose an agent, and the
    /// model can only read it if it reaches the request. See `SPEC-subagents`
    /// section 5.
    description: String,
    /// The agent names, sorted. The schema offers these as an `enum`.
    names: Vec<String>,
}

impl SpawnAgentTool {
    /// Build the tool from its environment.
    pub fn new(env: Arc<SpawnEnv>) -> Self {
        let mut names: Vec<String> = env.definitions.keys().cloned().collect();
        names.sort();

        let mut description = String::from(
            "Delegate a task to a subagent. The child does the work in a fresh \
             conversation and returns only a summary, so the work stays out of your \
             context. Choose one of these agents:",
        );
        for name in &names {
            let purpose = env
                .definitions
                .get(name)
                .map(|def| def.description.as_str())
                .unwrap_or_default();
            description.push_str("\n- ");
            description.push_str(name);
            description.push_str(": ");
            description.push_str(purpose);
        }

        Self {
            env,
            description,
            names,
        }
    }
}

#[async_trait]
impl Tool for SpawnAgentTool {
    fn name(&self) -> &str {
        "spawn_agent"
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn kind(&self) -> ToolKind {
        // Spawning runs code and can change state through the child's tools. It
        // is mutating, so a read-only policy denies it. The child is then
        // confined further by the composed policy.
        ToolKind::Execute
    }
    fn input_schema(&self) -> serde_json::Value {
        // The `enum` is the load-bearing part. Without it the model invents an
        // agent name, and a live run proved it does: it named three agents that
        // did not exist. See `docs/verification/subagents-bedrock.md`.
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "enum": self.names,
                    "description": "The agent definition name. Choose one of the listed values."
                },
                "prompt": { "type": "string", "description": "The work to delegate to the child." },
                "artifacts": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description":
                        "Files the child must deliver, relative to the session root. rho checks \
                         each one after the child stops, and reports the task rejected when one \
                         is missing or empty. Declare a file only when the child must write it."
                }
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
        Ok(run_one_child(
            &self.env,
            &args.agent,
            &args.prompt,
            &args.artifacts,
            &ctx.cancel,
            &ctx.agent_events,
        )
        .await)
    }
}

/// Run one child to completion and return the text the parent should see.
///
/// Shared by `spawn_agent` and `spawn_agents`, so a fan-out and a single spawn
/// cannot drift apart. Every refusal is a returned result, never an error, so a
/// limit or a dead child lets the parent choose again.
async fn run_one_child(
    env: &Arc<SpawnEnv>,
    agent: &str,
    prompt: &str,
    artifacts: &[String],
    parent_cancel: &rho_core::CancelToken,
    events: &tokio::sync::mpsc::Sender<rho_core::AgentEvent>,
) -> ToolOutput {
    // A task rho cannot act on is refused first, and the refusal teaches. A goal is
    // required, because the goal is the child's prompt. See `SPEC-agent-tasks`.
    if let Err(refusal) = rho_core::AgentTask::new(agent, prompt).validate() {
        return error_result(refusal.to_string());
    }

    // Find the definition. A missing agent is a result, not a fault.
    let Some(def) = env.definitions.get(agent) else {
        return error_result(format!(
            "no agent named \"{agent}\" is defined. Check the agent name."
        ));
    };

    // The child runs under a **child** of the parent's token. Cancelling the parent
    // cancels every descendant, and a child that hits its own timeout does not end
    // its parent's run. Sharing one token gave the second behaviour: a live run
    // showed a child timeout killing the whole session silently. See
    // `SPEC-subagents` section 8 and `docs/verification/subagents-bedrock.md`.
    //
    // The token is derived here, before the spawn, because the registry stores it
    // on the handle so a caller can cancel this one child.
    let cancel = parent_cancel.child();

    // Reserve a slot in the tree. A refusal names the limit and what to do.
    // The slot frees when it drops at the end of this call, and the handle goes
    // with it. A failed spawn is a result, so the model can choose again. See
    // decision D-measured-cost-and-cache.
    let spawn = match env.node.spawn_child(agent, cancel.clone()) {
        Ok(spawn) => spawn,
        Err(refusal) => return error_result(refusal.to_string()),
    };
    let child_node = &spawn.node;

    // The frontend learns about the child now, not when it finishes. A send that
    // fails means nobody is listening, which is not an error.
    let _ = events
        .send(rho_core::AgentEvent::AgentSpawned {
            id: child_node.id(),
            agent: agent.to_string(),
            depth: child_node.depth(),
        })
        .await;

    // The sandbox may only narrow. A weaker request is refused.
    let sandbox = match narrow_sandbox(env.parent_config.sandbox, def.sandbox) {
        Ok(mode) => mode,
        Err(refusal) => return error_result(refusal.to_string()),
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

    // The child's tool-call budget is the parent's, capped by the subagent limit.
    // A turn cap counts provider round trips, so it cannot bound a child that makes
    // forty tool calls inside one turn.
    let max_tool_calls = rho_core::cap_tool_calls(
        env.parent_config.max_tool_calls,
        Some(env.node.limits().max_tool_calls),
    );

    let child_config = SessionConfig::new(model, env.parent_config.session_root.clone(), approval)
        .with_sandbox(sandbox)
        .with_max_turns(max_turns)
        .with_max_tool_calls(max_tool_calls);

    // The body is the child's system prompt. It never enters the parent.
    let body = load_agent_body(def).await.unwrap_or_default();

    // The child reads the queue its handle writes to. Without this the handle would
    // push into a queue nobody drains, and every steer would vanish.
    let child = Session::with_config(
        child_config,
        Arc::clone(&env.provider),
        Arc::new(child_registry),
        Arc::clone(&env.hooks),
        Context::new(Some(body), Vec::new()),
    )
    .with_queue(spawn.queue());

    let transcript = env.transcript_dir.join(format!("{}.log", child_node.id()));
    let timeout = env.node.limits().child_timeout;

    // Keep the work text, so a death can be keyed by the work and not only by
    // the agent name. Two different tasks for one agent must not share a count.
    let work = prompt.to_string();
    // Named `child_events` on purpose. The parameter `events` carries this child's
    // progress **up** to the parent's frontend, and this stream carries the child's
    // own turns. Calling both `events` shadowed the parameter, and the finish event
    // was silently never sent.
    let child_events = child.prompt(
        vec![rho_core::ContentBlock::Text {
            text: prompt.to_string(),
        }],
        cancel.clone(),
    );
    // The gate needs a token too, and `collect_report` consumes one.
    let cancel_for_gate = cancel.clone();
    let report = collect_report(
        def.name.clone(),
        child_events,
        cancel,
        timeout,
        Some(transcript),
    )
    .await;

    // rho verifies the work. The child never verifies itself, and no type here
    // lets it: only a gate builds a `CheckResult`. See `SPEC-agent-tasks` and
    // decision D-a-child-does-not-grade-itself.
    let mut report = report;
    if !artifacts.is_empty() {
        let task = rho_core::AgentTask::new(agent, prompt).with_artifacts(
            artifacts
                .iter()
                .map(|path| rho_core::ArtifactSpec::File { path: path.into() })
                .collect(),
        );
        let gate_ctx = rho_core::GateContext {
            session_root: env.parent_config.session_root.clone(),
            cancel: cancel_for_gate,
            runner: Arc::clone(&env.runner),
        };
        match rho_core::DefaultGate::new().verify(&task, &gate_ctx).await {
            Ok(gate) => {
                // A failed gate is never `Done`. A reader that trusts only the
                // outcome must still see the failure.
                if !gate.passed() {
                    report.outcome = rho_core::AgentOutcome::Rejected {
                        failed: gate.failed_labels(),
                    };
                }
                report.gate = gate;
            }
            // A gate that cannot run has proved nothing, so it fails closed.
            Err(refusal) => {
                report.outcome = rho_core::AgentOutcome::Rejected {
                    failed: vec![refusal.to_string()],
                };
            }
        }
    }

    // Publish the final progress, so a handle read after the run sees the truth.
    spawn.publish(rho_core::AgentProgress {
        turns: report.turns,
        usage: report.usage,
    });
    let _ = events
        .send(rho_core::AgentEvent::AgentProgressed {
            id: child_node.id(),
            turns: report.turns,
            usage: report.usage,
        })
        .await;
    let _ = events
        .send(rho_core::AgentEvent::AgentFinished {
            id: child_node.id(),
            report: report.clone(),
        })
        .await;

    // The parent's context receives the summary and nothing else. A dropped
    // tool name is added as a short note, so a bad definition is visible.
    //
    // The outcome is stated whenever it is not plain success. A timed-out or
    // failed child left an empty summary, so the tool returned an empty
    // string and the parent had nothing to act on. A failure must be a
    // result the model can read. See decision D-measured-cost-and-cache.
    let mut text = String::new();
    match &report.outcome {
        AgentOutcome::Done => {}
        AgentOutcome::OutOfTurns => text.push_str(&format!(
            "[the {} subagent used all {} of its turns. What follows is what it had.]\n\n",
            report.agent, report.turns
        )),
        // The gate refused the work. This is not a death, so it does not count
        // toward the retry cap: the child ran and produced something wrong, and a
        // retry with a clearer goal is the right next move.
        AgentOutcome::Rejected { failed } => text.push_str(&format!(
            "[the {} subagent finished, and rho's checks failed: {}. The work is not accepted. \
             Fix it here, or delegate again with a clearer goal.]\n\n",
            report.agent,
            failed.join(", ")
        )),
        // A child that did not finish counts as a death for the retry cap.
        // The same work dying again and again must stop, or a poisoned task
        // burns the whole budget. The key is the agent and the work, so a
        // different task starts from zero.
        AgentOutcome::Canceled | AgentOutcome::Failed { .. } => {
            let key = format!("{}\u{1f}{}", agent, work);
            if let Err(capped) = env.retries.record_death(&key) {
                return error_result(capped.to_string());
            }
            match &report.outcome {
                AgentOutcome::Canceled => text.push_str(&format!(
                    "[the {} subagent was cancelled, most likely by its {} second timeout, \
                         after {} turn(s). Do the work here, or delegate a smaller piece.]\n\n",
                    report.agent,
                    timeout.as_secs(),
                    report.turns
                )),
                AgentOutcome::Failed { reason } => text.push_str(&format!(
                    "[the {} subagent failed after {} turn(s): {reason} Do the work here, or \
                         try a different agent.]\n\n",
                    report.agent, report.turns
                )),
                _ => unreachable!("the outer match limits this arm"),
            }
        }
    }
    text.push_str(&report.summary);
    // A rejected task is a failure. A model that reads only `is_error` must still
    // learn the work was not accepted, so a rejection never looks like a success.
    let rejected = matches!(report.outcome, AgentOutcome::Rejected { .. });
    if !intersection.dropped.is_empty() {
        text.push_str(&format!(
            "\n\n[note: these requested tools were dropped because the parent does not hold \
                 them: {}]",
            intersection.dropped.join(", ")
        ));
    }
    if rejected {
        return error_result(text);
    }
    ToolOutput::text(text)
}

/// One task in a fan-out.
#[derive(Debug, Deserialize)]
struct FanOutTask {
    /// The name of the agent definition to run.
    agent: String,
    /// The work to delegate to this child.
    prompt: String,
}

/// Arguments for the `spawn_agents` tool.
#[derive(Debug, Deserialize)]
struct FanOutArgs {
    tasks: Vec<FanOutTask>,
}

/// The `spawn_agents` tool. It runs several subagents at once.
///
/// A fan-out is one tool call, because `AgentLoop::dispatch` runs tool calls one
/// at a time on purpose. Making dispatch concurrent would race two `edit` calls
/// and two approval prompts. See decision D-fan-out-is-one-tool-call.
pub struct SpawnAgentsTool {
    env: Arc<SpawnEnv>,
    description: String,
    names: Vec<String>,
}

impl SpawnAgentsTool {
    /// Build the tool from its environment.
    pub fn new(env: Arc<SpawnEnv>) -> Self {
        let mut names: Vec<String> = env.definitions.keys().cloned().collect();
        names.sort();

        let mut description = String::from(
            "Delegate several tasks at once. Every child runs at the same time, in a \
             fresh conversation, and only a summary of each comes back. Prefer this \
             over one call per child when the tasks do not depend on each other. \
             Choose from these agents:",
        );
        for name in &names {
            let purpose = env
                .definitions
                .get(name)
                .map(|def| def.description.as_str())
                .unwrap_or_default();
            description.push_str("\n- ");
            description.push_str(name);
            description.push_str(": ");
            description.push_str(purpose);
        }

        Self {
            env,
            description,
            names,
        }
    }
}

#[async_trait]
impl Tool for SpawnAgentsTool {
    fn name(&self) -> &str {
        "spawn_agents"
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn kind(&self) -> ToolKind {
        // Same reasoning as `spawn_agent`. A child can change state, so a
        // read-only policy must deny this.
        ToolKind::Execute
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "minItems": 1,
                    "description": "The tasks to run at the same time. One child per entry.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "agent": {
                                "type": "string",
                                "enum": self.names,
                                "description": "The agent definition name. Choose one of the listed values."
                            },
                            "prompt": {
                                "type": "string",
                                "description": "The work to delegate to this child."
                            }
                        },
                        "required": ["agent", "prompt"]
                    }
                }
            },
            "required": ["tasks"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: FanOutArgs = parse_args(args)?;

        // An empty list is a model mistake. Returning an empty string would be the
        // fail-open shape, so the refusal says what to do instead.
        if args.tasks.is_empty() {
            return Ok(error_result(
                "a fan-out needs at least one task. Give one entry per child, each with an \
                 agent and a prompt.",
            ));
        }

        // Run every task together. `join_all` keeps the results in request order,
        // even though execution is concurrent, so the prompt prefix stays stable
        // and the provider cache stays warm. See decision D-measured-cost-and-cache.
        let runs =
            args.tasks.iter().map(|task| {
                let env = Arc::clone(&self.env);
                let cancel = ctx.cancel.clone();
                let events = ctx.agent_events.clone();
                async move {
                    run_one_child(&env, &task.agent, &task.prompt, &[], &cancel, &events).await
                }
            });
        let results = futures::future::join_all(runs).await;

        // One section per child, named by its agent and its task, so the model can
        // tell which answer belongs to which request.
        let mut text = String::new();
        let mut any_error = false;
        for (task, result) in args.tasks.iter().zip(results.iter()) {
            any_error |= result.is_error;
            text.push_str(&format!("## {} — {}\n", task.agent, task.prompt));
            for block in &result.content {
                if let rho_core::ContentBlock::Text { text: body } = block {
                    text.push_str(body.as_str());
                }
            }
            text.push_str("\n\n");
        }

        // A refused or dead child is a per-task result, never a failed call. One
        // bad task must not lose every good answer.
        Ok(ToolOutput {
            content: vec![rho_core::ContentBlock::Text {
                text: text.trim_end().to_string(),
            }],
            is_error: any_error && results.iter().all(|r| r.is_error),
        })
    }
}

pub(crate) fn error_result(reason: impl Into<String>) -> ToolOutput {
    ToolOutput {
        content: vec![rho_core::ContentBlock::Text {
            text: reason.into(),
        }],
        is_error: true,
    }
}
