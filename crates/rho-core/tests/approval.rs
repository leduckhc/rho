//! Tests for the approval policy wired into tool dispatch.
//!
//! A denied call must never run the tool. It must produce an error tool result,
//! so the model sees the denial and can change course. See SPEC-01 section 9 and
//! SPEC-03 section 5.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use common::{ScriptedProvider, text_turn, tool_call_turn};
use futures::StreamExt;
use rho_core::{
    AgentEvent, AgentEvents, CancelToken, ContentBlock, Context, HookChain, Provider,
    ReadOnlyPolicy, Session, SessionConfig, Tool, ToolContext, ToolError, ToolKind, ToolOutput,
    ToolRegistry,
};

/// A tool with a chosen `ToolKind` that records whether it ran. The kind drives
/// the approval decision, so the test must set it.
struct KindedTool {
    name: String,
    kind: ToolKind,
    ran: Arc<AtomicBool>,
}

impl KindedTool {
    fn new(name: &str, kind: ToolKind) -> Self {
        Self {
            name: name.to_string(),
            kind,
            ran: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[async_trait]
impl Tool for KindedTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "a kinded test tool"
    }
    fn kind(&self) -> ToolKind {
        self.kind
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        self.ran.store(true, Ordering::SeqCst);
        Ok(ToolOutput::text("tool ran"))
    }
}

fn user_input(text: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text {
        text: text.to_string(),
    }]
}

async fn collect(mut events: AgentEvents) -> Vec<AgentEvent> {
    let mut out = Vec::new();
    while let Some(item) = events.next().await {
        out.push(item.expect("no error event"));
    }
    out
}

fn tool_output(events: &[AgentEvent]) -> Option<ToolOutput> {
    events.iter().find_map(|e| match e {
        AgentEvent::ToolEnd { output, .. } => Some(output.clone()),
        _ => None,
    })
}

/// Build a session over a `ReadOnlyPolicy` that asks for `tool` once.
fn session_read_only(tool: Arc<KindedTool>) -> Session {
    let mut registry = ToolRegistry::new();
    let name = tool.name().to_string();
    registry.register(tool);
    let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::new(vec![
        tool_call_turn("call_1", &name, serde_json::json!({})),
        text_turn("done"),
    ]));
    let config = SessionConfig::new("test-model", ".", Arc::new(ReadOnlyPolicy));
    Session::with_config(
        config,
        provider,
        Arc::new(registry),
        Arc::new(HookChain::new()),
        Context::new(Some("system".to_string()), Vec::new()),
    )
}

#[tokio::test]
async fn approval_denied_mutating_call_never_runs_the_tool() {
    let tool = Arc::new(KindedTool::new("write", ToolKind::Edit));
    let ran = Arc::clone(&tool.ran);
    let session = session_read_only(tool);

    let got = collect(session.prompt(user_input("go"), CancelToken::new())).await;

    assert!(!ran.load(Ordering::SeqCst), "a denied tool must never run");
    let output = tool_output(&got).expect("a denied call still ends with a tool result");
    assert!(output.is_error, "the denied result is an error result");
}

#[tokio::test]
async fn approval_denied_result_carries_the_denied_variant_message() {
    // The denial must flow through the typed `ToolError::Denied` variant. This
    // test pins the message to the variant, so the wiring cannot be removed in
    // silence. See SPEC-03 section 5.
    let tool = Arc::new(KindedTool::new("write", ToolKind::Edit));
    let session = session_read_only(tool);

    let got = collect(session.prompt(user_input("go"), CancelToken::new())).await;

    let output = tool_output(&got).expect("a denied call still ends with a tool result");
    assert!(output.is_error, "a denied call yields an error result");
    let text = output
        .content
        .iter()
        .find_map(|block| match block {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .expect("a text block in the denial result");
    assert_eq!(
        text,
        ToolError::Denied.to_string(),
        "the denial message comes from the ToolError::Denied variant"
    );
}

#[tokio::test]
async fn approval_allowed_reading_call_runs_the_tool() {
    let tool = Arc::new(KindedTool::new("read", ToolKind::Read));
    let ran = Arc::clone(&tool.ran);
    let session = session_read_only(tool);

    let got = collect(session.prompt(user_input("go"), CancelToken::new())).await;

    assert!(ran.load(Ordering::SeqCst), "an allowed tool runs");
    let output = tool_output(&got).expect("a tool result");
    assert!(!output.is_error, "an allowed result is not an error");
}
