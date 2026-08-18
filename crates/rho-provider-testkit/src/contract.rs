//! The shared provider contract, from `SPEC-provider-interface` section 7.
//!
//! Each function drives a provider through one [`Script`] and asserts one rule.
//! An outside author calls [`run_all`] to run every check, or calls one check.
//! Every function bounds its wait with a timeout, so a stuck provider fails the
//! test instead of hanging the suite.

use crate::harness::{HarnessRun, ProviderHarness};
use crate::script::{SCRIPT_TEXT, Script, script_tool_arguments};
use futures::StreamExt;
use rho_core::StreamEvent;
use std::time::Duration;
use tokio::time::timeout;

/// The longest wait for one event. A stuck provider trips this and fails.
const EVENT_TIMEOUT: Duration = Duration::from_secs(5);

/// Read every event from the run. Fail on a stream error or a timeout.
async fn collect(run: HarnessRun) -> Vec<StreamEvent> {
    let HarnessRun { mut stream, guard } = run;
    let mut events = Vec::new();
    loop {
        match timeout(EVENT_TIMEOUT, stream.next()).await {
            Ok(Some(Ok(event))) => events.push(event),
            Ok(Some(Err(error))) => panic!("the provider stream returned an error: {error}"),
            Ok(None) => break,
            Err(_) => panic!("the provider stream did not end within the timeout"),
        }
    }
    drop(guard);
    events
}

/// Every provider emits `MessageStart` as its first event.
pub async fn provider_contract_emits_message_start_first(harness: &dyn ProviderHarness) {
    let run = harness.run(Script::Text).await;
    let events = collect(run).await;
    let first = events.first().expect("the stream produced no event");
    assert!(
        matches!(first, StreamEvent::MessageStart { .. }),
        "the first event must be MessageStart, but it was {first:?}"
    );
}

/// Every provider ends a good turn with `Done`.
pub async fn provider_contract_emits_done_last(harness: &dyn ProviderHarness) {
    let run = harness.run(Script::Text).await;
    let events = collect(run).await;
    let last = events.last().expect("the stream produced no event");
    assert!(
        matches!(last, StreamEvent::Done { .. }),
        "the last event must be Done, but it was {last:?}"
    );
}

/// A provider yields its first event while the transport still holds the tail.
///
/// This proves the provider streams. A provider that buffers the whole body
/// blocks until the tail arrives, so the bounded wait fails it.
pub async fn provider_contract_yields_first_event_before_stream_end(harness: &dyn ProviderHarness) {
    let HarnessRun { mut stream, guard } = harness.run(Script::Gated).await;
    // The transport holds the tail far longer than this wait. A streaming
    // provider still yields the first event fast.
    let first = timeout(Duration::from_secs(2), stream.next()).await;
    match first {
        Ok(Some(Ok(event))) => assert!(
            matches!(
                event,
                StreamEvent::MessageStart { .. } | StreamEvent::TextStart { .. }
            ),
            "the first streamed event was unexpected: {event:?}"
        ),
        Ok(Some(Err(error))) => panic!("the provider stream returned an error: {error}"),
        Ok(None) => panic!("the provider stream ended before it yielded an event"),
        Err(_) => panic!("the provider buffered the body; no event arrived before the tail"),
    }
    // Drop the stream to cancel the run and release the held transport.
    drop(stream);
    drop(guard);
}

/// Text deltas arrive in index order and reassemble to the scripted text.
pub async fn provider_contract_text_deltas_in_order(harness: &dyn ProviderHarness) {
    let run = harness.run(Script::Text).await;
    let events = collect(run).await;
    let mut text = String::new();
    let mut last_index: Option<u32> = None;
    for event in &events {
        if let StreamEvent::TextDelta { index, delta } = event {
            if let Some(previous) = last_index {
                assert!(*index >= previous, "text delta index went backwards");
            }
            last_index = Some(*index);
            text.push_str(delta);
        }
    }
    assert_eq!(
        text, SCRIPT_TEXT,
        "the reassembled text did not match the script"
    );
}

/// `ToolCallEnd` carries a parsed JSON object, not a string.
pub async fn provider_contract_tool_call_end_has_parsed_arguments(harness: &dyn ProviderHarness) {
    let run = harness.run(Script::ToolCall).await;
    let events = collect(run).await;
    let end = events
        .iter()
        .find_map(|event| match event {
            StreamEvent::ToolCallEnd { arguments, .. } => Some(arguments),
            _ => None,
        })
        .expect("the stream produced no ToolCallEnd event");
    assert!(
        end.is_object(),
        "ToolCallEnd arguments must be a parsed object, but they were {end:?}"
    );
    assert_eq!(
        end,
        &script_tool_arguments(),
        "the assembled tool arguments did not match the script"
    );
}

/// Run every contract check against one provider. An outside author calls this.
pub async fn run_all(harness: &dyn ProviderHarness) {
    provider_contract_emits_message_start_first(harness).await;
    provider_contract_emits_done_last(harness).await;
    provider_contract_yields_first_event_before_stream_end(harness).await;
    provider_contract_text_deltas_in_order(harness).await;
    provider_contract_tool_call_end_has_parsed_arguments(harness).await;
}
