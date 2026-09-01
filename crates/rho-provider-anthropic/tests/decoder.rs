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

// ---- tool_use ----

#[test]
fn a_tool_use_block_streams_start_then_json_deltas_then_end_with_parsed_arguments() {
    let events = play(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {"role": "assistant"}}),
        ),
        (
            "content_block_start",
            json!({
                "type": "content_block_start",
                "index": 1,
                "content_block": {"type": "tool_use", "id": "call-1", "name": "read", "input": {}}
            }),
        ),
        (
            "content_block_delta",
            json!({
                "type": "content_block_delta",
                "index": 1,
                "delta": {"type": "input_json_delta", "partial_json": "{\"path\""}
            }),
        ),
        (
            "content_block_delta",
            json!({
                "type": "content_block_delta",
                "index": 1,
                "delta": {"type": "input_json_delta", "partial_json": ":\"fact.txt\"}"}
            }),
        ),
        (
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 1}),
        ),
    ]);

    // The start event carries the id and name from the wire.
    assert_eq!(
        events[1],
        StreamEvent::ToolCallStart {
            index: 1,
            id: "call-1".into(),
            name: "read".into(),
        }
    );

    // Every partial_json fragment reaches the consumer as ToolCallDelta, in order.
    let deltas: Vec<&StreamEvent> = events
        .iter()
        .filter(|e| matches!(e, StreamEvent::ToolCallDelta { .. }))
        .collect();
    assert_eq!(deltas.len(), 2);

    // On stop, the arguments are parsed as JSON. The state stays None because Anthropic does
    // not attach a signature to a tool call.
    let last = events.last().expect("at least one event");
    match last {
        StreamEvent::ToolCallEnd {
            index,
            arguments,
            state,
        } => {
            assert_eq!(*index, 1);
            assert_eq!(arguments, &json!({"path": "fact.txt"}));
            assert!(state.is_none(), "no state for anthropic tool calls");
        }
        other => panic!("last event must be ToolCallEnd, got {other:?}"),
    }
}

#[test]
fn a_tool_use_input_over_the_cap_fails_decode() {
    // The spec caps the accumulated input JSON at 1 MiB. A hostile stream that keeps
    // pushing partial_json must fail Decode, not run out of memory. This test writes
    // slightly over 1 MiB and asserts a Decode error.
    use rho_provider_anthropic::sse::Decoder;
    let mut decoder = Decoder::new();
    let start = json!({
        "type": "content_block_start",
        "index": 0,
        "content_block": {"type": "tool_use", "id": "big", "name": "read", "input": {}}
    });
    decoder
        .on_event("content_block_start", &start)
        .expect("start opens the block");

    // 1 MiB + 1 byte total.
    let big = "x".repeat(1024 * 1024 + 1);
    let delta = json!({
        "type": "content_block_delta",
        "index": 0,
        "delta": {"type": "input_json_delta", "partial_json": big}
    });
    let outcome = decoder.on_event("content_block_delta", &delta);
    assert!(
        outcome.is_err(),
        "a partial_json over the cap must fail Decode: {outcome:?}"
    );
}
