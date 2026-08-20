//! Tests for the `spawn_agent` tool. See `SPEC-subagents` sections 3 and 6.
//!
//! These prove the parent receives only the summary, and that a dropped tool
//! name is reported to the caller.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::stream;
use rho_core::{
    AgentRegistry, AllowAllPolicy, CancelToken, CompletionRequest, ContentBlock, HookChain,
    Provider, ProviderError, ProviderStream, Role, SessionConfig, StopReason, StreamEvent,
    SubagentLimits, Tool, ToolContext, ToolRegistry,
};
use rho_skills::{AgentDefinition, SkillOrigin};
use rho_tools::{ChildToolFactory, SpawnAgentTool, SpawnEnv};

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
    })
}

fn ctx(root: PathBuf) -> ToolContext {
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    ToolContext {
        session_root: root,
        cancel: CancelToken::new(),
        updates: tx,
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
