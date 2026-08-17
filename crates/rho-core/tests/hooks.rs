//! Tests for the hook chain and tool dispatch.
//!
//! These drive the agent loop over a scripted provider that asks for one tool.
//! The loop body is `todo!()` in S3, so they fail for the right reason until S4.

mod common;

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use common::{
    ArgEditHook, BlockingHook, OrderRecordingHook, OutputEditHook, RecordingTool, ScriptedProvider,
    tool_call_turn,
};
use futures::StreamExt;
use rho_core::{
    AgentEvent, AgentEvents, CancelToken, ContentBlock, Context, Hook, HookChain, Provider,
    Session, Tool, ToolOutput, ToolRegistry,
};

fn user_input(text: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text {
        text: text.to_string(),
    }]
}

/// Build a session that asks for `tool` once, with the given hooks and tool.
fn session_with_hooks(tool: Arc<RecordingTool>, hooks: Vec<Arc<dyn Hook>>) -> Session {
    let mut registry = ToolRegistry::new();
    let name = tool.name().to_string();
    registry.register(tool);
    let mut chain = HookChain::new();
    for hook in hooks {
        chain.push(hook);
    }
    let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::new(vec![
        tool_call_turn("call_1", &name, serde_json::json!({ "n": 1 })),
        common::text_turn("done"),
    ]));
    let context = Context::new(Some("system".to_string()), Vec::new());
    Session::new(provider, Arc::new(registry), Arc::new(chain), context)
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

#[tokio::test]
async fn hook_chain_runs_in_registration_order() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let tool = Arc::new(RecordingTool::new("do_it"));
    let hooks: Vec<Arc<dyn Hook>> = vec![
        Arc::new(OrderRecordingHook::new("first", Arc::clone(&log))),
        Arc::new(OrderRecordingHook::new("second", Arc::clone(&log))),
    ];
    let session = session_with_hooks(tool, hooks);

    let _ = collect(session.prompt(user_input("go"), CancelToken::new())).await;

    let order = log.lock().unwrap().clone();
    assert_eq!(order, vec!["first".to_string(), "second".to_string()]);
}

#[tokio::test]
async fn hook_before_tool_call_first_block_wins() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let tool = Arc::new(RecordingTool::new("do_it"));
    let ran = Arc::clone(&tool.ran);
    let hooks: Vec<Arc<dyn Hook>> = vec![
        Arc::new(BlockingHook::new("blocker", Arc::clone(&log))),
        Arc::new(OrderRecordingHook::new("second", Arc::clone(&log))),
    ];
    let session = session_with_hooks(tool, hooks);

    let _ = collect(session.prompt(user_input("go"), CancelToken::new())).await;

    let order = log.lock().unwrap().clone();
    assert_eq!(
        order,
        vec!["blocker".to_string()],
        "the second hook must not run"
    );
    assert!(
        !ran.load(Ordering::SeqCst),
        "the tool must not run after a block"
    );
}

#[tokio::test]
async fn hook_before_tool_call_edits_arguments() {
    let tool = Arc::new(RecordingTool::new("do_it"));
    let seen = Arc::clone(&tool.last_args);
    let hooks: Vec<Arc<dyn Hook>> = vec![Arc::new(ArgEditHook::new(
        "editor",
        "n",
        serde_json::json!(42),
    ))];
    let session = session_with_hooks(tool, hooks);

    let _ = collect(session.prompt(user_input("go"), CancelToken::new())).await;

    let args = seen
        .lock()
        .unwrap()
        .clone()
        .expect("the tool ran with args");
    assert_eq!(
        args["n"],
        serde_json::json!(42),
        "the tool sees the edited argument"
    );
}

#[tokio::test]
async fn hook_after_tool_result_edits_output() {
    let tool = Arc::new(RecordingTool::new("do_it"));
    let hooks: Vec<Arc<dyn Hook>> = vec![Arc::new(OutputEditHook::new("appender", "EDITED"))];
    let session = session_with_hooks(tool, hooks);

    let got = collect(session.prompt(user_input("go"), CancelToken::new())).await;

    let output = tool_output(&got).expect("a tool result");
    let has_marker = output
        .content
        .iter()
        .any(|block| matches!(block, ContentBlock::Text { text } if text == "EDITED"));
    assert!(has_marker, "the loop appends the edited output");
}

#[tokio::test]
async fn hook_block_produces_error_tool_result() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let tool = Arc::new(RecordingTool::new("do_it"));
    let ran = Arc::clone(&tool.ran);
    let hooks: Vec<Arc<dyn Hook>> = vec![Arc::new(BlockingHook::new("blocker", log))];
    let session = session_with_hooks(tool, hooks);

    let got = collect(session.prompt(user_input("go"), CancelToken::new())).await;

    let output = tool_output(&got).expect("an error tool result");
    assert!(
        output.is_error,
        "a blocked call yields an error tool result"
    );
    assert!(!ran.load(Ordering::SeqCst), "the blocked tool never runs");
}
