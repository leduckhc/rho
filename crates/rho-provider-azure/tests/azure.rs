//! Azure-specific tests, from `SPEC-provider-interface` section 7.
//!
//! Every test uses a mock server or a pure header builder. No test reaches the
//! network.

mod common;

use common::{RESPONSES_PATH, sse_reasoning, sse_text, sse_tool_call, stream_body};
use futures::StreamExt;
use rho_core::{CancelToken, Provider, StreamEvent};
use rho_provider_azure::{
    AZURE_ENTRA_AUDIENCE, AzureAuth, AzureConfig, AzureProvider, RetryPolicy, Secret,
};
use std::time::Duration;
use tokio::time::timeout;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn drain(mut stream: rho_core::ProviderStream) -> Vec<StreamEvent> {
    let mut out = Vec::new();
    while let Ok(Some(Ok(event))) = timeout(Duration::from_secs(5), stream.next()).await {
        out.push(event);
    }
    out
}

#[test]
fn provider_azure_entra_audience_is_pinned() {
    // The trailing slash is required. A missing slash is a known real-world bug
    // source. Azure rejects a token minted for the wrong audience. This test
    // pins the exact string so a refactor cannot drop the slash.
    assert_eq!(AZURE_ENTRA_AUDIENCE, "https://cognitiveservices.azure.com/");
    assert!(
        AZURE_ENTRA_AUDIENCE.ends_with('/'),
        "the trailing slash is required"
    );
}

#[tokio::test]
async fn provider_azure_streams_output_text_delta() {
    let (stream, _server) = stream_body(sse_text()).await;
    let events = drain(stream).await;
    let text: String = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::TextDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Hello");
}

#[tokio::test]
async fn provider_azure_assembles_function_call_items() {
    let (stream, _server) = stream_body(sse_tool_call()).await;
    let events = drain(stream).await;
    let end = events
        .iter()
        .find_map(|event| match event {
            StreamEvent::ToolCallEnd { arguments, .. } => Some(arguments.clone()),
            _ => None,
        })
        .expect("a ToolCallEnd event");
    assert_eq!(
        end,
        serde_json::json!({ "city": "Paris", "unit": "celsius" })
    );
}

#[tokio::test]
async fn provider_azure_maps_reasoning_summary_to_thinking() {
    let (stream, _server) = stream_body(sse_reasoning()).await;
    let events = drain(stream).await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::ThinkingDelta { .. })),
        "a reasoning summary delta must map to ThinkingDelta"
    );
}

#[test]
fn provider_azure_api_key_sets_api_key_header() {
    // API-key mode sets the api-key header and no Authorization header.
    let auth = AzureAuth::ApiKey(Secret::new("test-key"));
    let (name, value) = auth.header();
    assert_eq!(name, "api-key");
    assert_eq!(value, "test-key");
    assert_ne!(
        name, "Authorization",
        "API-key mode must not set Authorization"
    );
}

#[test]
fn provider_azure_entra_sets_bearer_header() {
    // Entra mode sets Authorization: Bearer and no api-key header.
    let auth = AzureAuth::Entra(Secret::new("token-123"));
    let (name, value) = auth.header();
    assert_eq!(name, "Authorization");
    assert_eq!(value, "Bearer token-123");
    assert_ne!(name, "api-key", "Entra mode must not set api-key");
}

#[test]
fn azure_config_debug_does_not_leak_secret() {
    let config = AzureConfig::new(
        "https://example.openai.azure.com",
        "gpt-4o",
        AzureAuth::ApiKey(Secret::new("sk-super-secret")),
    );
    let shown = format!("{config:?}");
    assert!(
        !shown.contains("sk-super-secret"),
        "config Debug leaked the secret"
    );
}

// --- Request shape, from a live defect -------------------------------------

/// A conversation where the assistant made two tool calls and both returned.
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
                ContentBlock::Text {
                    text: "I will read them.".to_string(),
                },
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

fn request_with(messages: Vec<rho_core::Message>) -> rho_core::CompletionRequest {
    rho_core::CompletionRequest {
        model: "gpt-x".to_string(),
        system: None,
        messages,
        tools: Vec::new(),
        max_tokens: None,
        temperature: None,
        reasoning: None,
    }
}

fn input_items(body: &serde_json::Value) -> Vec<serde_json::Value> {
    body["input"].as_array().expect("input is an array").clone()
}

#[test]
fn build_request_body_never_sends_the_tool_role() {
    // A live run against Azure found this. The Responses API answered 400:
    //
    //   Invalid value: 'tool'. Supported values are: 'assistant', 'system',
    //   'developer', and 'user'.
    //
    // Responses has no tool role. A tool result is an item, not a message.
    let body = rho_provider_azure::build_request_body(
        &request_with(two_tool_results_conversation()),
        "gpt-x",
    );
    for item in input_items(&body) {
        if let Some(role) = item.get("role").and_then(|value| value.as_str()) {
            assert_ne!(
                role, "tool",
                "Responses rejects the tool role. Send a function_call_output item"
            );
        }
    }
}

#[test]
fn build_request_body_sends_tool_results_as_function_call_output_items() {
    let body = rho_provider_azure::build_request_body(
        &request_with(two_tool_results_conversation()),
        "gpt-x",
    );
    let outputs: Vec<&serde_json::Value> = input_items(&body)
        .iter()
        .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("function_call_output"))
        .cloned()
        .collect::<Vec<_>>()
        .leak()
        .iter()
        .collect();
    assert_eq!(outputs.len(), 2, "both tool results must appear");
    let ids: Vec<&str> = outputs
        .iter()
        .map(|item| item["call_id"].as_str().expect("call_id is a string"))
        .collect();
    assert_eq!(ids, vec!["call_1", "call_2"]);
    assert_eq!(outputs[0]["output"], serde_json::json!("alpha"));
}

#[test]
fn build_request_body_replays_the_assistant_tool_calls() {
    // Responses requires that a `function_call` item precedes its output, and that
    // the `call_id` values match. The provider dropped the assistant's tool calls, so
    // an output referenced a call the service had never seen in the input.
    let body = rho_provider_azure::build_request_body(
        &request_with(two_tool_results_conversation()),
        "gpt-x",
    );
    let items = input_items(&body);
    let calls: Vec<&serde_json::Value> = items
        .iter()
        .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("function_call"))
        .collect();
    assert_eq!(calls.len(), 2, "both tool calls must be replayed");
    assert_eq!(calls[0]["call_id"], serde_json::json!("call_1"));
    assert_eq!(calls[0]["name"], serde_json::json!("read"));
    // Responses carries the arguments as a JSON string, not as an object.
    let arguments = calls[0]["arguments"]
        .as_str()
        .expect("arguments must be a JSON string");
    let parsed: serde_json::Value =
        serde_json::from_str(arguments).expect("the arguments string must parse");
    assert_eq!(parsed, serde_json::json!({ "path": "a.txt" }));

    // Ordering matters: every function_call must precede its own output.
    let call_position = items
        .iter()
        .position(|item| item.get("type").and_then(|v| v.as_str()) == Some("function_call"))
        .expect("a function_call item");
    let output_position = items
        .iter()
        .position(|item| item.get("type").and_then(|v| v.as_str()) == Some("function_call_output"))
        .expect("a function_call_output item");
    assert!(
        call_position < output_position,
        "a function_call must precede its output"
    );
}

#[test]
fn build_request_body_keeps_plain_text_messages_as_messages() {
    use rho_core::{ContentBlock, Message, Role};
    let body = rho_provider_azure::build_request_body(
        &request_with(vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: "hello".to_string(),
            }],
        }]),
        "gpt-x",
    );
    let items = input_items(&body);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["role"], serde_json::json!("user"));
    assert_eq!(items[0]["content"], serde_json::json!("hello"));
}

#[tokio::test]
async fn provider_azure_reports_cache_tokens() {
    // The shape is copied from a live probe of the Responses endpoint, not from memory:
    // `usage.input_tokens_details.cached_tokens`. rho used to report zero. See D-measured-cost-and-cache.
    let body = concat!(
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"usage\":",
        "{\"input_tokens\":1200,\"input_tokens_details\":{\"cached_tokens\":900,",
        "\"cache_write_tokens\":300},\"output_tokens\":42,",
        "\"output_tokens_details\":{\"reasoning_tokens\":0},\"total_tokens\":1242},",
        "\"output\":[]}}\n\n",
    );
    let (stream, _server) = stream_body(body.to_string()).await;
    let events = drain(stream).await;
    let usage = events
        .iter()
        .find_map(|event| match event {
            StreamEvent::Usage(usage) => Some(*usage),
            _ => None,
        })
        .expect("a Usage event");
    assert_eq!(usage.input_tokens, 1200);
    assert_eq!(usage.cache_read_tokens, 900, "cached_tokens must be read");
    assert_eq!(usage.cache_write_tokens, 300);
    // Azure reports no charge, so the field stays absent rather than reading as free.
    assert_eq!(usage.cost_usd, None);
}

/// A set effort level that cannot travel must be reported, not swallowed.
///
/// A review called the silence the defect: the agent carries the level, Bedrock consumes it,
/// and here nothing happened at all, so `unset` and `set` looked the same. rho has no Azure
/// account to drive, so it does not guess a field name. It says the level had no effect.
#[test]
fn a_set_effort_is_reported_as_having_no_effect() {
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct BufferWriter(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for BufferWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufferWriter {
        type Writer = BufferWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    let buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(BufferWriter(Arc::clone(&buffer)))
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let body = tracing::subscriber::with_default(subscriber, || {
        let mut request = common::sample_request();
        request.reasoning = Some(rho_core::ReasoningEffort::Medium);
        let body = rho_provider_azure::build_request_body(&request, "deployment");
        tracing::warn!("the capture is live");
        body
    });
    let logged = String::from_utf8(buffer.lock().unwrap().clone()).expect("utf8");
    assert!(logged.contains("the capture is live"), "the capture works");
    assert!(
        logged.contains("had no effect"),
        "the gap is reported: {logged}"
    );
    // And rho invents no field, because an unknown field is a failed turn.
    assert!(body.get("reasoning").is_none(), "no guessed field: {body}");
    assert!(body.get("reasoning_effort").is_none());
}

// --- The HTTP client policy, from a security review ------------------------
//
// Azure carries an api-key on every request, and its base URL comes from
// `AZURE_OPENAI_ENDPOINT`, which the config trust filter never touches. So the
// client must not follow a redirect, and must not route a loopback request
// through a proxy. These two tests pin that. Break the client policy and both
// fail. See TODO items B2 and B3.

/// A redirect must not be followed, so the api-key never reaches a second origin.
#[tokio::test]
async fn a_redirect_is_not_followed_so_the_api_key_stays_on_the_named_host() {
    // The origin the api-key must never reach.
    let attacker = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(RESPONSES_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse_text(), "text/event-stream"))
        .mount(&attacker)
        .await;

    // The named endpoint answers 307 to the attacker origin. A 307 preserves the
    // POST method and body, so a followed hop would resend the whole request.
    let named = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(RESPONSES_PATH))
        .respond_with(
            ResponseTemplate::new(307)
                .append_header("location", format!("{}{RESPONSES_PATH}", attacker.uri())),
        )
        .mount(&named)
        .await;

    let config = AzureConfig::new(
        named.uri(),
        "gpt-4o",
        AzureAuth::ApiKey(Secret::new("sk-secret")),
    )
    .with_retry(RetryPolicy::none());
    let provider = AzureProvider::new(config);
    let result = provider
        .stream(common::sample_request(), CancelToken::new())
        .await;

    assert!(
        result.is_err(),
        "a 3xx redirect must surface as an error, not a followed hop"
    );
    let hits = attacker.received_requests().await.unwrap_or_default();
    assert!(
        hits.is_empty(),
        "the api-key must never reach the redirect target; saw {} request(s)",
        hits.len()
    );
}

/// Serialises the proxy tests, because a proxy variable is process-global.
static PROXY_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A loopback request must bypass a proxy variable, so the api-key never reaches it.
#[tokio::test]
async fn a_loopback_request_bypasses_a_proxy_variable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(RESPONSES_PATH))
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

        let config = AzureConfig::new(
            server.uri(),
            "gpt-4o",
            AzureAuth::ApiKey(Secret::new("sk-secret")),
        )
        .with_retry(RetryPolicy::none());
        let provider = AzureProvider::new(config);

        for (key, value) in prior {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
        provider
    };

    let result = provider
        .stream(common::sample_request(), CancelToken::new())
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
