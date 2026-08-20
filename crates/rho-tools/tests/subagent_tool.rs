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

/// A provider that replays one turn for ever, so a child runs until a cap stops it.
struct CyclingProvider {
    turn: Vec<StreamEvent>,
}

#[async_trait]
impl Provider for CyclingProvider {
    fn id(&self) -> &str {
        "cycling"
    }
    async fn stream(
        &self,
        _request: CompletionRequest,
        _cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError> {
        Ok(Box::pin(stream::iter(
            self.turn.clone().into_iter().map(Ok),
        )))
    }
}

/// One turn that asks for a tool call, so the run continues past it.
fn tool_call_turn(id: &str, tool_name: &str, arguments: serde_json::Value) -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageStart {
            role: Role::Assistant,
        },
        StreamEvent::ToolCallStart {
            index: 0,
            id: id.to_string(),
            name: tool_name.to_string(),
        },
        StreamEvent::ToolCallEnd {
            index: 0,
            arguments,
        },
        StreamEvent::Done {
            stop_reason: StopReason::ToolUse,
        },
    ]
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
        node: registry.new_tree(),
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
        node: registry.new_tree(),
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
        node: registry.new_tree(),
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

/// A fan-out env whose process-wide cap is `max_live`. That cap still refuses.
fn live_capped_env(dir: &std::path::Path, max_live: usize) -> Arc<SpawnEnv> {
    let limits = SubagentLimits {
        max_live_total: max_live,
        ..SubagentLimits::new()
    };
    let registry = AgentRegistry::new(limits);
    let def = definition(dir, Some(vec!["read".to_string()]));
    let mut definitions = HashMap::new();
    definitions.insert(def.name.clone(), def);
    Arc::new(SpawnEnv {
        node: registry.new_tree(),
        definitions,
        parent_config: SessionConfig::new(
            "parent-model",
            dir.to_path_buf(),
            Arc::new(AllowAllPolicy),
        ),
        provider: Arc::new(ScriptedProvider::new(vec![
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
async fn a_task_over_the_process_wide_cap_is_refused_and_the_others_still_run() {
    // The per-parent cap queues now, so this pins the cap that still refuses. It must
    // bite per task, never per call: a limit that failed the whole fan-out would make
    // one refused task lose every good result. See section 2.7 of
    // `SPEC-subagent-slots-handles-grace`.
    let dir = tempfile::tempdir().unwrap();
    let tool = SpawnAgentsTool::new(live_capped_env(dir.path(), 1));

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
        text.contains("--max-live-agents"),
        "a refusal must name the flag that would raise the limit, got: {text}"
    );
    assert!(
        text.contains("child answer"),
        "the task that fitted must still report its work, got: {text}"
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

    let live = env.node.registry().live_under(&env.node);
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
    let theirs_root = mine.node.registry().new_tree();
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

// --- Grace turns reach a real child (SPEC-subagent-slots-handles-grace section 4) ---

/// A child whose model keeps calling a tool, so it runs until its turn cap.
///
/// A text turn ends a run at `EndTurn`, so a text script never reaches a cap and
/// never reaches the grace window.
fn grace_env(dir: &std::path::Path, grace_turns: u32, max_turns: u32) -> Arc<SpawnEnv> {
    let limits = SubagentLimits {
        grace_turns,
        ..SubagentLimits::new()
    };
    let registry = AgentRegistry::new(limits);
    let mut def = definition(dir, Some(vec!["read".to_string()]));
    def.max_turns = Some(max_turns);
    let mut definitions = HashMap::new();
    definitions.insert(def.name.clone(), def);
    Arc::new(SpawnEnv {
        node: registry.new_tree(),
        definitions,
        parent_config: SessionConfig::new(
            "parent-model",
            dir.to_path_buf(),
            Arc::new(AllowAllPolicy),
        ),
        provider: Arc::new(CyclingProvider {
            turn: tool_call_turn("c1", "read", serde_json::json!({})),
        }),
        hooks: Arc::new(HookChain::new()),
        tools: Arc::new(FakeToolFactory {
            parent: vec!["read".to_string()],
        }),
        transcript_dir: dir.to_path_buf(),
        runner: Arc::new(rho_tools::SandboxedRunner::new(rho_core::SandboxMode::Off)),
        retries: Arc::new(rho_core::RetryLedger::new()),
    })
}

#[tokio::test]
async fn a_child_is_warned_before_its_turn_cap() {
    // The wiring test. `SubagentLimits::grace_turns` must reach the child's
    // `SessionConfig`, or the feature exists in `rho-core` and never runs.
    let dir = tempfile::tempdir().unwrap();
    let env = grace_env(dir.path(), 2, 4);
    let tool = SpawnAgentTool::new(Arc::clone(&env));
    let tool_ctx = ctx(dir.path().to_path_buf());

    let result = tool
        .execute(
            serde_json::json!({ "agent": "scout", "prompt": "work until you stop" }),
            tool_ctx,
        )
        .await
        .expect("a spawn returns a result");

    let transcript = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .unwrap_or_default();

    // The transcript records a delivery, and nothing else steers this child, so a
    // delivery here can only be the grace warning. The verbatim text is proved in
    // `crates/rho-core/tests/subagent_grace.rs`.
    assert!(
        transcript.contains(r#""type":"Delivered""#),
        "the child must be warned before its cap. Result: {:?}. Transcript: {transcript}",
        result.content
    );
}

#[tokio::test]
async fn a_child_with_a_zero_grace_window_is_not_warned() {
    // The negative half. A host that turns the warning off must reach the child too.
    let dir = tempfile::tempdir().unwrap();
    let env = grace_env(dir.path(), 0, 4);
    let tool = SpawnAgentTool::new(Arc::clone(&env));
    let tool_ctx = ctx(dir.path().to_path_buf());

    tool.execute(
        serde_json::json!({ "agent": "scout", "prompt": "work until you stop" }),
        tool_ctx,
    )
    .await
    .expect("a spawn returns a result");

    let transcript = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .unwrap_or_default();

    assert!(
        !transcript.contains(r#""type":"Delivered""#),
        "a zero window must send nothing: {transcript}"
    );
}

#[tokio::test]
async fn a_grace_window_wider_than_the_child_turn_cap_still_warns_once_after_work() {
    // A window of 9 against a cap of 2. The warning must still fire once, and it must
    // land after the child has taken a turn, because a child asked to summarise
    // nothing wastes the turn. The driver's boundary rule is what guarantees this.
    let dir = tempfile::tempdir().unwrap();
    let env = grace_env(dir.path(), 9, 2);
    let tool = SpawnAgentTool::new(Arc::clone(&env));
    let tool_ctx = ctx(dir.path().to_path_buf());

    tool.execute(
        serde_json::json!({ "agent": "scout", "prompt": "work until you stop" }),
        tool_ctx,
    )
    .await
    .expect("a spawn returns a result");

    let transcript = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .unwrap_or_default();

    // A cap of two and a window of nine.
    let deliveries = transcript.matches(r#""type":"Delivered""#).count();
    let first_turn = transcript.find(r#""type":"TurnStart""#).unwrap_or(0);
    let first_delivery = transcript
        .find(r#""type":"Delivered""#)
        .expect("a capped window must still warn once");
    assert_eq!(deliveries, 1, "one warning only: {transcript}");
    assert!(
        first_delivery > first_turn,
        "the child must work before it is told to summarise: {transcript}"
    );
}

// --- The queue reaches the model (SPEC-subagent-slots-handles-grace section 2.8) ---

/// An env whose parent cap is `max_children` and whose child never answers.
///
/// A hanging child holds its slot, so the next spawn has to queue. The timeout is
/// long on purpose: a slot that frees itself would prove nothing about a queue.
fn queued_env(dir: &std::path::Path, max_children: usize) -> Arc<SpawnEnv> {
    let limits = SubagentLimits {
        max_children_per_parent: max_children,
        child_timeout: Duration::from_secs(600),
        ..SubagentLimits::new()
    };
    let registry = AgentRegistry::new(limits);
    let def = definition(dir, Some(vec!["read".to_string()]));
    let mut definitions = HashMap::new();
    definitions.insert(def.name.clone(), def);
    Arc::new(SpawnEnv {
        node: registry.new_tree(),
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

/// The id a background spawn reported. A test that guessed it would prove nothing.
fn spawned_id(output: &rho_core::ToolOutput) -> u64 {
    let text = output_text(output);
    text.split("id ")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|digits| digits.parse().ok())
        .unwrap_or_else(|| panic!("the result must name a numeric id: {text}"))
}

/// Queue a background child, and prove the id arrives before the child starts.
///
/// The bound is the assertion. The parent's slot is held by a child that never
/// answers, so a spawn that waited for the start would never return at all. A test
/// that just awaited it would hang, and a hang teaches nothing.
async fn queue_a_child(env: &Arc<SpawnEnv>, dir: &std::path::Path) -> rho_core::ToolOutput {
    let tool = SpawnAgentTool::new(Arc::clone(env));
    tokio::time::timeout(
        Duration::from_secs(5),
        tool.execute(
            serde_json::json!({ "agent": "scout", "prompt": "wait your turn", "background": true }),
            ctx(dir.to_path_buf()),
        ),
    )
    .await
    .expect("a queued child reports its id at once, so the model can poll it while it waits")
    .expect("a spawn over the cap is admitted, not refused")
}

/// Start a background child that hangs, so it holds its slot for the whole test.
async fn hold_a_slot(env: &Arc<SpawnEnv>, dir: &std::path::Path) -> u64 {
    let tool = SpawnAgentTool::new(Arc::clone(env));
    let output = tool
        .execute(
            serde_json::json!({ "agent": "scout", "prompt": "hold the slot", "background": true }),
            ctx(dir.to_path_buf()),
        )
        .await
        .expect("a background spawn is a result");
    spawned_id(&output)
}

#[tokio::test]
async fn a_fan_out_over_the_cap_queues_the_extra_tasks_and_runs_them_all() {
    // The per-parent cap used to refuse a task, so a fan-out of four under a cap of two
    // lost half the work and the model had to retry it by hand. Every task must now run.
    // See decision D-queue-over-refuse.
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
        .expect("a fan-out is a result, not a fault");

    let text = output_text(&output);
    assert!(
        !text.contains("per-parent child limit"),
        "the per-parent cap must queue, never refuse a task: {text}"
    );
    assert_eq!(
        text.matches("child answer").count(),
        4,
        "every task must run, whether it waited or not: {text}"
    );
}

#[tokio::test]
async fn a_fan_out_reports_in_request_order_though_start_order_differs() {
    // The first task never starts, because its agent does not exist. So the start order
    // is beta then gamma, and the report order must still be alpha, beta, gamma. A
    // result order that followed the start order would break the stable prompt prefix,
    // and with it the provider cache. See decision D-per-parent-fifo-start-order.
    let dir = tempfile::tempdir().unwrap();
    let tool = SpawnAgentsTool::new(fanout_env(dir.path(), 1));

    let output = tool
        .execute(
            serde_json::json!({
                "tasks": [
                    { "agent": "ghost", "prompt": "alpha" },
                    { "agent": "scout", "prompt": "beta" },
                    { "agent": "scout", "prompt": "gamma" }
                ]
            }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .expect("a fan-out is a result, not a fault");

    let text = output_text(&output);
    let alpha = text.find("alpha").expect("task 1 must be reported");
    let beta = text.find("beta").expect("task 2 must be reported");
    let gamma = text.find("gamma").expect("task 3 must be reported");
    assert!(
        alpha < beta && beta < gamma,
        "the report follows the request, not the start: {text}"
    );
    assert_eq!(
        text.matches("child answer").count(),
        2,
        "both real tasks must run under a cap of one: {text}"
    );
}

#[tokio::test]
async fn agent_status_answers_for_a_queued_child() {
    // A queued id the model cannot poll is a dead id. The tool path must answer, and it
    // must name the place in the line, because that is the one number a model can act on.
    let dir = tempfile::tempdir().unwrap();
    let env = queued_env(dir.path(), 1);
    let _holder = hold_a_slot(&env, dir.path()).await;

    let queued = queue_a_child(&env, dir.path()).await;
    assert!(
        !queued.is_error,
        "a full parent queues, so this is not an error: {}",
        output_text(&queued)
    );
    let id = spawned_id(&queued);

    let status = rho_tools::AgentStatusTool::new(Arc::clone(&env));
    let output = status
        .execute(
            serde_json::json!({ "id": id }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .expect("a status read is a result");
    let text = output_text(&output);
    assert!(
        text.contains("queued") && text.contains("place 1"),
        "the tool must report the queued state and its place: {text}"
    );
}

#[tokio::test]
async fn cancel_agent_stops_a_queued_child() {
    // A queued child that cannot be cancelled would hold its place until a slot freed,
    // and then run work the parent no longer wants.
    let dir = tempfile::tempdir().unwrap();
    let env = queued_env(dir.path(), 1);
    let _holder = hold_a_slot(&env, dir.path()).await;

    let queued = queue_a_child(&env, dir.path()).await;
    let id = spawned_id(&queued);

    let cancel = rho_tools::CancelAgentTool::new(Arc::clone(&env));
    let output = cancel
        .execute(
            serde_json::json!({ "id": id }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .expect("a cancel is a result");
    let text = output_text(&output);
    assert!(
        !output.is_error,
        "a queued child is cancellable by the id the model holds: {text}"
    );
    assert!(
        text.contains("stop"),
        "the result must say the child was asked to stop: {text}"
    );
}

#[tokio::test]
async fn steer_agent_buffers_for_a_queued_child() {
    // The model is told the id at once, so it may steer before the child starts. A
    // dropped message there would be a silent loss.
    let dir = tempfile::tempdir().unwrap();
    let env = queued_env(dir.path(), 1);
    let _holder = hold_a_slot(&env, dir.path()).await;

    let queued = queue_a_child(&env, dir.path()).await;
    let id = spawned_id(&queued);

    let steer = rho_tools::SteerAgentTool::new(Arc::clone(&env));
    let output = steer
        .execute(
            serde_json::json!({ "id": id, "message": "read the spec first" }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .expect("a steer is a result");
    let text = output_text(&output);
    assert!(
        !output.is_error,
        "a queued child accepts a steer, which buffers: {text}"
    );
    assert!(
        text.contains("position 1"),
        "the result must say where the message sits: {text}"
    );
    assert!(
        text.contains("scout"),
        "the receipt must name the agent, which a queued child holds no live handle for: {text}"
    );
}

#[tokio::test]
async fn a_blocking_spawn_over_the_cap_waits_and_then_runs() {
    // A blocking spawn used to be refused over the cap. It now waits, and the model's
    // turn waits with it. Both halves matter: it must not return before a slot frees,
    // and it must run once one does. A test that only checked the end would pass against
    // an implementation that never queued at all.
    let dir = tempfile::tempdir().unwrap();
    let limits = SubagentLimits {
        max_children_per_parent: 1,
        child_timeout: Duration::from_secs(600),
        ..SubagentLimits::new()
    };
    let registry = AgentRegistry::new(limits);
    let def = definition(dir.path(), Some(vec!["read".to_string()]));
    let mut definitions = HashMap::new();
    definitions.insert(def.name.clone(), def);
    let env = Arc::new(SpawnEnv {
        node: registry.new_tree(),
        definitions,
        parent_config: SessionConfig::new(
            "parent-model",
            dir.path().to_path_buf(),
            Arc::new(AllowAllPolicy),
        ),
        // The first child never answers, so only a cancel frees its slot.
        provider: Arc::new(FirstHangsProvider {
            calls: Mutex::new(0),
        }),
        hooks: Arc::new(HookChain::new()),
        tools: Arc::new(FakeToolFactory {
            parent: vec!["read".to_string()],
        }),
        transcript_dir: dir.path().to_path_buf(),
        runner: Arc::new(rho_tools::SandboxedRunner::new(rho_core::SandboxMode::Off)),
        retries: Arc::new(rho_core::RetryLedger::new()),
    });

    let holder = hold_a_slot(&env, dir.path()).await;

    let spawn = SpawnAgentTool::new(Arc::clone(&env));
    let root = dir.path().to_path_buf();
    let mut waiting = tokio::spawn(async move {
        spawn
            .execute(
                serde_json::json!({ "agent": "scout", "prompt": "wait for a slot" }),
                ctx(root),
            )
            .await
            .expect("a blocking spawn is a result")
    });

    // It cannot finish while the only slot is held, and it must not refuse either.
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut waiting)
            .await
            .is_err(),
        "a blocking spawn over the cap waits, and it does not return a refusal"
    );

    let cancel = rho_tools::CancelAgentTool::new(Arc::clone(&env));
    cancel
        .execute(
            serde_json::json!({ "id": holder }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .expect("a cancel is a result");

    let output = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("a freed slot must start the waiter")
        .expect("the spawn task must not panic");
    let text = output_text(&output);
    assert!(
        text.contains("child answer"),
        "the waiter runs once a slot frees: {text}"
    );
}

/// A provider that hangs the first child and answers every later one.
///
/// The first child then dies at its own timeout and frees the slot, so the queued
/// child starts late. A single hanging provider could not show this, because every
/// child would hang.
struct FirstHangsProvider {
    calls: Mutex<usize>,
}

#[async_trait]
impl Provider for FirstHangsProvider {
    fn id(&self) -> &str {
        "first-hangs"
    }
    async fn stream(
        &self,
        _request: CompletionRequest,
        _cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError> {
        let first = {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            *calls == 1
        };
        if first {
            return Ok(Box::pin(stream::pending()));
        }
        Ok(Box::pin(stream::iter(
            text_turn("child answer").into_iter().map(Ok),
        )))
    }
}

// A paused clock, so the wait is a whole timeout long and costs the test no real time.
// Tokio advances a paused clock to the next timer when every task is idle, and the only
// timer here is the first child's timeout. So the waiter starts exactly when the first
// child's budget runs out, which is the moment a shared clock would show.
#[tokio::test(start_paused = true)]
async fn a_queued_child_does_not_spend_its_timeout_while_it_waits() {
    // The first child hangs and dies at its 300 ms timeout. The second waited that
    // whole time, and it must still arrive with its full budget. A clock that started
    // at admission would cancel it the moment it started, and the parent would read a
    // cancelled child that never ran a turn. See section 2.4.
    let dir = tempfile::tempdir().unwrap();
    let limits = SubagentLimits {
        max_children_per_parent: 1,
        child_timeout: Duration::from_millis(300),
        ..SubagentLimits::new()
    };
    let registry = AgentRegistry::new(limits);
    let def = definition(dir.path(), Some(vec!["read".to_string()]));
    let mut definitions = HashMap::new();
    definitions.insert(def.name.clone(), def);
    let env = Arc::new(SpawnEnv {
        node: registry.new_tree(),
        definitions,
        parent_config: SessionConfig::new(
            "parent-model",
            dir.path().to_path_buf(),
            Arc::new(AllowAllPolicy),
        ),
        provider: Arc::new(FirstHangsProvider {
            calls: Mutex::new(0),
        }),
        hooks: Arc::new(HookChain::new()),
        tools: Arc::new(FakeToolFactory {
            parent: vec!["read".to_string()],
        }),
        transcript_dir: dir.path().to_path_buf(),
        runner: Arc::new(rho_tools::SandboxedRunner::new(rho_core::SandboxMode::Off)),
        retries: Arc::new(rho_core::RetryLedger::new()),
    });

    let tool = SpawnAgentsTool::new(env);
    let output = tool
        .execute(
            serde_json::json!({
                "tasks": [
                    { "agent": "scout", "prompt": "the one that hangs" },
                    { "agent": "scout", "prompt": "the one that waits" }
                ]
            }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .expect("a fan-out is a result, not a fault");

    let text = output_text(&output);
    let waiter = text
        .split("## scout — the one that waits")
        .nth(1)
        .expect("the waiting task must be reported")
        .to_string();
    assert!(
        waiter.contains("child answer"),
        "a task that waited keeps its whole budget: {text}"
    );
    assert!(
        !waiter.contains("was cancelled"),
        "the timeout clock must start at the start, not at the admission: {text}"
    );
}

#[tokio::test]
async fn agent_status_says_a_cancelled_queued_child_will_not_start() {
    // A live run on Bedrock cancelled a queued child and polled it at once. The tool
    // still promised the child would start, and offered a steer that could never be
    // delivered. Whatever the timing, the answer must never promise a start after a
    // cancel. See decision D-a-cancelled-waiter-says-so.
    let dir = tempfile::tempdir().unwrap();
    let env = queued_env(dir.path(), 1);
    let _holder = hold_a_slot(&env, dir.path()).await;
    let id = spawned_id(&queue_a_child(&env, dir.path()).await);

    let cancel = rho_tools::CancelAgentTool::new(Arc::clone(&env));
    cancel
        .execute(
            serde_json::json!({ "id": id }),
            ctx(dir.path().to_path_buf()),
        )
        .await
        .expect("a cancel is a result");

    let status = rho_tools::AgentStatusTool::new(Arc::clone(&env));
    let text = output_text(
        &status
            .execute(
                serde_json::json!({ "id": id }),
                ctx(dir.path().to_path_buf()),
            )
            .await
            .expect("a status read is a result"),
    );

    assert!(
        !text.contains("it will start"),
        "a cancelled child must never be promised a start: {text}"
    );
    assert!(
        text.contains("cancel"),
        "the answer must say the child was cancelled: {text}"
    );
}
