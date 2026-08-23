//! Bedrock-specific tests, from `SPEC-provider-interface` section 5 and section 7.
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

/// A real Bedrock id that supports extended thinking. A message builder needs a model,
/// because a stored reasoning payload may only travel back to the model that made it.
const THINKING_MODEL: &str = "us.anthropic.claude-haiku-4-5-20251001-v1:0";

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

// --- Event kind coverage, from SPEC-provider-interface section 5. ------------------------

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

// --- Tool use assembly, from SPEC-provider-interface section 5. --------------------------

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

// --- Stop reason mapping, from SPEC-provider-interface section 5. ------------------------
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

// --- Usage, from SPEC-provider-interface section 5. --------------------------------------

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
            // Bedrock reports no charge on the stream. rho leaves it empty rather than
            // estimating from a price table. See decision D-measured-cost-and-cache.
            cost_usd: None,
        }
    );
}

// --- Error mapping, from SPEC-provider-interface section 5. ------------------------------

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

// --- Request shape, from a live defect -------------------------------------

/// Build a conversation where the model made two tool calls in one turn.
///
/// `rho-core` records one `Role::Tool` message per tool result, which is correct for
/// its own model. Bedrock has no tool role, so both map to `user`.
fn two_tool_results_conversation() -> Vec<rho_core::Message> {
    use rho_core::{ContentBlock, Message, Role};
    vec![
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: "read both files".to_string(),
            }],
        },
        Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::ToolCall {
                    id: "call_1".to_string(),
                    name: "read".to_string(),
                    arguments: serde_json::json!({ "path": "a.txt" }),
                    state: None,
                },
                ContentBlock::ToolCall {
                    id: "call_2".to_string(),
                    name: "read".to_string(),
                    arguments: serde_json::json!({ "path": "b.txt" }),
                    state: None,
                },
            ],
        },
        Message {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                tool_call_id: "call_1".to_string(),
                content: vec![ContentBlock::Text {
                    text: "alpha".to_string(),
                }],
                is_error: false,
            }],
        },
        Message {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                tool_call_id: "call_2".to_string(),
                content: vec![ContentBlock::Text {
                    text: "beta".to_string(),
                }],
                is_error: false,
            }],
        },
    ]
}

#[test]
fn build_messages_never_emits_two_messages_with_the_same_role_in_a_row() {
    // A live run against Bedrock found this. The model made two tool calls in one
    // turn, and Bedrock answered 400.
    //
    // Converse requires strictly alternating roles. `rho-core` records one Tool
    // message per result, and Bedrock has no tool role, so two results became two
    // consecutive user messages. One tool call worked, which is why the unit tests
    // and the first live check both passed.
    //
    // The recorded fixtures could never catch this, because they describe responses.
    // This defect is in the request.
    use aws_sdk_bedrockruntime::types::ConversationRole;

    let built = rho_provider_bedrock::build_messages_for_model(
        &two_tool_results_conversation(),
        THINKING_MODEL,
    );
    let roles: Vec<&ConversationRole> = built.iter().map(|message| message.role()).collect();
    for pair in roles.windows(2) {
        assert_ne!(
            pair[0], pair[1],
            "Bedrock rejects two messages with the same role in a row, got {roles:?}"
        );
    }
}

#[test]
fn build_messages_merges_tool_results_into_one_user_message() {
    // The merge must keep both results, in order, in a single user message. Dropping
    // one would lose a tool result in silence, which is worse than the 400.
    use aws_sdk_bedrockruntime::types::{ContentBlock as SdkBlock, ConversationRole};

    let built = rho_provider_bedrock::build_messages_for_model(
        &two_tool_results_conversation(),
        THINKING_MODEL,
    );
    assert_eq!(
        built.len(),
        3,
        "user, assistant, then one merged user message"
    );
    assert_eq!(built[2].role(), &ConversationRole::User);

    let ids: Vec<&str> = built[2]
        .content()
        .iter()
        .filter_map(|block| match block {
            SdkBlock::ToolResult(result) => Some(result.tool_use_id()),
            _ => None,
        })
        .collect();
    assert_eq!(
        ids,
        vec!["call_1", "call_2"],
        "both tool results must survive the merge, in order"
    );
}

#[test]
fn build_messages_keeps_a_single_tool_result_working() {
    // The single-call path already worked live. Do not regress it.
    use aws_sdk_bedrockruntime::types::ConversationRole;

    let mut conversation = two_tool_results_conversation();
    conversation.pop();
    let built = rho_provider_bedrock::build_messages_for_model(&conversation, THINKING_MODEL);
    assert_eq!(built.len(), 3);
    assert_eq!(built[2].role(), &ConversationRole::User);
}

#[test]
fn every_content_block_has_an_explicit_arm() {
    // The request builder must name every content-block kind. A `_ => {}` arm dropped a
    // block in silence, which SPEC-reasoning-across-providers section 3 "Three" calls a
    // defect. The match in `build_messages` is now exhaustive, so a new block kind breaks
    // the build instead of hiding. This test proves the two non-travelling kinds are
    // dropped in their named arms, and the travelling kinds still travel.
    use aws_sdk_bedrockruntime::types::ContentBlock as SdkBlock;
    use rho_core::{ContentBlock, ImageSource, Message, Role};

    let conversation = vec![Message {
        role: Role::Assistant,
        content: vec![
            ContentBlock::Text {
                text: "answer".to_string(),
            },
            // A trace must not travel. A replay block with a matching owner does, and the
            // unit tests in the crate cover that. See `SPEC-reasoning-across-providers`.
            ContentBlock::ReasoningTrace {
                text: "private".to_string(),
            },
            // An image in an assistant message is out of scope for the request builder.
            ContentBlock::Image {
                source: ImageSource {
                    data: "AAAA".to_string(),
                    mime_type: "image/png".to_string(),
                },
            },
        ],
    }];

    let built = rho_provider_bedrock::build_messages_for_model(&conversation, THINKING_MODEL);
    assert_eq!(built.len(), 1, "the one assistant message survives");

    let text_blocks = built[0]
        .content()
        .iter()
        .filter(|block| matches!(block, SdkBlock::Text(_)))
        .count();
    assert_eq!(text_blocks, 1, "the text block travels");
    assert_eq!(
        built[0].content().len(),
        1,
        "only the text block travels; reasoning and image are dropped in named arms"
    );
}
