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
    // SPEC-core-runtime names this test against the context. `Session` exposes no context
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
async fn agent_loop_cancel_before_first_turn_pairs_turn_start_and_end() {
    // A cancel that lands before any turn starts must not emit an unpaired
    // `TurnEnd`. A frontend pairs `TurnStart` with `TurnEnd`, so an unpaired
    // `TurnEnd` mis-renders or panics. `rho-tui` and `rho-acp` both rely on this.
    let provider = Arc::new(ScriptedProvider::new(vec![text_turn("never reached")]));
    let session = session_with(provider, ToolRegistry::new());

    // The token is already cancelled, so the loop stops before it runs a turn.
    let cancel = CancelToken::new();
    cancel.cancel();
    let got = collect(session.prompt(user_input("go"), cancel)).await;

    let turn_end = got
        .iter()
        .position(|e| matches!(e, AgentEvent::TurnEnd { .. }))
        .expect("a TurnEnd event");
    let turn_start = got
        .iter()
        .position(|e| matches!(e, AgentEvent::TurnStart))
        .expect("a TurnStart event must precede every TurnEnd");
    assert!(
        turn_start < turn_end,
        "a TurnStart must precede the TurnEnd, so the pair is balanced"
    );
    assert_eq!(
        got.last(),
        Some(&AgentEvent::AgentEnd {
            stop_reason: AgentStopReason::Canceled
        })
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

#[tokio::test]
async fn agent_loop_tool_error_returns_to_the_model_and_the_run_continues() {
    // A live smoke test found this bug. A tool that returned `Err` aborted the whole
    // run, and the model never saw the failure.
    //
    // That makes the harness unusable in practice. Almost every real session has a
    // tool error: a file is missing, a grep matches nothing, a command exits non-zero.
    // A coding agent must report the failure to the model, so the model can try
    // something else. Only a transport or provider fault ends a run.
    let tool = common::FailingTool::new("read", "no such file");
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(tool));

    let provider: Arc<dyn Provider> = Arc::new(common::ScriptedProvider::new(vec![
        common::tool_call_turn(
            "call_1",
            "read",
            serde_json::json!({ "path": "missing.txt" }),
        ),
        common::text_turn("the file was missing, so I stopped"),
    ]));
    let session = session_with(provider, registry);
    let events = collect(session.prompt(
        vec![ContentBlock::Text {
            text: "read missing.txt".to_string(),
        }],
        CancelToken::new(),
    ))
    .await;

    // The run must reach a normal end, not an error.
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentEvent::AgentEnd {
                stop_reason: AgentStopReason::EndTurn
            }
        )),
        "the run must end normally, got {events:?}"
    );

    // The tool result must be marked as an error and must carry the reason, so the
    // model can act on it.
    let messages = session.messages().await;
    let tool_results: Vec<_> = messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|block| match block {
            ContentBlock::ToolResult {
                content, is_error, ..
            } => Some((content, is_error)),
            _ => None,
        })
        .collect();
    assert_eq!(tool_results.len(), 1, "one tool result must be recorded");
    let (content, is_error) = tool_results[0];
    assert!(*is_error, "a failed tool must produce an error result");
    let text = format!("{content:?}");
    assert!(
        text.contains("no such file"),
        "the result must carry the reason, got {text}"
    );
}

#[tokio::test]
async fn agent_loop_pairs_every_turn_start_with_a_turn_end() {
    // A live smoke test found this bug. A turn that ended in a tool call returned
    // without emitting `TurnEnd`, so `TurnStart` events were unpaired.
    //
    // Every other exit path emitted `TurnEnd`, which is why the gap survived. The
    // existing tool test asserted `ToolStart`, `ToolEnd`, and a second `TurnStart`,
    // and never checked that the first turn closed.
    //
    // A frontend pairs these two events to track state. `rho-tui` uses the pair to
    // decide whether the agent is running, and `rho-acp` maps it onto a session
    // update. An unpaired start leaks that state forever.
    //
    // This test states the invariant for the whole run, so any future exit path is
    // covered too.
    let tool = common::RecordingTool::new("probe");
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(tool));

    let provider: Arc<dyn Provider> = Arc::new(common::ScriptedProvider::new(vec![
        common::tool_call_turn("call_1", "probe", serde_json::json!({})),
        common::tool_call_turn("call_2", "probe", serde_json::json!({})),
        common::text_turn("done"),
    ]));
    let session = session_with(provider, registry);
    let events = collect(session.prompt(
        vec![ContentBlock::Text {
            text: "use the tool twice".to_string(),
        }],
        CancelToken::new(),
    ))
    .await;

    let starts = events
        .iter()
        .filter(|event| matches!(event, AgentEvent::TurnStart))
        .count();
    let ends = events
        .iter()
        .filter(|event| matches!(event, AgentEvent::TurnEnd { .. }))
        .count();
    assert_eq!(
        starts, 3,
        "three turns must start: two tool turns and the final answer"
    );
    assert_eq!(
        ends, starts,
        "every TurnStart needs a TurnEnd, got {starts} starts and {ends} ends"
    );

    // The pairing must also be ordered. A start always precedes its end, and no
    // second start arrives before the first end.
    let mut open = 0i32;
    for event in &events {
        match event {
            AgentEvent::TurnStart => {
                open += 1;
                assert_eq!(open, 1, "a turn started while another was still open");
            }
            AgentEvent::TurnEnd { .. } => {
                open -= 1;
                assert_eq!(open, 0, "a turn ended without a matching start");
            }
            _ => {}
        }
    }
    assert_eq!(open, 0, "a turn was left open at the end of the run");
}
