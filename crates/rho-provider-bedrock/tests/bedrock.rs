//! Bedrock-specific tests, from `SPEC-02` section 5 and section 7.
//!
//! These drive the pure mapping functions `map_converse_event` and
//! `map_converse_error`. A test builds a `ConverseStream` event from recorded
//! JSON, then asserts the normalised output. No AWS client runs. No network
//! runs.

mod common;

use common::load_fixture;
use rho_core::{ProviderError, StopReason, StreamEvent, Usage};
use rho_provider_bedrock::{
    BedrockMapState, ConverseStreamEvent, map_converse_error, map_converse_event,
};

/// Parse one recorded event from JSON.
fn event(json: &str) -> ConverseStreamEvent {
    serde_json::from_str(json).expect("the event JSON must parse")
}

/// Map a list of events in order and collect the output.
fn map_all(events: Vec<ConverseStreamEvent>) -> Vec<StreamEvent> {
    let mut state = BedrockMapState::default();
    let mut out = Vec::new();
    for one in events {
        out.extend(map_converse_event(&mut state, one));
    }
    out
}

/// Map one event with a fresh state and collect the output.
fn map_one(json: &str) -> Vec<StreamEvent> {
    let mut state = BedrockMapState::default();
    map_converse_event(&mut state, event(json))
}

// --- Event kind coverage, from SPEC-02 section 5. ------------------------

#[tokio::test]
async fn provider_bedrock_maps_message_start() {
    // A messageStart opens the assistant message.
    let out = map_one(r#"{"messageStart":{"role":"assistant"}}"#);
    assert!(
        out.iter()
            .any(|e| matches!(e, StreamEvent::MessageStart { .. })),
        "messageStart must map to MessageStart, got {out:?}"
    );
}

#[tokio::test]
async fn provider_bedrock_maps_content_block_delta_text() {
    // A text contentBlockDelta becomes a TextDelta with the same text.
    let out = map_one(r#"{"contentBlockDelta":{"contentBlockIndex":0,"delta":{"text":"Hel"}}}"#);
    let found = out.iter().any(
        |e| matches!(e, StreamEvent::TextDelta { index, delta } if *index == 0 && delta == "Hel"),
    );
    assert!(found, "a text delta must map to TextDelta, got {out:?}");
}

#[tokio::test]
async fn provider_bedrock_maps_first_text_delta_starts_the_block() {
    // The first text delta for an index also opens the block with TextStart.
    let out = map_one(r#"{"contentBlockDelta":{"contentBlockIndex":0,"delta":{"text":"Hel"}}}"#);
    assert!(
        out.iter()
            .any(|e| matches!(e, StreamEvent::TextStart { index } if *index == 0)),
        "the first text delta must emit TextStart, got {out:?}"
    );
}

#[tokio::test]
async fn provider_bedrock_maps_tool_use_start() {
    // A contentBlockStart with a toolUse opens a tool call.
    let out = map_one(
        r#"{"contentBlockStart":{"contentBlockIndex":0,"start":{"toolUse":{"toolUseId":"tool_1","name":"get_weather"}}}}"#,
    );
    let found = out.iter().any(|e| {
        matches!(e, StreamEvent::ToolCallStart { index, id, name }
            if *index == 0 && id == "tool_1" && name == "get_weather")
    });
    assert!(
        found,
        "a toolUse start must map to ToolCallStart, got {out:?}"
    );
}

#[tokio::test]
async fn provider_bedrock_maps_reasoning_delta_to_thinking() {
    // A reasoningContent delta becomes a ThinkingDelta.
    let out = map_one(
        r#"{"contentBlockDelta":{"contentBlockIndex":0,"delta":{"reasoningContent":{"text":"hmm"}}}}"#,
    );
    let found = out
        .iter()
        .any(|e| matches!(e, StreamEvent::ThinkingDelta { delta, .. } if delta == "hmm"));
    assert!(
        found,
        "reasoningContent must map to ThinkingDelta, got {out:?}"
    );
}

#[tokio::test]
async fn provider_bedrock_content_block_stop_ends_text_block() {
    // A stop after a text delta closes the block with TextEnd.
    let out = map_all(vec![
        event(r#"{"contentBlockDelta":{"contentBlockIndex":0,"delta":{"text":"Hel"}}}"#),
        event(r#"{"contentBlockStop":{"contentBlockIndex":0}}"#),
    ]);
    assert!(
        out.iter()
            .any(|e| matches!(e, StreamEvent::TextEnd { index } if *index == 0)),
        "a text block stop must emit TextEnd, got {out:?}"
    );
}

// --- Tool use assembly, from SPEC-02 section 5. --------------------------

#[tokio::test]
async fn provider_bedrock_assembles_tool_use_input() {
    // The fixture splits the tool input across three deltas, split mid-token.
    // The stop must emit one ToolCallEnd with the full parsed object.
    let out = map_all(load_fixture("converse_tool_call.json"));
    let arguments = out
        .iter()
        .find_map(|e| match e {
            StreamEvent::ToolCallEnd { arguments, .. } => Some(arguments.clone()),
            _ => None,
        })
        .expect("a ToolCallEnd event");
    assert_eq!(
        arguments,
        serde_json::json!({ "city": "Paris", "unit": "celsius" }),
        "the split fragments must assemble into one parsed object"
    );
}

#[tokio::test]
async fn provider_bedrock_tool_use_input_is_parsed_object_not_string() {
    // ToolCallEnd must carry a parsed object, never the raw JSON string.
    let out = map_all(load_fixture("converse_tool_call.json"));
    let arguments = out
        .iter()
        .find_map(|e| match e {
            StreamEvent::ToolCallEnd { arguments, .. } => Some(arguments.clone()),
            _ => None,
        })
        .expect("a ToolCallEnd event");
    assert!(arguments.is_object(), "arguments must be a parsed object");
}

// --- Stop reason mapping, from SPEC-02 section 5. ------------------------
//
// A wrong stop-reason mapping is silent. The test asserts all six values.

/// Map a messageStop and return its StopReason.
fn stop_reason_of(reason: &str) -> StopReason {
    let json = format!(r#"{{"messageStop":{{"stopReason":"{reason}"}}}}"#);
    map_one(&json)
        .into_iter()
        .find_map(|e| match e {
            StreamEvent::Done { stop_reason } => Some(stop_reason),
            _ => None,
        })
        .expect("a Done event")
}

#[tokio::test]
async fn provider_bedrock_maps_stop_reason_tool_use() {
    assert_eq!(stop_reason_of("tool_use"), StopReason::ToolUse);
}

#[tokio::test]
async fn provider_bedrock_maps_all_stop_reasons() {
    assert_eq!(stop_reason_of("end_turn"), StopReason::EndTurn);
    assert_eq!(stop_reason_of("tool_use"), StopReason::ToolUse);
    assert_eq!(stop_reason_of("max_tokens"), StopReason::MaxTokens);
    assert_eq!(stop_reason_of("stop_sequence"), StopReason::StopSequence);
    // Both guardrail and content filter map to ContentFiltered.
    assert_eq!(
        stop_reason_of("guardrail_intervened"),
        StopReason::ContentFiltered
    );
    assert_eq!(
        stop_reason_of("content_filtered"),
        StopReason::ContentFiltered
    );
}

// --- Usage, from SPEC-02 section 5. --------------------------------------

#[tokio::test]
async fn provider_bedrock_reports_usage_with_cache_tokens() {
    // The prompt-cache story depends on the cache read and write counts.
    let out = map_one(
        r#"{"metadata":{"usage":{"inputTokens":12,"outputTokens":7,"cacheReadInputTokens":3,"cacheWriteInputTokens":4}}}"#,
    );
    let usage = out
        .iter()
        .find_map(|e| match e {
            StreamEvent::Usage(usage) => Some(*usage),
            _ => None,
        })
        .expect("a Usage event");
    assert_eq!(
        usage,
        Usage {
            input_tokens: 12,
            output_tokens: 7,
            cache_read_tokens: 3,
            cache_write_tokens: 4,
        }
    );
}

// --- Error mapping, from SPEC-02 section 5. ------------------------------

#[tokio::test]
async fn provider_bedrock_maps_throttling_to_rate_limited() {
    // Throttling is retryable.
    let error = map_converse_error("throttlingException");
    assert!(
        matches!(error, ProviderError::RateLimited { .. }),
        "throttlingException must map to RateLimited, got {error:?}"
    );
    assert!(error.is_retryable(), "a throttling error must be retryable");
}

#[tokio::test]
async fn provider_bedrock_maps_validation_to_client_not_retryable() {
    // A validation error is a permanent 4xx. A retry would waste a request.
    let error = map_converse_error("validationException");
    assert!(
        matches!(error, ProviderError::Client { status: 400, .. }),
        "validationException must map to Client 400, got {error:?}"
    );
    assert!(
        !error.is_retryable(),
        "a validation error must not be retryable"
    );
}

#[tokio::test]
async fn provider_bedrock_midstream_error_maps_to_server_error() {
    // A model stream error surfaces as an error, not a silent truncation.
    let error = map_converse_error("modelStreamErrorException");
    assert!(
        matches!(error, ProviderError::Server { .. }),
        "modelStreamErrorException must map to Server, got {error:?}"
    );
}

#[tokio::test]
async fn provider_bedrock_maps_service_faults_to_server_error() {
    for name in ["serviceUnavailableException", "internalServerException"] {
        let error = map_converse_error(name);
        assert!(
            matches!(error, ProviderError::Server { .. }),
            "{name} must map to Server, got {error:?}"
        );
        assert!(error.is_retryable(), "{name} must be retryable");
    }
}
