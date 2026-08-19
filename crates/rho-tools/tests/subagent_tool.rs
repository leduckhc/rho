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
    assert_eq!(text, "the bug is at parser.rs:42");
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
