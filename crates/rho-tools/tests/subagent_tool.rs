//! Tests for the `spawn_agent` tool. See `SPEC-subagents` sections 3 and 6.
//!
//! These prove the parent receives only the summary, and that a dropped tool
//! name is reported to the caller.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream;
use rho_core::{
    AgentRegistry, AllowAllPolicy, CancelToken, CompletionRequest, ContentBlock, HookChain,
    Provider, ProviderError, ProviderStream, Role, SessionConfig, StopReason, StreamEvent,
    SubagentLimits, Tool, ToolContext, ToolRegistry,
};
use rho_skills::{AgentDefinition, SkillOrigin};
use rho_tools::{ChildToolFactory, SpawnAgentTool, SpawnAgentsTool, SpawnEnv};

/// A provider that replays one scripted turn.
struct ScriptedProvider {
    turns: Mutex<VecDeque<Vec<StreamEvent>>>,
}

impl ScriptedProvider {
    fn new(turns: Vec<Vec<StreamEvent>>) -> Self {
        Self {
            turns: Mutex::new(turns.into()),
        }
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn id(&self) -> &str {
        "scripted"
    }
    async fn stream(
        &self,
        _request: CompletionRequest,
        _cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError> {
        let events = self.turns.lock().unwrap().pop_front().unwrap_or_default();
        Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
    }
}

/// A provider that never yields an event, so a child reaches its timeout.
///
/// A test needs this to cover the timeout path. An empty script fails fast
/// instead, which is a different branch, and a test that used it passed while the
/// cancel bug was live. See decision D-two-weak-tests.
struct HangingProvider;

#[async_trait]
impl Provider for HangingProvider {
    fn id(&self) -> &str {
        "hanging"
    }
    async fn stream(
        &self,
        _request: CompletionRequest,
        _cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError> {
        Ok(Box::pin(stream::pending()))
    }
}

/// A text turn that answers and ends the run.
fn text_turn(text: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageStart {
            role: Role::Assistant,
        },
        StreamEvent::TextStart { index: 0 },
        StreamEvent::TextDelta {
            index: 0,
            delta: text.to_string(),
        },
        StreamEvent::TextEnd { index: 0 },
        StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        },
    ]
}

/// A factory that builds an empty child registry and reports the parent's tools.
struct FakeToolFactory {
    parent: Vec<String>,
}

impl ChildToolFactory for FakeToolFactory {
    fn build(&self, _allowed: &[String]) -> ToolRegistry {
        ToolRegistry::new()
    }
    fn parent_tool_names(&self) -> Vec<String> {
        self.parent.clone()
    }
}

/// Write a definition file so the body loads, and return its `AgentDefinition`.
fn definition(dir: &std::path::Path, tools: Option<Vec<String>>) -> AgentDefinition {
    let path = dir.join("scout.md");
    std::fs::write(
        &path,
        "---\nname: scout\ndescription: recon.\n---\nYou locate code.\n",
    )
    .unwrap();
    AgentDefinition {
        name: "scout".to_string(),
        description: "recon.".to_string(),
        path,
        origin: SkillOrigin::User,
        tools,
        model: None,
        max_turns: Some(4),
        sandbox: None,
        warnings: Vec::new(),
    }
}

fn spawn_env(
    dir: &std::path::Path,
    turns: Vec<Vec<StreamEvent>>,
    tools: Option<Vec<String>>,
    parent_tools: Vec<String>,
) -> Arc<SpawnEnv> {
    let registry = AgentRegistry::new(SubagentLimits::new());
    let def = definition(dir, tools);
    let mut definitions = HashMap::new();
    definitions.insert(def.name.clone(), def);
    Arc::new(SpawnEnv {
        node: registry.root(),
        definitions,
        parent_config: SessionConfig::new(
            "parent-model",
            dir.to_path_buf(),
            Arc::new(AllowAllPolicy),
        ),
        provider: Arc::new(ScriptedProvider::new(turns)),
        hooks: Arc::new(HookChain::new()),
        tools: Arc::new(FakeToolFactory {
            parent: parent_tools,
        }),
        transcript_dir: dir.to_path_buf(),
        runner: Arc::new(rho_tools::SandboxedRunner::new(rho_core::SandboxMode::Off)),
        retries: Arc::new(rho_core::RetryLedger::new()),
    })
}

/// An env whose child hangs and whose timeout is short.
fn hanging_env(dir: &std::path::Path) -> Arc<SpawnEnv> {
    let limits = SubagentLimits {
        child_timeout: Duration::from_millis(50),
        ..SubagentLimits::new()
    };
    let registry = AgentRegistry::new(limits);
    let def = definition(dir, Some(vec!["read".to_string()]));
    let mut definitions = HashMap::new();
    definitions.insert(def.name.clone(), def);
    Arc::new(SpawnEnv {
        node: registry.root(),
        definitions,
        parent_config: SessionConfig::new(
            "parent-model",
            dir.to_path_buf(),
            Arc::new(AllowAllPolicy),
        ),
        provider: Arc::new(HangingProvider),
        hooks: Arc::new(HookChain::new()),
        tools: Arc::new(FakeToolFactory {
            parent: vec!["read".to_string()],
        }),
        transcript_dir: dir.to_path_buf(),
        runner: Arc::new(rho_tools::SandboxedRunner::new(rho_core::SandboxMode::Off)),
        retries: Arc::new(rho_core::RetryLedger::new()),
    })
}

fn ctx(root: PathBuf) -> ToolContext {
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(16);
    ToolContext {
        session_root: root,
        cancel: CancelToken::new(),
        updates: tx,
        agent_events: agent_tx,
    }
}

#[tokio::test]
async fn a_parent_receives_only_the_child_summary() {
    let dir = tempfile::tempdir().unwrap();
    let env = spawn_env(
        dir.path(),
        vec![text_turn("the bug is at parser.rs:42")],
        None,
        vec!["read".to_string()],
    );
    let tool = SpawnAgentTool::new(env);
    let output = tool
        .execute(
            serde_json::json!({ "agent": "scout", "prompt": "find the bug" }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .unwrap();

    let text = match &output.content[0] {
        ContentBlock::Text { text } => text.clone(),
        other => panic!("expected text, got {other:?}"),
    };
    // Not an exact match: the result now also points at the transcript, on purpose.
    // The load-bearing claim is that the child's intermediate work never appears.
    assert!(text.contains("the bug is at parser.rs:42"), "got: {text}");
    assert!(!output.is_error);
}

#[tokio::test]
async fn a_dropped_tool_name_is_reported_to_the_caller() {
    let dir = tempfile::tempdir().unwrap();
    // The definition asks for `write`, which the parent (read only) does not
    // hold. The drop must be reported in the tool result.
    let env = spawn_env(
        dir.path(),
        vec![text_turn("done")],
        Some(vec!["read".to_string(), "write".to_string()]),
        vec!["read".to_string()],
    );
    let tool = SpawnAgentTool::new(env);
    let output = tool
        .execute(
            serde_json::json!({ "agent": "scout", "prompt": "work" }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .unwrap();
    let text = match &output.content[0] {
        ContentBlock::Text { text } => text.clone(),
        other => panic!("expected text, got {other:?}"),
    };
    assert!(text.contains("done"), "the summary is present: {text}");
    assert!(text.contains("write"), "the dropped tool is named: {text}");
}

#[tokio::test]
async fn an_unknown_agent_name_is_a_result_not_a_fault() {
    let dir = tempfile::tempdir().unwrap();
    let env = spawn_env(
        dir.path(),
        vec![text_turn("x")],
        None,
        vec!["read".to_string()],
    );
    let tool = SpawnAgentTool::new(env);
    let output = tool
        .execute(
            serde_json::json!({ "agent": "ghost", "prompt": "work" }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .unwrap();
    assert!(output.is_error, "a missing agent is an error result");
    let text = match &output.content[0] {
        ContentBlock::Text { text } => text.clone(),
        other => panic!("expected text, got {other:?}"),
    };
    assert!(text.contains("ghost"), "{text}");
}

#[tokio::test]
async fn the_schema_offers_only_the_agents_that_loaded() {
    // A live Bedrock run showed the model inventing three agent names, because
    // the schema took a free-text string and named no choice. The `enum` is what
    // stops the guess. See `docs/verification/subagents-bedrock.md`.
    let dir = tempfile::tempdir().unwrap();
    let env = spawn_env(
        dir.path(),
        vec![text_turn("ok")],
        Some(vec!["read".to_string()]),
        vec!["read".to_string()],
    );
    let tool = SpawnAgentTool::new(env);

    let schema = tool.input_schema();
    let choices = schema["properties"]["agent"]["enum"]
        .as_array()
        .expect("the agent property must offer an enum of the loaded names");
    assert_eq!(
        choices,
        &vec![serde_json::json!("scout")],
        "the enum must hold exactly the loaded agent names"
    );
}

#[tokio::test]
async fn the_description_carries_each_agent_purpose() {
    // `SPEC-subagents` section 5 requires a `description` because "the model
    // reads this to choose". It only reads it if the description reaches the
    // request, and it did not before this test.
    let dir = tempfile::tempdir().unwrap();
    let env = spawn_env(
        dir.path(),
        vec![text_turn("ok")],
        Some(vec!["read".to_string()]),
        vec!["read".to_string()],
    );
    let tool = SpawnAgentTool::new(env);

    let described = tool.description();
    assert!(
        described.contains("scout"),
        "the description must name the agent, got: {described}"
    );
    assert!(
        described.contains("recon."),
        "the description must carry the definition's own description, got: {described}"
    );
}

#[tokio::test]
async fn a_timed_out_child_reports_its_outcome_and_leaves_the_parent_live() {
    // A live run showed the worst defect of the sweep: the child shared the
    // parent's cancel token, so a child timeout cancelled the parent session and
    // the run ended with exit 0 and no answer. The tool also dropped
    // `report.outcome`, so a timed-out child returned an empty string.
    // See docs/verification/subagents-bedrock.md.
    let dir = tempfile::tempdir().unwrap();
    let tool = SpawnAgentTool::new(hanging_env(dir.path()));

    let parent_cancel = rho_core::CancelToken::new();
    let mut context = ctx(dir.path().to_path_buf());
    context.cancel = parent_cancel.clone();

    let output = tool
        .execute(
            serde_json::json!({ "agent": "scout", "prompt": "do the work" }),
            context,
        )
        .await
        .expect("a child failure is a result, not a fault");

    let text = match &output.content[0] {
        rho_core::ContentBlock::Text { text } => text.clone(),
        other => panic!("expected text, got {other:?}"),
    };
    assert!(
        !text.trim().is_empty(),
        "a failed child must return something the model can act on, got an empty string"
    );
    assert!(
        text.contains("scout"),
        "the note must name the agent, got: {text}"
    );
    assert!(
        !parent_cancel.is_cancelled(),
        "a child ending must never cancel the parent session"
    );
}

#[tokio::test]
async fn a_repeatedly_dying_child_is_refused_at_the_retry_cap() {
    // `RetryLedger` existed, was exported, and was unit-tested, and **nothing
    // used it**. So a poisoned task could be re-delegated forever and burn the
    // budget. That is the same gap the module comment in `rho-cli` warns about:
    // a guard that no caller reaches is not shipped. See `SPEC-subagents`
    // section 8 and docs/verification/subagents-bedrock.md.
    let dir = tempfile::tempdir().unwrap();
    let tool = SpawnAgentTool::new(hanging_env(dir.path()));
    let args = serde_json::json!({ "agent": "scout", "prompt": "the poisoned task" });

    let mut refusal = None;
    for attempt in 1..=rho_core::MAX_CHILD_RETRIES {
        let output = tool
            .execute(args.clone(), ctx(dir.path().to_path_buf()))
            .await
            .expect("a dying child is a result, not a fault");
        let text = match &output.content[0] {
            rho_core::ContentBlock::Text { text } => text.clone(),
            other => panic!("expected text, got {other:?}"),
        };
        if text.contains("retry") {
            refusal = Some((attempt, text));
            break;
        }
    }

    let (attempt, text) = refusal.expect("the same dying work must be refused at the cap");
    assert!(
        attempt <= rho_core::MAX_CHILD_RETRIES,
        "the refusal must arrive by attempt {}, arrived at {attempt}",
        rho_core::MAX_CHILD_RETRIES
    );
    assert!(
        text.contains("retry"),
        "the refusal must name the retry cap, got: {text}"
    );
}

/// An env with several definitions and a per-parent cap, for the fan-out tests.
fn fanout_env(dir: &std::path::Path, max_children: usize) -> Arc<SpawnEnv> {
    let limits = SubagentLimits {
        max_children_per_parent: max_children,
        ..SubagentLimits::new()
    };
    let registry = AgentRegistry::new(limits);
    let def = definition(dir, Some(vec!["read".to_string()]));
    let mut definitions = HashMap::new();
    definitions.insert(def.name.clone(), def);
    Arc::new(SpawnEnv {
        node: registry.root(),
        definitions,
        parent_config: SessionConfig::new(
            "parent-model",
            dir.to_path_buf(),
            Arc::new(AllowAllPolicy),
        ),
        // One turn per child. Every child answers with the same text.
        provider: Arc::new(ScriptedProvider::new(vec![
            text_turn("child answer"),
            text_turn("child answer"),
            text_turn("child answer"),
            text_turn("child answer"),
            text_turn("child answer"),
            text_turn("child answer"),
        ])),
        hooks: Arc::new(HookChain::new()),
        tools: Arc::new(FakeToolFactory {
            parent: vec!["read".to_string()],
        }),
        transcript_dir: dir.to_path_buf(),
        runner: Arc::new(rho_tools::SandboxedRunner::new(rho_core::SandboxMode::Off)),
        retries: Arc::new(rho_core::RetryLedger::new()),
    })
}

fn output_text(output: &rho_core::ToolOutput) -> String {
    match &output.content[0] {
        rho_core::ContentBlock::Text { text } => text.clone(),
        other => panic!("expected text, got {other:?}"),
    }
}

#[tokio::test]
async fn a_fan_out_runs_every_task_and_reports_in_request_order() {
    // A fan-out is one tool call, because concurrent general dispatch would race
    // two `edit` calls and two approval prompts. See D-fan-out-is-one-tool-call.
    let dir = tempfile::tempdir().unwrap();
    let tool = SpawnAgentsTool::new(fanout_env(dir.path(), 4));

    let output = tool
        .execute(
            serde_json::json!({
                "tasks": [
                    { "agent": "scout", "prompt": "first" },
                    { "agent": "scout", "prompt": "second" },
                    { "agent": "scout", "prompt": "third" }
                ]
            }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .expect("a fan-out is a result, not a fault");

    let text = output_text(&output);
    let first = text.find("first").expect("task 1 must be reported");
    let second = text.find("second").expect("task 2 must be reported");
    let third = text.find("third").expect("task 3 must be reported");
    assert!(
        first < second && second < third,
        "results must appear in request order, so the prompt prefix stays stable: {text}"
    );
}

#[tokio::test]
async fn a_task_over_the_per_parent_cap_is_refused_and_the_others_still_run() {
    // The cap must bite per task, never per call. A limit that fails the whole
    // fan-out would make one bad task lose every good result.
    let dir = tempfile::tempdir().unwrap();
    let tool = SpawnAgentsTool::new(fanout_env(dir.path(), 2));

    let output = tool
        .execute(
            serde_json::json!({
                "tasks": [
                    { "agent": "scout", "prompt": "alpha" },
                    { "agent": "scout", "prompt": "beta" },
                    { "agent": "scout", "prompt": "gamma" },
                    { "agent": "scout", "prompt": "delta" }
                ]
            }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .expect("a refused task is a result, not a fault");

    let text = output_text(&output);
    assert!(
        text.contains("per-parent child limit is 2"),
        "a refusal must name the limit it hit, got: {text}"
    );
    assert!(
        text.contains("child answer"),
        "the tasks that fit must still report their work, got: {text}"
    );
}

#[tokio::test]
async fn an_empty_fan_out_is_refused_with_a_named_reason() {
    // An empty list is a model mistake. It must teach, not run zero children and
    // return an empty string, which is the fail-open shape.
    let dir = tempfile::tempdir().unwrap();
    let tool = SpawnAgentsTool::new(fanout_env(dir.path(), 4));

    let output = tool
        .execute(
            serde_json::json!({ "tasks": [] }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .expect("an empty list is a result, not a fault");

    let text = output_text(&output);
    assert!(
        !text.trim().is_empty(),
        "an empty fan-out must say what went wrong"
    );
    assert!(
        text.contains("at least one"),
        "the refusal must say what to do instead, got: {text}"
    );
}

#[tokio::test]
async fn a_spawn_emits_the_agent_events_into_the_parent_stream() {
    // `AgentSpawned`, `AgentProgressed`, and `AgentFinished` were defined in
    // `rho-core` and consumed by `rho-tui`, and **nothing emitted them**. So the
    // TUI held a renderer for events that never arrived. That is
    // D-a-panel-nobody-can-open, and `SPEC-subagents` section 9 admitted the
    // wiring was missing.
    let dir = tempfile::tempdir().unwrap();
    let env = spawn_env(
        dir.path(),
        vec![text_turn("the child answer")],
        Some(vec!["read".to_string()]),
        vec!["read".to_string()],
    );
    let tool = SpawnAgentTool::new(env);

    let (events_tx, mut events_rx) = tokio::sync::mpsc::channel(64);
    let mut context = ctx(dir.path().to_path_buf());
    context.agent_events = events_tx;

    tool.execute(
        serde_json::json!({ "agent": "scout", "prompt": "do the work" }),
        context,
    )
    .await
    .unwrap();

    let mut seen = Vec::new();
    while let Ok(event) = events_rx.try_recv() {
        seen.push(event);
    }

    assert!(
        matches!(seen.first(), Some(rho_core::AgentEvent::AgentSpawned { agent, depth, .. }) if agent == "scout" && *depth == 1),
        "the first event must be AgentSpawned, naming the agent and its depth, got: {seen:?}"
    );
    assert!(
        matches!(seen.last(), Some(rho_core::AgentEvent::AgentFinished { report, .. }) if report.agent == "scout"),
        "the last event must be AgentFinished, carrying the report, got: {seen:?}"
    );
    // A frontend must be able to pair every spawn with its finish. Assert the
    // pairing, not one expected event. See AGENTS.md step 12.
    let spawned: Vec<_> = seen
        .iter()
        .filter(|e| matches!(e, rho_core::AgentEvent::AgentSpawned { .. }))
        .collect();
    let finished: Vec<_> = seen
        .iter()
        .filter(|e| matches!(e, rho_core::AgentEvent::AgentFinished { .. }))
        .collect();
    assert_eq!(
        spawned.len(),
        finished.len(),
        "every spawn must have exactly one finish, got: {seen:?}"
    );
}

// --- The gate has a caller (SPEC-agent-tasks) ---

#[tokio::test]
async fn a_child_that_skips_its_declared_artifact_is_rejected() {
    // The gate existed in `rho-core` with no caller, which is the "an unreachable
    // guard is not shipped" trap this project has hit three times. This is the
    // caller. See SPEC-agent-tasks and D-a-child-does-not-grade-itself.
    let dir = tempfile::tempdir().unwrap();
    let env = spawn_env(
        dir.path(),
        // The child claims success and writes nothing.
        vec![text_turn("I wrote the report, it is excellent")],
        Some(vec!["read".to_string()]),
        vec!["read".to_string()],
    );
    let tool = SpawnAgentTool::new(env);

    let output = tool
        .execute(
            serde_json::json!({
                "agent": "scout",
                "prompt": "write report.md",
                "artifacts": ["report.md"]
            }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .expect("a rejected task is a result, not a fault");

    let text = output_text(&output);
    assert!(
        text.contains("checks failed") || text.contains("not accepted"),
        "the parent must be told the work was rejected, got: {text}"
    );
    assert!(
        text.contains("report.md"),
        "the refusal must name the missing artifact, got: {text}"
    );
    assert!(
        !text.contains("excellent") || text.contains("checks failed"),
        "a child's own praise must never stand in for a verdict, got: {text}"
    );
}

#[tokio::test]
async fn a_child_that_delivers_its_artifact_passes_the_gate() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("report.md"), "the findings").unwrap();
    let env = spawn_env(
        dir.path(),
        vec![text_turn("done")],
        Some(vec!["read".to_string()]),
        vec!["read".to_string()],
    );
    let tool = SpawnAgentTool::new(env);

    let output = tool
        .execute(
            serde_json::json!({
                "agent": "scout",
                "prompt": "write report.md",
                "artifacts": ["report.md"]
            }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .unwrap();

    let text = output_text(&output);
    assert!(
        !text.contains("checks failed"),
        "a delivered artifact must pass, got: {text}"
    );
    assert!(text.contains("done"), "the summary must reach the parent");
}

#[tokio::test]
async fn a_task_with_no_declared_artifact_behaves_as_before() {
    // The gate must not change the common case. A spawn with no artifacts is the
    // old behaviour exactly.
    let dir = tempfile::tempdir().unwrap();
    let env = spawn_env(
        dir.path(),
        vec![text_turn("the bug is at parser.rs:42")],
        Some(vec!["read".to_string()]),
        vec!["read".to_string()],
    );
    let tool = SpawnAgentTool::new(env);

    let output = tool
        .execute(
            serde_json::json!({ "agent": "scout", "prompt": "find the bug" }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .unwrap();

    let text = output_text(&output);
    assert!(
        text.contains("the bug is at parser.rs:42"),
        "the summary must reach the parent, got: {text}"
    );
}

#[tokio::test]
async fn the_model_cannot_declare_a_command_check() {
    // A model-authored gate command is an injection surface: a prompt-injected
    // child could choose the command that judges it. The schema takes file names
    // only, so an extra field is ignored rather than executed. See decision
    // D-an-acceptance-check-has-a-trusted-author.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("report.md"), "ok").unwrap();
    let marker = dir.path().join("PWNED");
    let env = spawn_env(
        dir.path(),
        vec![text_turn("done")],
        Some(vec!["read".to_string()]),
        vec!["read".to_string()],
    );
    let tool = SpawnAgentTool::new(env);

    let output = tool
        .execute(
            serde_json::json!({
                "agent": "scout",
                "prompt": "write report.md",
                "artifacts": ["report.md"],
                "acceptance": [{ "label": "pwn", "check": {
                    "kind": "command",
                    "run": format!("touch {}", marker.display())
                }}]
            }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .unwrap();

    assert!(
        !marker.exists(),
        "a model-supplied command must never run as a gate check"
    );
    let _ = output;
}

// --- Steering and cancelling a live child from a tool ---

#[tokio::test]
async fn steer_agent_refuses_an_unknown_id_and_says_which_are_live() {
    // A child that already finished is the common case, and it is a result, not a
    // fault. The refusal must teach: it names the live children so the model can
    // choose again.
    let dir = tempfile::tempdir().unwrap();
    let env = spawn_env(
        dir.path(),
        vec![text_turn("done")],
        Some(vec!["read".to_string()]),
        vec!["read".to_string()],
    );
    let tool = rho_tools::SteerAgentTool::new(Arc::clone(&env));

    let output = tool
        .execute(
            serde_json::json!({ "id": 999, "message": "look elsewhere" }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .expect("an unknown id is a result, not a fault");

    let text = output_text(&output);
    assert!(
        text.contains("999"),
        "the refusal must name the id that failed, got: {text}"
    );
    assert!(
        text.contains("no subagent") || text.contains("not running"),
        "the refusal must say why, got: {text}"
    );
}

#[tokio::test]
async fn steer_agent_reaches_a_live_child() {
    // The tool writes into the queue the child drains, so the message really
    // arrives. A tool that pushed into its own queue would look identical and do
    // nothing.
    let dir = tempfile::tempdir().unwrap();
    let env = spawn_env(
        dir.path(),
        vec![text_turn("done")],
        Some(vec!["read".to_string()]),
        vec!["read".to_string()],
    );
    let spawn = env
        .node
        .spawn_child("scout", rho_core::CancelToken::new())
        .unwrap();
    let id = spawn.node.id();

    let tool = rho_tools::SteerAgentTool::new(Arc::clone(&env));
    let output = tool
        .execute(
            serde_json::json!({ "id": id.0, "message": "check parser.rs instead" }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .unwrap();

    assert!(!output.is_error, "steering a live child must succeed");
    assert_eq!(
        spawn.queue().len(),
        1,
        "the message must land in the queue the child drains"
    );
}

#[tokio::test]
async fn cancel_agent_stops_one_child_and_leaves_its_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let env = spawn_env(
        dir.path(),
        vec![text_turn("done")],
        Some(vec!["read".to_string()]),
        vec!["read".to_string()],
    );
    let parent = rho_core::CancelToken::new();
    let first = env.node.spawn_child("scout", parent.child()).unwrap();
    let second = env.node.spawn_child("greedy", parent.child()).unwrap();

    let tool = rho_tools::CancelAgentTool::new(Arc::clone(&env));
    let output = tool
        .execute(
            serde_json::json!({ "id": first.node.id().0 }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .unwrap();
    assert!(!output.is_error);

    let live = env.node.registry().live();
    let first_handle = live.iter().find(|h| h.id == first.node.id()).unwrap();
    let second_handle = live.iter().find(|h| h.id == second.node.id()).unwrap();
    assert!(first_handle.is_cancelled(), "the named child must stop");
    assert!(!second_handle.is_cancelled(), "a sibling must keep running");
    assert!(!parent.is_cancelled(), "the parent must keep running");
}

#[tokio::test]
async fn spawn_agent_reports_a_rejected_task_as_an_error_result() {
    // `SPEC-agent-tasks` promised this and no test proved it. A rejected task must
    // set `is_error`, so a model that reads only the flag still learns the work was
    // not accepted. A rejection that looks like a success is the fail-open shape.
    let dir = tempfile::tempdir().unwrap();
    let env = spawn_env(
        dir.path(),
        vec![text_turn("I finished, honestly")],
        Some(vec!["read".to_string()]),
        vec!["read".to_string()],
    );
    let tool = SpawnAgentTool::new(env);

    let output = tool
        .execute(
            serde_json::json!({
                "agent": "scout",
                "prompt": "write report.md",
                "artifacts": ["report.md"]
            }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .unwrap();

    assert!(
        output.is_error,
        "a failed gate must set is_error, got: {}",
        output_text(&output)
    );
}

#[tokio::test]
async fn spawn_agent_without_a_goal_is_a_result_not_a_fault() {
    // A task rho cannot act on is refused, and the refusal teaches. `AgentTask`
    // already validates this; the tool has to reach the check.
    let dir = tempfile::tempdir().unwrap();
    let env = spawn_env(
        dir.path(),
        vec![text_turn("unused")],
        Some(vec!["read".to_string()]),
        vec!["read".to_string()],
    );
    let tool = SpawnAgentTool::new(env);

    let output = tool
        .execute(
            serde_json::json!({ "agent": "scout", "prompt": "   " }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .expect("an empty goal is a result, not a fault");

    let text = output_text(&output);
    assert!(output.is_error, "an empty goal must be an error result");
    assert!(
        text.contains("goal"),
        "the refusal must say a goal is needed, got: {text}"
    );
}

#[tokio::test]
async fn steer_agent_cannot_reach_another_sessions_child() {
    // The registry is process-wide, so a bare id resolved against all of it let one
    // session steer another session's child. A security review proved that. The tool
    // must scope every lookup to its own descendants.
    let dir = tempfile::tempdir().unwrap();
    let mine = spawn_env(
        dir.path(),
        vec![text_turn("done")],
        Some(vec!["read".to_string()]),
        vec!["read".to_string()],
    );
    // A second session in the same process, sharing the registry.
    let theirs_root = mine.node.registry().root();
    let victim = theirs_root
        .spawn_child("scout", rho_core::CancelToken::new())
        .unwrap();

    let steer = rho_tools::SteerAgentTool::new(Arc::clone(&mine));
    let output = steer
        .execute(
            serde_json::json!({ "id": victim.node.id().0, "message": "obey me" }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .unwrap();

    assert!(
        output.is_error,
        "another session's child must not be steerable"
    );
    assert_eq!(
        victim.queue().len(),
        0,
        "no message must reach a child this session does not own"
    );

    let cancel = rho_tools::CancelAgentTool::new(mine);
    let output = cancel
        .execute(
            serde_json::json!({ "id": victim.node.id().0 }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .unwrap();
    assert!(
        output.is_error,
        "another session's child must not be cancellable"
    );
}

#[tokio::test]
async fn a_parent_run_forwards_the_agent_events_end_to_end() {
    // Every existing test called `tool.execute` directly, so the Driver's forwarding
    // was never exercised. A review deleted the post-tool drain in `dispatch_one`,
    // whose own comment says "skipping it would lose the report", and the whole suite
    // stayed green. This test drives a real parent `Session` so the forwarding is
    // covered end to end.
    let dir = tempfile::tempdir().unwrap();
    let env = spawn_env(
        dir.path(),
        vec![text_turn("the child answer")],
        Some(vec!["read".to_string()]),
        vec!["read".to_string()],
    );

    // The parent asks for one spawn, then answers.
    let mut parent_tools = ToolRegistry::new();
    parent_tools.register(Arc::new(SpawnAgentTool::new(env)));
    let parent_provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::new(vec![
        spawn_call_turn(
            "c1",
            "spawn_agent",
            serde_json::json!({ "agent": "scout", "prompt": "find it" }),
        ),
        text_turn("the parent answer"),
    ]));
    let parent = rho_core::Session::with_config(
        rho_core::SessionConfig::new(
            "parent-model",
            dir.path().to_path_buf(),
            Arc::new(AllowAllPolicy),
        ),
        parent_provider,
        Arc::new(parent_tools),
        Arc::new(HookChain::new()),
        rho_core::Context::new(None, Vec::new()),
    );

    let cancel = rho_core::CancelToken::new();
    let mut events = parent.prompt(
        vec![rho_core::ContentBlock::Text {
            text: "delegate it".to_string(),
        }],
        cancel,
    );

    let mut spawned = 0;
    let mut finished = 0;
    while let Some(Ok(event)) = futures::StreamExt::next(&mut events).await {
        match event {
            rho_core::AgentEvent::AgentSpawned { .. } => spawned += 1,
            rho_core::AgentEvent::AgentFinished { report, .. } => {
                assert_eq!(report.agent, "scout", "the report must name the child");
                finished += 1;
            }
            _ => {}
        }
    }

    assert_eq!(spawned, 1, "the parent stream must carry the spawn");
    assert_eq!(
        finished, 1,
        "the parent stream must carry the finish, which the post-tool drain delivers"
    );
}

/// One turn that asks for a single tool call, then ends.
fn spawn_call_turn(id: &str, tool: &str, arguments: serde_json::Value) -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageStart {
            role: rho_core::Role::Assistant,
        },
        StreamEvent::ToolCallStart {
            index: 0,
            id: id.to_string(),
            name: tool.to_string(),
        },
        StreamEvent::ToolCallEnd {
            index: 0,
            arguments,
        },
        StreamEvent::Done {
            stop_reason: rho_core::StopReason::ToolUse,
        },
    ]
}

#[tokio::test]
async fn every_failing_outcome_sets_is_error() {
    // A parent that reads only `is_error` must never read a failure as a success.
    // `Rejected` was fixed and its three siblings were missed, in the same function,
    // a few lines apart. That is the fail-open family, found for the fourth time in
    // this feature.
    //
    // A hanging provider makes the child time out, which reports `Canceled`.
    let dir = tempfile::tempdir().unwrap();
    let tool = SpawnAgentTool::new(hanging_env(dir.path()));

    let output = tool
        .execute(
            serde_json::json!({ "agent": "scout", "prompt": "work that never finishes" }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .expect("a timed-out child is a result, not a fault");

    let text = output_text(&output);
    assert!(
        output.is_error,
        "a cancelled child must set is_error, got text: {text}"
    );
    assert!(
        text.contains("scout"),
        "and the note must still name the agent, got: {text}"
    );
}

#[tokio::test]
async fn the_tool_result_points_the_parent_at_the_transcript() {
    // The summary is the default, and the full transcript is one line away. rho wrote
    // the file and told nobody, so the parent could not choose to read it.
    let dir = tempfile::tempdir().unwrap();
    let env = spawn_env(
        dir.path(),
        vec![text_turn("the child answer")],
        Some(vec!["read".to_string()]),
        vec!["read".to_string()],
    );
    let tool = SpawnAgentTool::new(env);

    let output = tool
        .execute(
            serde_json::json!({ "agent": "scout", "prompt": "find it" }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .unwrap();

    let text = output_text(&output);
    assert!(
        text.contains("full transcript:"),
        "the parent must be told where the transcript is, got: {text}"
    );
    let path = text
        .split("full transcript: ")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .expect("the note carries a path");
    assert!(
        std::path::Path::new(path).exists(),
        "the path must name a file that exists: {path}"
    );
    let body = std::fs::read_to_string(path).unwrap();
    assert!(
        body.contains("the child answer"),
        "and the file must hold the child's work, got: {body}"
    );
}

// --- Background children (round nine) ---

#[tokio::test]
async fn a_background_spawn_returns_at_once_and_names_the_child() {
    // The must-have. `spawn_agent` blocked until the child finished, so a parent could
    // never poll one: by the time it got a turn, the child was gone. A background
    // spawn returns the id straight away and the child keeps working.
    let dir = tempfile::tempdir().unwrap();
    let tool = SpawnAgentTool::new(hanging_env(dir.path()));

    let output = tool
        .execute(
            serde_json::json!({
                "agent": "scout",
                "prompt": "take your time",
                "background": true
            }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .expect("a background spawn is a result");

    let text = output_text(&output);
    assert!(!output.is_error, "starting a child is not an error: {text}");
    assert!(
        text.contains("id"),
        "the parent must be told the id so it can poll, got: {text}"
    );
    assert!(
        !text.contains("full transcript"),
        "a child that has not finished has no final transcript line yet, got: {text}"
    );
}

#[tokio::test]
async fn agent_status_reports_a_finished_background_child() {
    // Polling is useless if a finished child vanishes. `ChildSlot::drop` removes the
    // live handle, so the outcome has to be kept somewhere the parent can still read.
    let dir = tempfile::tempdir().unwrap();
    let env = spawn_env(
        dir.path(),
        vec![text_turn("the background answer")],
        Some(vec!["read".to_string()]),
        vec!["read".to_string()],
    );
    let spawn = SpawnAgentTool::new(Arc::clone(&env));

    let started = spawn
        .execute(
            serde_json::json!({ "agent": "scout", "prompt": "do it", "background": true }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .unwrap();
    let id: u64 = output_text(&started)
        .split("id ")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|digits| digits.parse().ok())
        .unwrap_or_else(|| {
            panic!(
                "the result must name a numeric id: {}",
                output_text(&started)
            )
        });

    // Wait for it to finish, without a sleep: poll the status the parent would poll.
    let status = rho_tools::AgentStatusTool::new(Arc::clone(&env));
    let mut text = String::new();
    for _ in 0..200 {
        let out = status
            .execute(
                serde_json::json!({ "id": id }),
                ctx(dir.path().to_path_buf()),
            )
            .await
            .unwrap();
        text = output_text(&out);
        if text.contains("done") || text.contains("finished") {
            break;
        }
        tokio::task::yield_now().await;
    }

    assert!(
        text.contains("done") || text.contains("finished"),
        "a finished background child must still be reportable, got: {text}"
    );
    assert!(
        text.contains("the background answer") || text.contains("transcript"),
        "and the parent must be able to reach its work, got: {text}"
    );
}
