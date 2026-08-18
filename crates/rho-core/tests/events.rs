//! Tests for the streaming event and stop-reason wire shapes.

use rho_core::{AgentStopReason, Role, StopReason, StreamEvent, Usage};

fn kind_of(event: &StreamEvent) -> String {
    let json = serde_json::to_value(event).unwrap();
    json["kind"].as_str().unwrap().to_string()
}

#[test]
fn stream_event_tags_are_snake_case() {
    let cases = [
        (
            StreamEvent::MessageStart {
                role: Role::Assistant,
            },
            "message_start",
        ),
        (StreamEvent::TextStart { index: 0 }, "text_start"),
        (
            StreamEvent::TextDelta {
                index: 0,
                delta: "x".to_string(),
            },
            "text_delta",
        ),
        (StreamEvent::TextEnd { index: 0 }, "text_end"),
        (StreamEvent::ThinkingStart { index: 0 }, "thinking_start"),
        (
            StreamEvent::ThinkingDelta {
                index: 0,
                delta: "x".to_string(),
            },
            "thinking_delta",
        ),
        (
            StreamEvent::ThinkingEnd {
                index: 0,
                signature: None,
            },
            "thinking_end",
        ),
        (
            StreamEvent::ToolCallStart {
                index: 0,
                id: "1".to_string(),
                name: "read".to_string(),
            },
            "tool_call_start",
        ),
        (
            StreamEvent::ToolCallDelta {
                index: 0,
                delta: "{".to_string(),
            },
            "tool_call_delta",
        ),
        (
            StreamEvent::ToolCallEnd {
                index: 0,
                arguments: serde_json::json!({}),
            },
            "tool_call_end",
        ),
        (StreamEvent::Usage(Usage::default()), "usage"),
        (
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
            "done",
        ),
    ];
    for (event, expected) in cases {
        assert_eq!(kind_of(&event), expected);
    }
}

#[test]
fn stop_reason_serialises_snake_case() {
    let cases = [
        (StopReason::EndTurn, "end_turn"),
        (StopReason::ToolUse, "tool_use"),
        (StopReason::MaxTokens, "max_tokens"),
        (StopReason::StopSequence, "stop_sequence"),
        (StopReason::ContentFiltered, "content_filtered"),
        (StopReason::Canceled, "canceled"),
    ];
    for (reason, expected) in cases {
        assert_eq!(serde_json::to_value(reason).unwrap(), expected);
    }
}

#[test]
fn agent_stop_reason_serialises_snake_case() {
    let cases = [
        (AgentStopReason::EndTurn, "end_turn"),
        (AgentStopReason::MaxTokens, "max_tokens"),
        (AgentStopReason::MaxTurnRequests, "max_turn_requests"),
        (AgentStopReason::Refusal, "refusal"),
    ];
    for (reason, expected) in cases {
        assert_eq!(serde_json::to_value(reason).unwrap(), expected);
    }
}

/// Guards decision D-acp-cancelled-spelling. ACP spells the cancelled stop reason with two letters
/// `l`, as `cancelled`. The Rust variant is `Canceled` with one `l`. The
/// `serde(rename)` attribute must emit the ACP spelling. No ACP client accepts
/// `canceled`. Do not remove this test.
#[test]
fn agent_stop_reason_canceled_serialises_as_cancelled() {
    let json = serde_json::to_value(AgentStopReason::Canceled).unwrap();
    assert_eq!(json, "cancelled");
    // Prove it round-trips from the ACP wire spelling too.
    let back: AgentStopReason = serde_json::from_value(serde_json::json!("cancelled")).unwrap();
    assert_eq!(back, AgentStopReason::Canceled);
}
