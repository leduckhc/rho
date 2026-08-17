//! Tests for public items that no other test touched.
//!
//! A reviewer read every public item in `rho-core` and listed the ones with no
//! coverage. That list mattered, because the three worst defects in this crate all
//! hid in untested public surface: `confine` was left unimplemented, `Session::new`
//! carried an insecure default, and the approval boundary failed open on an
//! undeclared tool kind. A green suite proved nothing about any of them.
//!
//! So this file closes the list.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use rho_core::{
    AgentEvent, AgentStopReason, AllowAllPolicy, CancelToken, ContentBlock, Context, Provider,
    ProviderError, Session, SessionConfig, StopReason, ToolRegistry, ToolSpec,
};

fn config() -> SessionConfig {
    common::test_config()
}

// --- SessionConfig ---------------------------------------------------------

#[test]
fn session_config_for_current_dir_uses_the_working_directory() {
    let config = SessionConfig::for_current_dir("model-x", Arc::new(AllowAllPolicy))
        .expect("the working directory must be readable");
    assert_eq!(
        config.session_root,
        std::env::current_dir().expect("a working directory")
    );
    assert_eq!(config.model, "model-x");
}

#[test]
fn session_config_with_max_turns_overrides_the_default() {
    let default_turns = config().max_turns;
    let config = config().with_max_turns(3);
    assert_eq!(config.max_turns, 3);
    assert_ne!(
        default_turns, 3,
        "pick a value that differs from the default, or the test proves nothing"
    );
}

#[tokio::test]
async fn agent_loop_honours_a_max_turns_override() {
    // The existing turn-cap test relies on the default of 32, so the setter itself
    // was never proven to reach the loop.
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(common::RecordingTool::new("probe")));
    let provider: Arc<dyn Provider> = Arc::new(common::ScriptedProvider::cycling(
        common::tool_call_turn("call_1", "probe", serde_json::json!({})),
    ));
    let session = Session::with_config(
        config().with_max_turns(2),
        provider,
        Arc::new(registry),
        Arc::new(rho_core::HookChain::new()),
        Context::new(None, Vec::new()),
    );
    let events = drain(session.prompt(
        vec![ContentBlock::Text {
            text: "loop".to_string(),
        }],
        CancelToken::new(),
    ))
    .await;

    let starts = events
        .iter()
        .filter(|event| matches!(event, AgentEvent::TurnStart))
        .count();
    assert_eq!(starts, 2, "the loop must stop at the override, not at 32");
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::AgentEnd {
            stop_reason: AgentStopReason::MaxTurnRequests
        }
    )));
}

// --- Context --------------------------------------------------------------

#[test]
fn context_tools_returns_the_registered_specs() {
    let specs = vec![ToolSpec {
        name: "read".to_string(),
        description: "Read a file.".to_string(),
        kind: rho_core::ToolKind::Read,
        input_schema: serde_json::json!({ "type": "object" }),
    }];
    let context = Context::new(None, specs);
    assert_eq!(context.tools().len(), 1);
    assert_eq!(context.tools()[0].name, "read");
}

// --- ProviderError --------------------------------------------------------

#[test]
fn provider_error_server_is_retryable() {
    // Transport, Client, and RateLimited were covered. Server was not, and a 5xx is
    // the most common retryable failure in practice.
    assert!(ProviderError::Server { status: 500 }.is_retryable());
    assert!(ProviderError::Server { status: 503 }.is_retryable());
}

#[test]
fn provider_error_decode_and_auth_are_not_retryable() {
    assert!(!ProviderError::Decode("bad json".into()).is_retryable());
    assert!(!ProviderError::Auth("no credentials".into()).is_retryable());
}

// --- Stop reason mapping through the real loop ----------------------------

#[tokio::test]
async fn agent_loop_maps_content_filtered_to_refusal() {
    // Only the serde form was tested. This drives the mapping through the loop.
    let stop = run_to_stop(StopReason::ContentFiltered).await;
    assert_eq!(stop, AgentStopReason::Refusal);
}

#[tokio::test]
async fn agent_loop_maps_max_tokens_through_the_loop() {
    let stop = run_to_stop(StopReason::MaxTokens).await;
    assert_eq!(stop, AgentStopReason::MaxTokens);
}

#[tokio::test]
async fn agent_loop_maps_stop_sequence_to_end_turn() {
    let stop = run_to_stop(StopReason::StopSequence).await;
    assert_eq!(stop, AgentStopReason::EndTurn);
}

// --- Unregistered tool ----------------------------------------------------

#[tokio::test]
async fn agent_loop_reports_an_unregistered_tool_and_continues() {
    // A model may invent a tool name. That must be a result the model can read, not
    // the end of the run.
    let provider: Arc<dyn Provider> = Arc::new(common::ScriptedProvider::new(vec![
        common::tool_call_turn("call_1", "no_such_tool", serde_json::json!({})),
        common::text_turn("I used the wrong name"),
    ]));
    let session = Session::with_config(
        config(),
        provider,
        Arc::new(ToolRegistry::new()),
        Arc::new(rho_core::HookChain::new()),
        Context::new(None, Vec::new()),
    );
    let events = drain(session.prompt(
        vec![ContentBlock::Text {
            text: "go".to_string(),
        }],
        CancelToken::new(),
    ))
    .await;

    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentEvent::AgentEnd {
                stop_reason: AgentStopReason::EndTurn
            }
        )),
        "the run must continue past an unknown tool"
    );

    let messages = session.messages().await;
    let found = messages
        .iter()
        .flat_map(|message| message.content.iter())
        .any(|block| match block {
            ContentBlock::ToolResult {
                content, is_error, ..
            } => *is_error && format!("{content:?}").contains("not registered"),
            _ => false,
        });
    assert!(found, "the model must be told the tool is not registered");
}

// --- Streamed tool output -------------------------------------------------

#[tokio::test]
async fn agent_loop_forwards_streamed_tool_output_in_order() {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(common::UpdatingTool::new(&[
        "line one",
        "line two",
        "line three",
    ])));
    let provider: Arc<dyn Provider> = Arc::new(common::ScriptedProvider::new(vec![
        common::tool_call_turn("call_1", "streamer", serde_json::json!({})),
        common::text_turn("done"),
    ]));
    let session = Session::with_config(
        config(),
        provider,
        Arc::new(registry),
        Arc::new(rho_core::HookChain::new()),
        Context::new(None, Vec::new()),
    );
    let events = drain(session.prompt(
        vec![ContentBlock::Text {
            text: "stream".to_string(),
        }],
        CancelToken::new(),
    ))
    .await;

    let lines: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ToolUpdate { output, .. } => Some(output.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(lines, vec!["line one", "line two", "line three"]);
}

// --- Dropping the event stream mid-tool -----------------------------------

#[tokio::test]
async fn dropping_events_drops_a_tool_future_in_flight() {
    // A tool that is still running must be dropped when the caller walks away, or a
    // long command outlives its session.
    //
    // The first version of this test was vacuous. It dropped the event stream and then
    // asserted that the tool had not completed. But the tool was waiting on a signal
    // that never fired, so it could never complete either way. The test passed against
    // a deliberately emptied `Drop` impl, which means it proved nothing.
    //
    // This version discriminates. After dropping the stream, it releases the tool. A
    // live future would wake and set the flag. A dropped future has no waiter, so the
    // release reaches nobody and the flag stays false.
    let tool = common::BlockingTool::new();
    let completed = tool.completed.clone();
    let started = tool.started.clone();
    let release = tool.release.clone();
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(tool));

    let provider: Arc<dyn Provider> = Arc::new(common::ScriptedProvider::new(vec![
        common::tool_call_turn("call_1", "blocker", serde_json::json!({})),
        common::text_turn("unreachable"),
    ]));
    let session = Session::with_config(
        config(),
        provider,
        Arc::new(registry),
        Arc::new(rho_core::HookChain::new()),
        Context::new(None, Vec::new()),
    );

    let notified = started.notified();
    let mut events = session.prompt(
        vec![ContentBlock::Text {
            text: "block".to_string(),
        }],
        CancelToken::new(),
    );

    // Pump events until the tool reports that it started. No sleep is involved.
    let pump = async {
        use futures::StreamExt;
        while let Some(item) = events.next().await {
            if let Ok(AgentEvent::ToolStart { .. }) = item {
                break;
            }
        }
    };
    tokio::join!(pump, notified);

    // Walk away while the tool is still waiting.
    drop(events);

    // Now release the tool. This is the step that makes the test discriminate.
    for _ in 0..64 {
        release.notify_waiters();
        tokio::task::yield_now().await;
        if completed.load(Ordering::SeqCst) {
            break;
        }
    }

    assert!(
        !completed.load(Ordering::SeqCst),
        "the tool future must be dropped. It completed, so the run outlived its caller"
    );
}

// --- helpers --------------------------------------------------------------

async fn run_to_stop(reason: StopReason) -> AgentStopReason {
    let provider: Arc<dyn Provider> = Arc::new(common::ScriptedProvider::new(vec![
        common::turn_ending_with(reason),
    ]));
    let session = Session::with_config(
        config(),
        provider,
        Arc::new(ToolRegistry::new()),
        Arc::new(rho_core::HookChain::new()),
        Context::new(None, Vec::new()),
    );
    let events = drain(session.prompt(
        vec![ContentBlock::Text {
            text: "go".to_string(),
        }],
        CancelToken::new(),
    ))
    .await;
    events
        .iter()
        .find_map(|event| match event {
            AgentEvent::AgentEnd { stop_reason } => Some(*stop_reason),
            _ => None,
        })
        .expect("the run must emit AgentEnd")
}

async fn drain(mut events: rho_core::AgentEvents) -> Vec<AgentEvent> {
    use futures::StreamExt;
    let mut out = Vec::new();
    while let Some(item) = events.next().await {
        if let Ok(event) = item {
            out.push(event);
        }
    }
    out
}
