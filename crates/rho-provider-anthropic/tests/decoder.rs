//! Map Anthropic's SSE stream onto `StreamEvent`.
//!
//! Each test feeds a scripted list of `(event_name, json_data)` pairs into the decoder and
//! asserts the sequence of `StreamEvent` it emits. The decoder is stateful, because a
//! `content_block_delta` alone does not know whether its content is text, a tool call, or
//! reasoning; the earlier `content_block_start` decided that.
//!
//! Every test drives the real decoder. No test asserts against a helper. This matches the
//! rule the launch amendment on `SPEC-the-tool-row-has-three-levels` sets, and it stops the
//! `concise.rs` scaffolding defect from happening here.

use rho_core::{Role, StopReason, StreamEvent};
use rho_provider_anthropic::sse::Decoder;
use serde_json::json;

/// A shorthand: feed the whole script through a fresh decoder, and return every event it
/// emitted. Panics on a `Decode` error, because these tests are the happy path.
fn play(script: &[(&str, serde_json::Value)]) -> Vec<StreamEvent> {
    let mut decoder = Decoder::new();
    let mut out = Vec::new();
    for (name, data) in script {
        for event in decoder
            .on_event(name, data)
            .expect("decoder rejects a known event")
        {
            out.push(event);
        }
    }
    out
}

#[test]
fn a_message_start_yields_a_message_start_event() {
    let events = play(&[(
        "message_start",
        json!({"type": "message_start", "message": {"role": "assistant"}}),
    )]);
    assert_eq!(
        events,
        vec![StreamEvent::MessageStart {
            role: Role::Assistant
        }]
    );
}

#[test]
fn a_text_block_streams_a_start_a_delta_and_an_end() {
    let events = play(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {"role": "assistant"}}),
        ),
        (
            "content_block_start",
            json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": {"type": "text", "text": ""}
            }),
        ),
        (
            "content_block_delta",
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "text_delta", "text": "hello"}
            }),
        ),
        (
            "content_block_delta",
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "text_delta", "text": " world"}
            }),
        ),
        (
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 0}),
        ),
    ]);
    assert_eq!(
        events,
        vec![
            StreamEvent::MessageStart {
                role: Role::Assistant
            },
            StreamEvent::TextStart { index: 0 },
            StreamEvent::TextDelta {
                index: 0,
                delta: "hello".into()
            },
            StreamEvent::TextDelta {
                index: 0,
                delta: " world".into()
            },
            StreamEvent::TextEnd { index: 0 },
        ]
    );
}

#[test]
fn message_stop_yields_done_with_the_end_turn_stop_reason() {
    // Anthropic sends the stop reason on `message_delta`, then `message_stop` closes the
    // stream. rho emits one `Done` at the close, carrying the last known reason.
    let events = play(&[
        (
            "message_delta",
            json!({
                "type": "message_delta",
                "delta": {"stop_reason": "end_turn", "stop_sequence": null},
                "usage": {"output_tokens": 3}
            }),
        ),
        ("message_stop", json!({"type": "message_stop"})),
    ]);
    assert_eq!(
        events.last(),
        Some(&StreamEvent::Done {
            stop_reason: StopReason::EndTurn
        }),
        "the last event is Done with the reported stop reason: {events:?}"
    );
}

#[test]
fn a_ping_event_is_silent() {
    // Ping is a framing keepalive. It reaches no StreamEvent.
    let events = play(&[("ping", json!({"type": "ping"}))]);
    assert!(
        events.is_empty(),
        "ping produces no StreamEvent: {events:?}"
    );
}

#[test]
fn an_unknown_content_bearing_event_fails_decode() {
    // The launch amendment forbids swallowing an unknown content-bearing event. rho does
    // not guess whether a new event carries content, so an unknown one fails Decode.
    let mut decoder = Decoder::new();
    let result = decoder.on_event(
        "content_block_shocker",
        &json!({"type": "content_block_shocker"}),
    );
    assert!(
        result.is_err(),
        "an unknown content event must fail: {result:?}"
    );
}
