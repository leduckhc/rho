//! Tests for the agent loop.
//!
//! Each test drives a scripted fake provider. The loop body is `todo!()` in S3,
//! so these tests fail for the right reason until S4 fills it in. They compile
//! against the real public types.

mod common;

use std::sync::Arc;

use common::{RecordingTool, ScriptedProvider, text_turn, tool_call_turn};
use futures::StreamExt;
use rho_core::{
    AgentEvent, AgentEvents, AgentStopReason, CancelToken, ContentBlock, Context, HookChain,
    Provider, Role, Session, StopReason, StreamEvent, ToolRegistry,
};

/// Build a session over a scripted provider, with the given tools registered.
fn session_with(provider: Arc<dyn Provider>, tools: ToolRegistry) -> Session {
    let hooks = HookChain::new();
    let context = Context::new(Some("system".to_string()), Vec::new());
    Session::with_config(
        common::test_config(),
        provider,
        Arc::new(tools),
        Arc::new(hooks),
        context,
    )
}

/// Drain every event of a run into a vector.
async fn collect(mut events: AgentEvents) -> Vec<AgentEvent> {
    let mut out = Vec::new();
    while let Some(item) = events.next().await {
        out.push(item.expect("no error event in these scripts"));
    }
    out
}

fn user_input(text: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text {
        text: text.to_string(),
    }]
}

#[tokio::test]
async fn agent_loop_emits_turn_start_then_stream_then_turn_end() {
    let provider = Arc::new(ScriptedProvider::new(vec![text_turn("hi there")]));
    let session = session_with(provider, ToolRegistry::new());

    let events = session.prompt(user_input("hello"), CancelToken::new());
    let got = collect(events).await;

    assert_eq!(got.first(), Some(&AgentEvent::TurnStart));
    assert!(
        got.iter()
            .any(|e| matches!(e, AgentEvent::Stream(StreamEvent::TextDelta { .. }))),
        "the run must forward stream text deltas"
    );
    let turn_end_pos = got
        .iter()
        .position(|e| matches!(e, AgentEvent::TurnEnd { .. }))
        .expect("a TurnEnd event");
    let agent_end_pos = got
        .iter()
        .position(|e| matches!(e, AgentEvent::AgentEnd { .. }))
        .expect("an AgentEnd event");
    assert!(turn_end_pos < agent_end_pos, "TurnEnd precedes AgentEnd");
}

#[tokio::test]
async fn agent_loop_runs_tool_then_continues() {
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(RecordingTool::new("do_it")));
    let provider = Arc::new(ScriptedProvider::new(vec![
        tool_call_turn("call_1", "do_it", serde_json::json!({})),
        text_turn("done"),
    ]));
    let session = session_with(provider, tools);

    let got = collect(session.prompt(user_input("go"), CancelToken::new())).await;

    let tool_start = got
        .iter()
        .position(|e| matches!(e, AgentEvent::ToolStart { .. }))
        .expect("a ToolStart event");
    let tool_end = got
        .iter()
        .position(|e| matches!(e, AgentEvent::ToolEnd { .. }))
        .expect("a ToolEnd event");
    // The second turn must begin after the tool finishes.
    let second_turn_start = got
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, AgentEvent::TurnStart))
        .nth(1)
        .map(|(i, _)| i)
        .expect("a second TurnStart event");
    assert!(tool_start < tool_end);
    assert!(tool_end < second_turn_start);
}

#[tokio::test]
async fn agent_loop_appends_assistant_and_tool_messages() {
    // SPEC-01 names this test against the context. `Session` exposes no context
    // reader, so this asserts the same ordering through the observable event
    // stream: the assistant tool call comes first, then the tool result.
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(RecordingTool::new("do_it")));
    let provider = Arc::new(ScriptedProvider::new(vec![
        tool_call_turn("call_1", "do_it", serde_json::json!({})),
        text_turn("done"),
    ]));
    let session = session_with(provider, tools);

    let got = collect(session.prompt(user_input("go"), CancelToken::new())).await;

    let assistant_pos = got
        .iter()
        .position(|e| matches!(e, AgentEvent::Stream(StreamEvent::ToolCallEnd { .. })))
        .expect("the assistant tool call");
    let tool_result_pos = got
        .iter()
        .position(|e| matches!(e, AgentEvent::ToolEnd { .. }))
        .expect("the tool result");
    assert!(
        assistant_pos < tool_result_pos,
        "the assistant message is appended before the tool result"
    );
}

#[tokio::test]
async fn agent_loop_end_turn_maps_to_agent_stop_reason_end_turn() {
    let provider = Arc::new(ScriptedProvider::new(vec![text_turn("all done")]));
    let session = session_with(provider, ToolRegistry::new());

    let got = collect(session.prompt(user_input("hi"), CancelToken::new())).await;

    assert_eq!(
        got.last(),
        Some(&AgentEvent::AgentEnd {
            stop_reason: AgentStopReason::EndTurn
        })
    );
}

#[tokio::test]
async fn agent_loop_turn_cap_stops_with_max_turn_requests() {
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(RecordingTool::new("do_it")));
    // The provider always asks for the tool, so only the turn cap can stop it.
    let provider = Arc::new(ScriptedProvider::cycling(tool_call_turn(
        "call_1",
        "do_it",
        serde_json::json!({}),
    )));
    let session = session_with(provider, tools);

    let got = collect(session.prompt(user_input("loop"), CancelToken::new())).await;

    assert_eq!(
        got.last(),
        Some(&AgentEvent::AgentEnd {
            stop_reason: AgentStopReason::MaxTurnRequests
        })
    );
}

#[tokio::test]
async fn agent_loop_cancel_ends_with_canceled_stop_reason() {
    // A turn that never emits `Done`, so only cancellation can end the run.
    let stalling_turn = vec![StreamEvent::MessageStart {
        role: Role::Assistant,
    }];
    let provider = Arc::new(ScriptedProvider::new(vec![stalling_turn]));
    let session = session_with(provider, ToolRegistry::new());
    let cancel = CancelToken::new();

    let mut events = session.prompt(user_input("go"), cancel.clone());
    // Cancel from the test task. The loop must resolve the run.
    cancel.cancel();

    let mut got = Vec::new();
    while let Some(item) = events.next().await {
        got.push(item.expect("no error event"));
    }

    assert_eq!(
        got.last(),
        Some(&AgentEvent::AgentEnd {
            stop_reason: AgentStopReason::Canceled
        })
    );
    assert!(
        got.iter().any(|e| matches!(
            e,
            AgentEvent::TurnEnd {
                stop_reason: StopReason::Canceled
            }
        )),
        "the turn ends with the Canceled stop reason"
    );
}

#[tokio::test]
async fn agent_events_drop_aborts_driver_task() {
    // The scripted provider flags a dropped stream. Dropping `AgentEvents` must
    // abort the driver task, which drops the in-flight provider stream.
    let provider = Arc::new(ScriptedProvider::new(vec![text_turn("hi")]));
    let dropped_flag = Arc::clone(&provider.stream_dropped);
    let dropped_signal = Arc::clone(&provider.dropped);
    let session = session_with(provider, ToolRegistry::new());

    let mut events = session.prompt(user_input("go"), CancelToken::new());
    // Pull one event, so the driver task started the provider stream.
    let _first = events.next().await;
    drop(events);

    // The `notify_one` permit means this resolves even if the drop already ran.
    // No sleep is needed.
    dropped_signal.notified().await;
    assert!(
        dropped_flag.load(std::sync::atomic::Ordering::SeqCst),
        "dropping AgentEvents drops the provider stream"
    );
}
