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
    Admission, AgentOutcome, AllowAllPolicy, ApprovalPolicy, BothPolicies, Context, Dequeued, Gate,
    HookChain, Provider, RetryLedger, Session, SessionConfig, Tool, ToolContext, ToolError,
    ToolKind, ToolOutput, ToolRegistry, collect_report, narrow_sandbox,
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
    /// Run the child in the background and return at once.
    ///
    /// A foreground spawn blocks until the child finishes, so the parent can never
    /// poll one. With this the parent gets the id straight away and asks
    /// `agent_status` when it wants to know.
    #[serde(default)]
    background: bool,
    /// A name for this child, so later calls can say it instead of an id.
    ///
    /// It never fails the spawn. A refused name is a note in the result, because the
    /// child is already admitted and the work matters more than the label. See
    /// `SPEC-subagent-slots-handles-grace` section 3.2.
    #[serde(default)]
    alias: Option<String>,
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
                },
                "background": {
                    "type": "boolean",
                    "description":
                        "Return at once instead of waiting. Use it for long work you want to \
                         carry on beside. Poll it with agent_status, redirect it with \
                         steer_agent, and stop it with cancel_agent."
                },
                "alias": {
                    "type": "string",
                    "description":
                        "A short name for this child, such as \"auth-audit\", so later calls \
                         can say the name instead of the id. At most 64 characters, no line \
                         breaks. A name already in use is reported and the work still runs."
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
        if !args.background {
            return Ok(run_one_child(
                &self.env,
                &ChildRequest {
                    agent: &args.agent,
                    prompt: &args.prompt,
                    artifacts: &args.artifacts,
                    alias: args.alias.as_deref(),
                },
                &ctx.cancel,
                &ctx.agent_events,
            )
            .await);
        }

        // A background child runs in its own task, so this call returns at once. The
        // reservation lives in that task, so the slot frees when the child ends, and
        // the report is recorded so the parent can still read it afterwards.
        let env = Arc::clone(&self.env);
        let cancel = ctx.cancel.clone();
        let events = ctx.agent_events.clone();
        let name = args.agent.clone();
        // Kept out of the task, because the label belongs to this call's result.
        let alias = args.alias.clone();
        let (id_tx, id_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let started = start_background_child(
                &env,
                &args.agent,
                &args.prompt,
                &args.artifacts,
                &cancel,
                &events,
                id_tx,
            )
            .await;
            if let Err(refusal) = started {
                tracing::warn!("a background subagent could not start: {refusal}");
            }
        });

        // Wait only for the id, never for the work.
        match id_rx.await {
            Ok(Ok(id)) => {
                // The label is set here, not in the task, because the tool result is
                // where the model learns the name it may use.
                let note = label_child(&self.env, id, alias.as_deref());
                let handle = self
                    .env
                    .node
                    .registry()
                    .handle_of(id)
                    .unwrap_or_else(|| name.clone());
                let mut text = format!(
                    "started {} in the background as \"{}\" (id {}). Poll it with \
                     agent_status, redirect it with steer_agent, or stop it with \
                     cancel_agent. Either the name or the id works.",
                    name, handle, id.0
                );
                if let Some(note) = note {
                    text.push_str("\n\n");
                    text.push_str(&note);
                }
                Ok(ToolOutput::text(text))
            }
            // A refusal before the child began, for example a limit.
            Ok(Err(refusal)) => Ok(error_result(refusal)),
            Err(_) => Ok(error_result(
                "the background subagent task ended before it reported an id.".to_string(),
            )),
        }
    }
}

/// One child's work, as the caller asked for it.
///
/// A named struct, not four more parameters. `run_one_child` already took six, and a
/// four-argument `Session::new` once hid a fake model id and an approve-all policy here.
/// See decision D-no-four-argument-session-new.
struct ChildRequest<'a> {
    agent: &'a str,
    prompt: &'a str,
    artifacts: &'a [String],
    /// The name the caller asked for, if any.
    alias: Option<&'a str>,
}

/// Set the caller's alias, and return the note the parent should read.
///
/// `None` means there is nothing to say: either no name was asked for, or it was taken.
/// A refusal is a note and never an error, so the work outlives the label.
fn label_child(env: &Arc<SpawnEnv>, id: rho_core::AgentId, alias: Option<&str>) -> Option<String> {
    let handle = env.node.registry().handle_of(id);
    let alias = alias?;
    match env.node.registry().set_alias(&env.node, id, alias) {
        Ok(()) => Some(format!("[you can call this child \"{alias}\".]")),
        Err(refusal) => Some(format!(
            "[the name \"{}\" was not set: {refusal} Use {} or id {}.]",
            alias.escape_debug(),
            handle
                .map(|name| format!("\"{name}\""))
                .unwrap_or_else(|| "its id".to_string()),
            id.0
        )),
    }
}

/// Run one child to completion and return the text the parent should see.
///
/// Shared by `spawn_agent` and `spawn_agents`, so a fan-out and a single spawn
/// cannot drift apart. Every refusal is a returned result, never an error, so a
/// limit or a dead child lets the parent choose again.
/// Start a child in the background, report its id, then run it to completion.
///
/// It sends the id as soon as the reservation is held, so the caller can return while
/// the work goes on. The report is recorded in the registry, because
/// `ChildSlot::drop` removes the live handle and a parent that polls afterwards would
/// otherwise find nothing.
async fn start_background_child(
    env: &Arc<SpawnEnv>,
    agent: &str,
    prompt: &str,
    artifacts: &[String],
    parent_cancel: &rho_core::CancelToken,
    events: &tokio::sync::mpsc::Sender<rho_core::AgentEvent>,
    id_tx: tokio::sync::oneshot::Sender<Result<rho_core::AgentId, String>>,
) -> Result<(), String> {
    // Check what cannot be undone before reserving anything. A reservation taken and
    // then abandoned would count against the caps for nothing.
    if let Err(refusal) = rho_core::AgentTask::new(agent, prompt).validate() {
        let _ = id_tx.send(Err(refusal.to_string()));
        return Err(refusal.to_string());
    }
    if !env.definitions.contains_key(agent) {
        let refusal = format!("no agent named \"{agent}\" is defined. Check the agent name.");
        let _ = id_tx.send(Err(refusal.clone()));
        return Err(refusal);
    }

    let cancel = parent_cancel.child();
    // A full parent queues, so the id exists before the slot does. The id goes out
    // first either way, because the model polls, steers, and cancels by it while the
    // child waits. See decision D-queue-over-refuse.
    let admitted = match env.node.admit_child(agent, cancel.clone()) {
        Ok(admitted) => admitted,
        Err(refusal) => {
            let _ = id_tx.send(Err(refusal.to_string()));
            return Err(refusal.to_string());
        }
    };
    let (id, waited) = match admitted {
        Admission::Started(spawn) => {
            let id = spawn.node.id();
            let _ = id_tx.send(Ok(id));
            (id, Ok(spawn))
        }
        Admission::Queued(queued) => {
            let id = queued.id();
            let _ = id_tx.send(Ok(id));
            (id, queued.started().await)
        }
    };

    // A child that never started still owes the parent an answer, because the parent
    // holds its id and will poll it. A cancel is a cancel, and a full process is a
    // failure, so the outcome keeps its meaning either way.
    let spawn = match waited {
        Ok(spawn) => spawn,
        Err(dequeued) => {
            env.node.registry().record_report(
                &env.node,
                id,
                unstarted_report(agent, dequeued_outcome(dequeued)),
            );
            return Err(dequeued.to_string());
        }
    };

    // A refusal is recorded as a failed report, so a parent that polls learns why
    // rather than finding nothing. The report is built here, by the one caller that
    // needs one, instead of every refusal inventing a fake one with zero turns.
    let report = match finish_child(env, agent, prompt, artifacts, cancel, events, spawn).await {
        Ok(finished) => finished.report,
        Err(refusal) => unstarted_report(agent, AgentOutcome::Failed { reason: refusal }),
    };
    env.node.registry().record_report(&env.node, id, report);
    Ok(())
}

/// What a child that never started reports as its outcome.
///
/// A pure function, because the two arms mean different things to a parent and only a
/// race can produce the second one. A test cannot drive that race through a tool, so
/// the mapping is tested here instead of nowhere: a review proved that swapping the two
/// arms passed every test. A cancel is what the parent asked for, so it is `Canceled`. A
/// full process is not, so it is `Failed` and it carries the reason.
fn dequeued_outcome(dequeued: Dequeued) -> AgentOutcome {
    match dequeued {
        Dequeued::Cancelled => AgentOutcome::Canceled,
        // The parent did not ask for this, so it is a failure and never a cancel.
        Dequeued::WaitedTooLong { .. } => AgentOutcome::Failed {
            reason: dequeued.to_string(),
        },
        Dequeued::ProcessWideFull { .. } => AgentOutcome::Failed {
            reason: dequeued.to_string(),
        },
    }
}

/// The report of a child that produced nothing: it never started, or it died first.
///
/// One builder, so a refusal and a death cannot describe themselves differently. Zero
/// turns and no transcript are the truth here, not a placeholder.
fn unstarted_report(agent: &str, outcome: AgentOutcome) -> rho_core::AgentReport {
    rho_core::AgentReport {
        agent: agent.to_string(),
        outcome,
        summary: String::new(),
        usage: Default::default(),
        turns: 0,
        gate: Default::default(),
        claims: Default::default(),
        transcript: None,
    }
}

async fn run_one_child(
    env: &Arc<SpawnEnv>,
    request: &ChildRequest<'_>,
    parent_cancel: &rho_core::CancelToken,
    events: &tokio::sync::mpsc::Sender<rho_core::AgentEvent>,
) -> ToolOutput {
    let ChildRequest {
        agent,
        prompt,
        artifacts,
        alias,
    } = *request;
    // A task rho cannot act on is refused first, and the refusal teaches. A goal is
    // required, because the goal is the child's prompt. See `SPEC-agent-tasks`.
    if let Err(refusal) = rho_core::AgentTask::new(agent, prompt).validate() {
        return error_result(refusal.to_string());
    }

    // Find the definition. A missing agent is a result, not a fault.
    // Checked here so a refusal costs no reservation. `finish_child` looks it up again.
    let Some(_def) = env.definitions.get(agent) else {
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

    // Reserve a slot in the tree, or wait for one. A refusal names the limit and what
    // to do. The slot frees when it drops at the end of this call, and the handle goes
    // with it. A failed spawn is a result, so the model can choose again. See
    // decision D-measured-cost-and-cache.
    //
    // A full parent queues. rho does the waiting, not the model, so a fan-out wider
    // than the cap runs every task instead of losing the extra ones to a refusal. The
    // wait ends when a sibling of this child frees its slot, and every sibling is
    // bounded by the child timeout. A line of waiters is bounded by the sum of the
    // timeouts ahead of it, not by one. The `child_timeout` clock starts in
    // `collect_report`, below, so a task that waited still gets its whole budget. See
    // `SPEC-subagent-slots-handles-grace` section 2.8.
    let spawn = match env.node.admit_child(agent, cancel.clone()) {
        Ok(Admission::Started(spawn)) => spawn,
        Ok(Admission::Queued(queued)) => match queued.started().await {
            Ok(spawn) => spawn,
            // Cancelled, or the process-wide cap filled while it waited. Both name
            // themselves, and neither is a fault.
            Err(dequeued) => return error_result(dequeued.to_string()),
        },
        Err(refusal) => return error_result(refusal.to_string()),
    };

    // The label is set once the child holds an id, so a sibling that asked for the same
    // name is told, and the second child still runs.
    let note = label_child(env, spawn.node.id(), alias);

    // A refusal after the reservation is a result for the model, not a fault.
    let mut output = match finish_child(env, agent, prompt, artifacts, cancel, events, spawn).await
    {
        Ok(finished) => finished.output,
        Err(refusal) => error_result(refusal),
    };
    if let Some(note) = note {
        output = with_note(output, &note);
    }
    output
}

/// Append a note to a tool result, keeping whether it was an error.
///
/// A note is rho's own line, so it goes after the child's text and never inside it.
fn with_note(output: ToolOutput, note: &str) -> ToolOutput {
    let mut content = output.content;
    match content.last_mut() {
        Some(rho_core::ContentBlock::Text { text }) => {
            text.push_str("\n\n");
            text.push_str(note);
        }
        _ => content.push(rho_core::ContentBlock::Text {
            text: note.to_string(),
        }),
    }
    ToolOutput {
        content,
        is_error: output.is_error,
    }
}

/// What a finished child produced: the text for the parent, and the report.
///
/// The background path needs the report to record it, and the foreground path needs
/// only the text. One struct, so the two paths cannot drift.
struct ChildOutput {
    output: ToolOutput,
    report: rho_core::AgentReport,
}

/// Run a child that already holds its reservation, and build the parent's result.
///
/// Shared by the foreground and the background path, so a fan-out, a blocking spawn,
/// and a background spawn cannot drift apart.
/// The text the parent reads, and whether it counts as a failure.
///
/// A pure function of the report, so it needs no provider and no session to test. It
/// was eighty lines in the middle of a two-hundred-line procedure, which is why the
/// `is_error` bug for three of five outcomes sat there unseen.
///
/// The retry ledger is **not** consulted here. Counting a death is a side effect and
/// this function has none, so the caller records it.
fn parent_note(
    report: &rho_core::AgentReport,
    dropped: &[String],
    timeout: std::time::Duration,
) -> (String, bool) {
    let mut text = String::new();
    match &report.outcome {
        AgentOutcome::Done => {}
        AgentOutcome::OutOfTurns => text.push_str(&format!(
            "[the {} subagent used all {} of its turns. What follows is what it had.]\n\n",
            report.agent, report.turns
        )),
        AgentOutcome::Rejected { failed } => text.push_str(&format!(
            "[the {} subagent finished, and rho's checks failed: {}. The work is not \
             accepted. Fix it here, or delegate again with a clearer goal.]\n\n",
            report.agent,
            failed.join(", ")
        )),
        AgentOutcome::Canceled => text.push_str(&format!(
            "[the {} subagent was cancelled, most likely by its {} second timeout, after {} \
             turn(s). Do the work here, or delegate a smaller piece.]\n\n",
            report.agent,
            timeout.as_secs(),
            report.turns
        )),
        AgentOutcome::Failed { reason } => text.push_str(&format!(
            "[the {} subagent failed after {} turn(s): {reason} Do the work here, or try a \
             different agent.]\n\n",
            report.agent, report.turns
        )),
    }
    text.push_str(&report.summary);

    // Point the parent at the full transcript, as pi does with its `.output` footer.
    // The summary stays the default and the file is there when the parent wants more.
    if let Some(path) = &report.transcript {
        text.push_str(&format!("\n\n[full transcript: {}]", path.display()));
    }
    if !dropped.is_empty() {
        text.push_str(&format!(
            "\n\n[note: these requested tools were dropped because the parent does not hold \
             them: {}]",
            dropped.join(", ")
        ));
    }
    // One place decides what counts as a failure, and it is the type.
    (text, report.outcome.is_failure())
}

/// Build the child session a definition describes.
///
/// Every confinement rule lives here and nowhere else: the sandbox may only narrow,
/// the tool set is an intersection, the policy is composed with the parent's, and the
/// root is inherited. Keeping them together means a reader checks confinement in one
/// place instead of scanning a two-hundred-line procedure for it.
///
/// The result policy a child inherits.
///
/// The bounds follow the parent, or a caller who tightened `max_result_bytes` would find every
/// child ignoring the tighter value.
///
/// The store never follows. `read_tool_result` is registered per session and a child does not
/// get it, so a store would swallow a tail the child could never read back. A cut that says the
/// tail is gone is better than a handle nobody can use. See `SPEC-tool-result-handle` section 9.
fn child_result_policy(parent: &SessionConfig) -> rho_core::ResultPolicy {
    rho_core::ResultPolicy {
        limits: parent.results.limits,
        preview: Arc::clone(&parent.results.preview),
        store: None,
    }
}

/// It returns the intersection too, because the caller reports a dropped tool name.
async fn build_child(
    env: &Arc<SpawnEnv>,
    def: &AgentDefinition,
    queue: rho_core::MessageQueue,
) -> Result<(Session, rho_core::ToolIntersection), rho_core::SubagentError> {
    // The sandbox may only narrow. A weaker request is refused.
    let sandbox = match narrow_sandbox(env.parent_config.sandbox, def.sandbox) {
        Ok(mode) => mode,
        Err(refusal) => return Err(refusal),
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

    let child_results = child_result_policy(&env.parent_config);
    let child_config = SessionConfig::new(model, env.parent_config.session_root.clone(), approval)
        .with_sandbox(sandbox)
        .with_max_turns(max_turns)
        .with_max_tool_calls(max_tool_calls)
        .with_result_policy(child_results)
        // A definition cannot set this, so a project file cannot turn off a child's
        // warning. No clamp against `max_turns` is applied here: the driver only warns
        // at a boundary after the child has taken a turn, so a window wider than the
        // cap already behaves the same. A clamp here was written first, and a
        // deliberate break proved no test could see it. See `SPEC-subagent-slots-handles-grace`
        // section 4 and AGENTS.md step 6.
        .with_grace_turns(env.node.limits().grace_turns);

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
    .with_queue(queue);
    Ok((child, intersection))
}

/// Verify a finished child's work, and fold the verdict into its report.
///
/// rho verifies. The child never does, and no type here lets it: only a gate builds a
/// `CheckResult`. A failed gate rewrites the outcome to `Rejected`, so a reader that
/// trusts only the outcome still sees the failure. A gate that cannot run has proved
/// nothing, so it fails closed. See `SPEC-agent-tasks` and decision
/// D-a-child-does-not-grade-itself.
async fn verify_work(
    env: &Arc<SpawnEnv>,
    agent: &str,
    prompt: &str,
    artifacts: &[String],
    cancel: rho_core::CancelToken,
    mut report: rho_core::AgentReport,
) -> rho_core::AgentReport {
    if artifacts.is_empty() {
        return report;
    }
    let task = rho_core::AgentTask::new(agent, prompt).with_artifacts(
        artifacts
            .iter()
            .map(|path| rho_core::ArtifactSpec::File { path: path.into() })
            .collect(),
    );
    let ctx = rho_core::GateContext {
        session_root: env.parent_config.session_root.clone(),
        cancel,
        runner: Arc::clone(&env.runner),
    };
    match rho_core::DefaultGate::new().verify(&task, &ctx).await {
        Ok(gate) => {
            if !gate.passed() {
                report.outcome = rho_core::AgentOutcome::Rejected {
                    failed: gate.failed_labels(),
                };
            }
            report.gate = gate;
        }
        Err(refusal) => {
            report.outcome = rho_core::AgentOutcome::Rejected {
                failed: vec![refusal.to_string()],
            };
        }
    }
    report
}

async fn finish_child(
    env: &Arc<SpawnEnv>,
    agent: &str,
    prompt: &str,
    artifacts: &[String],
    cancel: rho_core::CancelToken,
    events: &tokio::sync::mpsc::Sender<rho_core::AgentEvent>,
    spawn: rho_core::ChildSpawn,
) -> Result<ChildOutput, String> {
    let def = env
        .definitions
        .get(agent)
        .expect("the caller checked the definition before it reserved a slot");
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

    // Confinement lives in `build_child`, so this function reads as a sequence.
    let (child, intersection) = match build_child(env, def, spawn.queue()).await {
        Ok(pair) => pair,
        Err(refusal) => return Err(refusal.to_string()),
    };

    // JSONL content, so a JSONL extension. A reader should not have to guess.
    let transcript = env
        .transcript_dir
        .join(format!("{}.jsonl", child_node.id()));
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
    // Hand `collect_report` the progress sender, so a handle sees each turn as the
    // child works. Publishing once at the end made `progress()` read zero for the
    // whole run, which is a post-mortem and not progress.
    let report = collect_report(
        def.name.clone(),
        child_events,
        cancel,
        rho_core::CollectOptions::with_timeout(timeout)
            .transcript(transcript)
            .publishing(spawn.progress_sender()),
    )
    .await;

    let report = verify_work(env, agent, prompt, artifacts, cancel_for_gate, report).await;

    // The last word, so a handle read after the run matches the report exactly.
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
    // A child that did not finish counts as a death for the retry cap. The same work
    // dying again and again must stop, or a poisoned task burns the whole budget. The
    // key is the agent and the work, so a different task starts from zero.
    if matches!(
        report.outcome,
        AgentOutcome::Canceled | AgentOutcome::Failed { .. }
    ) {
        let key = format!("{}\u{1f}{}", agent, work);
        if let Err(capped) = env.retries.record_death(&key) {
            return Err(capped.to_string());
        }
    }

    let (text, failed) = parent_note(&report, &intersection.dropped, timeout);
    let output = if failed {
        error_result(text)
    } else {
        ToolOutput::text(text)
    };
    Ok(ChildOutput { output, report })
}

/// One task in a fan-out.
#[derive(Debug, Deserialize)]
struct FanOutTask {
    /// The name of the agent definition to run.
    agent: String,
    /// The work to delegate to this child.
    prompt: String,
    /// A name for this child. Two tasks that ask for one name give it to the first,
    /// and the second is told. See `SPEC-subagent-slots-handles-grace` section 3.2.
    #[serde(default)]
    alias: Option<String>,
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
                            },
                            "alias": {
                                "type": "string",
                                "description":
                                    "A short name for this child. One name belongs to one \
                                     child: a second task asking for it is told, and it \
                                     still runs."
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
        let runs = args.tasks.iter().map(|task| {
            let env = Arc::clone(&self.env);
            let cancel = ctx.cancel.clone();
            let events = ctx.agent_events.clone();
            async move {
                run_one_child(
                    &env,
                    &ChildRequest {
                        agent: &task.agent,
                        prompt: &task.prompt,
                        artifacts: &[],
                        alias: task.alias.as_deref(),
                    },
                    &cancel,
                    &events,
                )
                .await
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

#[cfg(test)]
mod unstarted_child_tests {
    use super::*;

    /// A child must inherit the parent's result bounds, and must get no store.
    ///
    /// A security review found that `build_child` set no result policy at all, so a caller who
    /// tightened the cap was ignored by every child. Nothing tested a child's cap.
    #[test]
    fn a_child_inherits_the_parent_result_bounds() {
        let tightened = rho_core::ResultLimits {
            max_result_bytes: 4096,
            store_threshold_bytes: 1024,
            ..rho_core::ResultLimits::default()
        };
        let dir = std::env::temp_dir();
        let parent = SessionConfig::new("m", &dir, Arc::new(rho_core::ReadOnlyPolicy))
            .with_result_policy(rho_core::ResultPolicy {
                limits: tightened,
                ..rho_core::ResultPolicy::default()
            });

        let child = child_result_policy(&parent);

        assert_eq!(
            child.limits, tightened,
            "a child that ignored the tighter cap would spend the context the caller saved"
        );
        assert!(
            child.store.is_none(),
            "a child has no read_tool_result tool, so a store would swallow a tail it cannot read"
        );
    }

    #[test]
    fn a_cancelled_waiter_is_cancelled_and_a_full_process_is_a_failure() {
        // A review swapped these two arms and every test still passed, because only a
        // race reaches the second one. The two mean different things: a parent asked for
        // the cancel, and it did not ask for the full process. A parent that reads
        // "cancelled" for a refusal would think its own cancel worked.
        assert!(matches!(
            dequeued_outcome(Dequeued::Cancelled),
            AgentOutcome::Canceled
        ));

        let outcome = dequeued_outcome(Dequeued::ProcessWideFull { limit: 32 });
        match outcome {
            AgentOutcome::Failed { reason } => assert!(
                reason.contains("--max-live-agents"),
                "the reason must name the flag that would raise the limit: {reason}"
            ),
            other => panic!("a full process is a failure, not {other:?}"),
        }
    }

    #[test]
    fn a_waiter_that_ran_out_of_patience_is_a_failure_that_names_the_wait() {
        // The parent did not ask for this, so it is a failure and never a cancel. A
        // parent that read "cancelled" here would think its own cancel worked. See
        // decision D-a-waiter-has-a-deadline.
        let outcome = dequeued_outcome(Dequeued::WaitedTooLong {
            limit: std::time::Duration::from_secs(600),
        });
        match outcome {
            AgentOutcome::Failed { reason } => {
                assert!(
                    reason.contains("--queue-wait-secs"),
                    "the reason must name the flag that raises the deadline: {reason}"
                );
                assert!(
                    reason.contains("600"),
                    "and how long the child waited: {reason}"
                );
            }
            other => panic!("a wait that ran out is a failure, not {other:?}"),
        }
    }

    #[test]
    fn an_unstarted_report_claims_no_work() {
        // Zero turns, no summary, and no transcript are the truth for a child that never
        // ran. A placeholder here would show a parent work that never happened.
        let report = unstarted_report("scout", AgentOutcome::Canceled);
        assert_eq!(report.agent, "scout");
        assert_eq!(report.turns, 0);
        assert!(report.summary.is_empty());
        assert!(report.transcript.is_none());
    }
}
