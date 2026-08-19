//! OpenRouter-specific tests, from `SPEC-provider-interface` section 7.
//!
//! Every test uses a mock server or the staged server. No test reaches the
//! network.

mod common;

use common::{
    sse_midstream_error, sse_parallel_tool_calls, sse_text, sse_tool_call, sse_usage,
    sse_usage_after_finish, sse_usage_with_cache_and_cost, stream_body,
};
use futures::StreamExt;
use rho_core::{StopReason, StreamEvent};
use rho_provider_openrouter::{OpenRouterConfig, RetryPolicy, Secret};
use std::time::Duration;
use tokio::time::timeout;

/// Read every event, with a bounded wait per event.
async fn drain(
    mut stream: rho_core::ProviderStream,
) -> Vec<Result<StreamEvent, rho_core::ProviderError>> {
    let mut out = Vec::new();
    while let Ok(Some(item)) = timeout(Duration::from_secs(5), stream.next()).await {
        let stop = item.is_err();
        out.push(item);
        if stop {
            break;
        }
    }
    out
}

#[tokio::test]
async fn provider_openrouter_streams_text_deltas() {
    let (stream, _server) = stream_body(sse_text()).await;
    let events = drain(stream).await;
    let text: String = events
        .iter()
        .filter_map(|item| match item {
            Ok(StreamEvent::TextDelta { delta, .. }) => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Hello");
}

#[tokio::test]
async fn provider_openrouter_assembles_split_tool_call_fragments() {
    let (stream, _server) = stream_body(sse_tool_call()).await;
    let events = drain(stream).await;
    let end = events
        .iter()
        .find_map(|item| match item {
            Ok(StreamEvent::ToolCallEnd { arguments, .. }) => Some(arguments.clone()),
            _ => None,
        })
        .expect("a ToolCallEnd event");
    assert_eq!(
        end,
        serde_json::json!({ "city": "Paris", "unit": "celsius" }),
        "split fragments must assemble into one parsed object"
    );
}

#[tokio::test]
async fn provider_openrouter_assembles_parallel_tool_calls() {
    // Two calls arrive interleaved by index. Neither may steal the other's
    // arguments. This is a common real bug in provider code.
    let (stream, _server) = stream_body(sse_parallel_tool_calls()).await;
    let events = drain(stream).await;
    let mut ends: Vec<(u32, serde_json::Value)> = events
        .iter()
        .filter_map(|item| match item {
            Ok(StreamEvent::ToolCallEnd { index, arguments }) => Some((*index, arguments.clone())),
            _ => None,
        })
        .collect();
    ends.sort_by_key(|(index, _)| *index);
    assert_eq!(ends.len(), 2, "both tool calls must complete");
    assert_eq!(ends[0].1, serde_json::json!({ "city": "Paris" }));
    assert_eq!(ends[1].1, serde_json::json!({ "zone": "UTC" }));
}

#[tokio::test]
async fn provider_openrouter_skips_processing_comment_lines() {
    // The text body starts with a `: OPENROUTER PROCESSING` comment. The
    // provider must skip it and still stream the text.
    let (stream, _server) = stream_body(sse_text()).await;
    let events = drain(stream).await;
    assert!(
        events
            .iter()
            .any(|item| matches!(item, Ok(StreamEvent::TextDelta { .. }))),
        "the comment line must not break the stream"
    );
}

#[tokio::test]
async fn provider_openrouter_reports_usage() {
    let (stream, _server) = stream_body(sse_usage()).await;
    let events = drain(stream).await;
    let usage = events
        .iter()
        .find_map(|item| match item {
            Ok(StreamEvent::Usage(usage)) => Some(*usage),
            _ => None,
        })
        .expect("a Usage event");
    assert_eq!(usage.input_tokens, 10);
    assert_eq!(usage.output_tokens, 5);
}

#[tokio::test]
async fn provider_openrouter_midstream_error_yields_err() {
    // A mid-stream error must surface as Err, not a silent truncation.
    let (stream, _server) = stream_body(sse_midstream_error()).await;
    let events = drain(stream).await;
    assert!(
        events.iter().any(|item| item.is_err()),
        "a mid-stream error must yield Err"
    );
}

#[tokio::test]
async fn provider_openrouter_maps_finish_reason_tool_calls_to_tool_use() {
    let (stream, _server) = stream_body(sse_tool_call()).await;
    let events = drain(stream).await;
    let done = events
        .iter()
        .find_map(|item| match item {
            Ok(StreamEvent::Done { stop_reason }) => Some(*stop_reason),
            _ => None,
        })
        .expect("a Done event");
    assert_eq!(done, StopReason::ToolUse);
}

// --- Retry policy, from SPEC-provider-interface section 3. -------------------------------

#[tokio::test]
async fn retry_policy_backs_off_on_server_error() {
    // A Server error is retryable, and the policy returns a delay within the
    // attempt cap.
    let policy = RetryPolicy::default();
    assert!(rho_core::ProviderError::Server { status: 503 }.is_retryable());
    assert!(
        policy.backoff(1, None).is_some(),
        "first retry must have a delay"
    );
    assert!(
        policy.backoff(policy.max_attempts + 1, None).is_none(),
        "no retry past the attempt cap"
    );
}

#[tokio::test]
async fn retry_policy_never_retries_client_error() {
    // A 4xx client error, other than 429, must never retry. A retry on a bad
    // key would burn the user's rate limit.
    let error = rho_core::ProviderError::Client {
        status: 401,
        message: "bad key".to_string(),
    };
    assert!(!error.is_retryable(), "a client error must not retry");
}

#[tokio::test]
async fn retry_policy_honours_retry_after() {
    // A server hint waits the exact hint and skips the jitter.
    let policy = RetryPolicy::default();
    let delay = policy.backoff(1, Some(2000)).expect("a delay");
    assert_eq!(delay, Duration::from_millis(2000));
}

// --- Secret redaction, from SPEC-provider-interface section 2. ---------------------------

#[test]
fn secret_debug_is_redacted() {
    // The Debug output must never carry the value. This proves redaction by
    // construction, not by a late filter.
    let secret = Secret::new("sk-super-secret");
    let shown = format!("{secret:?}");
    assert_eq!(shown, "Secret(***)");
    assert!(!shown.contains("sk-super-secret"));
}

#[test]
fn openrouter_config_debug_does_not_leak_key() {
    let config = OpenRouterConfig::new(Secret::new("sk-super-secret"));
    let shown = format!("{config:?}");
    assert!(
        !shown.contains("sk-super-secret"),
        "config Debug leaked the key"
    );
}

#[tokio::test]
async fn provider_openrouter_reports_cache_tokens_and_cost() {
    // rho used to report zero for both cache fields, so a user could not see the saving
    // that the append-only context rule works to earn. The shape here is copied from a
    // live probe of the API. See decision D-measured-cost-and-cache.
    let (stream, _server) = stream_body(sse_usage_with_cache_and_cost()).await;
    let events = drain(stream).await;
    let usage = events
        .iter()
        .find_map(|item| match item {
            Ok(StreamEvent::Usage(usage)) => Some(*usage),
            _ => None,
        })
        .expect("a Usage event");

    assert_eq!(usage.input_tokens, 2409);
    assert_eq!(usage.cache_read_tokens, 1800, "cached_tokens must be read");
    assert_eq!(usage.cache_write_tokens, 600);
    // The charge is measured, never estimated from a price table.
    assert_eq!(usage.cost_usd, Some(0.002489));
    let ratio = usage.cache_hit_ratio().expect("a ratio");
    assert!(
        (0.42..0.43).contains(&ratio),
        "1800 of 4209 input tokens is about 43 percent, got {ratio}"
    );
}

#[tokio::test]
async fn provider_openrouter_usage_without_details_reports_no_cost() {
    // The older shape has no details object. It must parse, and it must leave the cost
    // absent rather than reporting zero, because zero would read as a free call.
    let (stream, _server) = stream_body(sse_usage()).await;
    let events = drain(stream).await;
    let usage = events
        .iter()
        .find_map(|item| match item {
            Ok(StreamEvent::Usage(usage)) => Some(*usage),
            _ => None,
        })
        .expect("a Usage event");
    assert_eq!(usage.cache_read_tokens, 0);
    assert_eq!(usage.cost_usd, None, "absent must not become zero");
}

#[test]
fn request_asks_for_usage_accounting() {
    // OpenRouter omits `usage` from a streamed response unless the request opts in. rho
    // parsed the cache and cost fields correctly and never received them, so a live run of
    // fifty sessions reported zero tokens. The parse was right and unreachable.
    //
    // This test guards the opt-in, because nothing else would notice its absence: every
    // fixture supplies a usage chunk regardless of the request.
    let body = rho_provider_openrouter::build_request_body(&common::sample_request());
    assert_eq!(
        body["usage"]["include"],
        serde_json::json!(true),
        "the request must opt in to usage accounting: {body}"
    );
}

#[tokio::test]
async fn usage_arriving_after_the_finish_chunk_is_still_reported() {
    // The order a live probe showed: `finish_reason` in one chunk, then the whole `usage`
    // object in the next. rho ended the stream at the finish chunk, so it never saw usage
    // for any OpenRouter call. A fifty-session live run reported zero tokens, which is
    // what exposed it.
    //
    // Every other fixture puts usage in the same chunk as the finish, so nothing else
    // would catch this.
    let (stream, _server) = stream_body(sse_usage_after_finish()).await;
    let events = drain(stream).await;

    let usage = events
        .iter()
        .find_map(|item| match item {
            Ok(StreamEvent::Usage(usage)) => Some(*usage),
            _ => None,
        })
        .expect("a Usage event, even though it arrived after the finish chunk");
    assert_eq!(usage.input_tokens, 9);
    assert_eq!(usage.cache_read_tokens, 4);
    assert_eq!(usage.cost_usd, Some(0.000123));

    // The turn must still end, and exactly once.
    let dones = events
        .iter()
        .filter(|item| matches!(item, Ok(StreamEvent::Done { .. })))
        .count();
    assert_eq!(dones, 1, "exactly one Done event");

    // And the order must hold: usage before the turn ends, so a consumer that stops at
    // `Done` still sees it.
    let usage_at = events
        .iter()
        .position(|item| matches!(item, Ok(StreamEvent::Usage(_))))
        .expect("a usage event");
    let done_at = events
        .iter()
        .position(|item| matches!(item, Ok(StreamEvent::Done { .. })))
        .expect("a done event");
    assert!(
        usage_at < done_at,
        "usage must precede Done, or a consumer that stops at Done misses it"
    );
}

// --- Reading the reasoning wire. -----------------------------------------
//
// SPEC-reasoning-across-providers section 3 "One": rho must read `reasoning`,
// `reasoning_content`, and `reasoning_text`, and take the first non-empty one.

/// Collect every reasoning delta as one string.
fn reasoning_text(events: &[Result<StreamEvent, rho_core::ProviderError>]) -> String {
    events
        .iter()
        .filter_map(|item| match item {
            Ok(StreamEvent::ThinkingDelta { delta, .. }) => Some(delta.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn the_first_non_empty_reasoning_field_wins() {
    let (stream, _server) = stream_body(common::sse_reasoning_two_fields_same_text()).await;
    let events = drain(stream).await;
    assert_eq!(
        reasoning_text(&events),
        "B",
        "two fields with the same text must yield the text once, not twice"
    );
}

#[tokio::test]
async fn reasoning_content_is_read() {
    let (stream, _server) = stream_body(common::sse_reasoning_content_only()).await;
    let events = drain(stream).await;
    assert_eq!(reasoning_text(&events), "why");
}

#[tokio::test]
async fn reasoning_text_is_read() {
    let (stream, _server) = stream_body(common::sse_reasoning_text_only()).await;
    let events = drain(stream).await;
    assert_eq!(reasoning_text(&events), "hmm");
}

#[tokio::test]
async fn an_empty_reasoning_delta_starts_no_block() {
    let (stream, _server) = stream_body(common::sse_reasoning_empty()).await;
    let events = drain(stream).await;
    let starts = events
        .iter()
        .filter(|item| matches!(item, Ok(StreamEvent::ThinkingStart { .. })))
        .count();
    assert_eq!(starts, 0, "an empty reasoning field must start no block");
}
