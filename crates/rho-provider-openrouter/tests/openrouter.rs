//! OpenRouter-specific tests, from `SPEC-provider-interface` section 7.
//!
//! Every test uses a mock server or the staged server. No test reaches the
//! network.

mod common;

use common::{
    CHAT_PATH, sample_request, sse_midstream_error, sse_parallel_tool_calls, sse_text,
    sse_tool_call, sse_usage, sse_usage_after_finish, sse_usage_with_cache_and_cost, stream_body,
};
use futures::StreamExt;
use rho_core::{CancelToken, Provider, StopReason, StreamEvent};
use rho_provider_openrouter::{OpenRouterConfig, OpenRouterProvider, RetryPolicy, Secret};
use std::time::Duration;
use tokio::time::timeout;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
            Ok(StreamEvent::ToolCallEnd {
                index, arguments, ..
            }) => Some((*index, arguments.clone())),
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

/// The effort level must reach the OpenRouter body, or the setting is a lie on this provider.
///
/// A review found this: the agent carried the level faithfully, Bedrock consumed it, and this
/// crate ignored it. A user running `--reasoning-effort high` here got silence, so `unset` and
/// `set` collapsed to the same wire.
#[test]
fn the_request_body_carries_the_effort() {
    let mut request = common::sample_request();
    request.reasoning = Some(rho_core::ReasoningEffort::High);
    let body = rho_provider_openrouter::build_request_body(&request);
    assert_eq!(
        body["reasoning"]["effort"], "high",
        "the level reaches the wire: {body}"
    );
}

/// `off` asks the host not to think, rather than saying nothing.
#[test]
fn an_off_effort_disables_reasoning_on_the_wire() {
    let mut request = common::sample_request();
    request.reasoning = Some(rho_core::ReasoningEffort::Off);
    let body = rho_provider_openrouter::build_request_body(&request);
    assert_eq!(
        body["reasoning"]["enabled"], false,
        "off is an instruction, not a silence: {body}"
    );
    assert!(body["reasoning"].get("effort").is_none());
}

/// An unset level sends no field, so the host keeps its own default.
#[test]
fn an_absent_effort_sends_no_reasoning_field() {
    let request = common::sample_request();
    let body = rho_provider_openrouter::build_request_body(&request);
    assert!(
        body.get("reasoning").is_none(),
        "unset must not send a field: {body}"
    );
}

/// `xhigh` is not in OpenRouter's set, and rho must not invent a value the host rejects.
/// It maps to the highest level the host accepts, and the mapping is stated in one place.
#[test]
fn xhigh_maps_to_the_highest_accepted_level() {
    let mut request = common::sample_request();
    request.reasoning = Some(rho_core::ReasoningEffort::XHigh);
    let body = rho_provider_openrouter::build_request_body(&request);
    assert_eq!(body["reasoning"]["effort"], "high");
}

/// Every level maps to its own wire value, and the table says which.
///
/// A mutation review swapped `low` for `high` and every test still passed: the tests covered
/// `off`, `high`, and `xhigh`, so nothing pinned the middle of the ladder. A level that maps
/// upward costs a user money they did not ask to spend, so the whole mapping is now a table.
#[test]
fn every_level_maps_to_its_own_wire_value() {
    let cases = [
        (rho_core::ReasoningEffort::Off, None),
        (rho_core::ReasoningEffort::Low, Some("low")),
        (rho_core::ReasoningEffort::Medium, Some("medium")),
        (rho_core::ReasoningEffort::High, Some("high")),
        // The host has no `xhigh`, so rho sends the highest it accepts rather than inventing one.
        (rho_core::ReasoningEffort::XHigh, Some("high")),
    ];
    for (effort, expected) in cases {
        let mut request = common::sample_request();
        request.reasoning = Some(effort);
        let body = rho_provider_openrouter::build_request_body(&request);
        match expected {
            Some(word) => assert_eq!(
                body["reasoning"]["effort"],
                word,
                "{} must send {word}: {body}",
                effort.as_str()
            ),
            None => assert_eq!(
                body["reasoning"]["enabled"], false,
                "off disables rather than choosing a level: {body}"
            ),
        }
    }
}

// --- The HTTP client policy, from a security review ------------------------
//
// The bearer token must never leave the named host, and a loopback request must
// never go through a proxy. `wire.rs` tests the pure `bypasses_proxy` predicate
// beside the seam; these two tests drive the built client, so a revert of the
// redirect policy or the `no_proxy` call is caught. See TODO items B1 and B3.

/// A redirect must not be followed, so the bearer token never reaches a second origin.
#[tokio::test]
async fn a_redirect_is_not_followed_so_the_token_stays_on_the_named_host() {
    // The origin the token must never reach.
    let attacker = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(CHAT_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse_text(), "text/event-stream"))
        .mount(&attacker)
        .await;

    // The named endpoint answers 307 to the attacker origin. A 307 preserves the
    // POST method and body, so a followed hop would resend the whole request.
    let named = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(CHAT_PATH))
        .respond_with(
            ResponseTemplate::new(307)
                .append_header("location", format!("{}{CHAT_PATH}", attacker.uri())),
        )
        .mount(&named)
        .await;

    let config = OpenRouterConfig::new(Secret::new("sk-secret"))
        .with_base_url(named.uri())
        .with_retry(RetryPolicy::none());
    let provider = OpenRouterProvider::new(config);
    let result = provider.stream(sample_request(), CancelToken::new()).await;

    assert!(
        result.is_err(),
        "a 3xx redirect must surface as an error, not a followed hop"
    );
    let hits = attacker.received_requests().await.unwrap_or_default();
    assert!(
        hits.is_empty(),
        "the bearer token must never reach the redirect target; saw {} request(s)",
        hits.len()
    );
}

/// Serialises the proxy tests, because a proxy variable is process-global.
static PROXY_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A loopback request must bypass a proxy variable, so the token never reaches it.
#[tokio::test]
async fn a_loopback_request_bypasses_a_proxy_variable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(CHAT_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse_text(), "text/event-stream"))
        .mount(&server)
        .await;

    // Build the client inside a synchronous critical section that owns the process-global
    // proxy variables. `reqwest` reads the variables when the client is built, and the
    // client build is synchronous, so the guard never crosses an `.await`. That keeps the
    // proxy tests deterministic without holding a lock across a suspension point.
    let provider = {
        let _guard = PROXY_ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());

        // A dead proxy. If the client honoured it, the loopback request would route
        // there and fail.
        const PROXY_VARS: [&str; 3] = ["HTTP_PROXY", "http_proxy", "ALL_PROXY"];
        let prior: Vec<(&str, Option<String>)> = PROXY_VARS
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect();
        for key in PROXY_VARS {
            // SAFETY: the guard serialises the proxy tests, and every other client this
            // crate builds targets a loopback host and so calls `no_proxy`, which makes it
            // immune to this variable. The variable is restored before the guard drops.
            unsafe { std::env::set_var(key, "http://127.0.0.1:1") };
        }

        let config = OpenRouterConfig::new(Secret::new("sk-secret"))
            .with_base_url(server.uri())
            .with_retry(RetryPolicy::none());
        let provider = OpenRouterProvider::new(config);

        for (key, value) in prior {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
        provider
    };

    let result = provider
        .stream(sample_request(), CancelToken::new())
        .await
        .map(|_| ());
    assert!(
        result.is_ok(),
        "a loopback request must bypass the proxy, got {:?}",
        result.err()
    );
    let hits = server.received_requests().await.unwrap_or_default();
    assert_eq!(
        hits.len(),
        1,
        "the loopback server must receive the request directly"
    );
}
