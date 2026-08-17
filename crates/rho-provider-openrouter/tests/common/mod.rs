//! Shared test support for the OpenRouter provider.
//!
//! It builds real OpenRouter SSE bytes for each script. It implements the
//! testkit harness, so the shared contract runs against this provider.
//!
//! Each integration-test binary uses only part of this module. The allow below
//! silences the false dead-code warning that pattern creates.
#![allow(dead_code)]

use async_trait::async_trait;
use rho_core::{CancelToken, CompletionRequest, Provider};
use rho_provider_openrouter::{OpenRouterConfig, OpenRouterProvider, Secret};
use rho_provider_testkit::{HarnessRun, ProviderHarness, Script};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The chat-completions path on the OpenRouter base URL.
pub const CHAT_PATH: &str = "/api/v1/chat/completions";

/// A minimal request. The mock ignores its content.
pub fn sample_request() -> CompletionRequest {
    CompletionRequest {
        model: "openai/gpt-4o".to_string(),
        system: Some("You are a test.".to_string()),
        messages: Vec::new(),
        tools: Vec::new(),
        max_tokens: Some(256),
        temperature: Some(0.0),
    }
}

/// One SSE frame. Each frame is a `data:` line and a blank line.
fn frame(json: &str) -> String {
    format!("data: {json}\n\n")
}

/// The SSE body for the text script. It emits "Hel" then "lo", then stops.
/// It also includes a keep-alive comment line, which the provider must skip.
pub fn sse_text() -> String {
    let mut body = String::new();
    body.push_str(": OPENROUTER PROCESSING\n\n");
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"role":"assistant"}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"content":"Hel"}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"content":"lo"}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
    ));
    body.push_str("data: [DONE]\n\n");
    body
}

/// The SSE body for the usage script. It reports usage on the final chunk.
pub fn sse_usage() -> String {
    let mut body = String::new();
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"role":"assistant"}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"content":"Hel"}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"content":"lo"}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
    ));
    body.push_str("data: [DONE]\n\n");
    body
}

/// The SSE body for the tool-call script. It splits one call across four
/// chunks. It splits the JSON arguments mid-token, so they parse only after
/// concatenation.
pub fn sse_tool_call() -> String {
    let mut body = String::new();
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"role":"assistant"}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"get_weather","arguments":""}}]}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":\"Pa"}}]}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ris\",\"unit\":"}}]}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"celsius\"}"}}]}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
    ));
    body.push_str("data: [DONE]\n\n");
    body
}

/// The SSE body for a mid-stream error. The stream sends one delta, then a
/// chunk with a top-level `error` and `finish_reason: "error"`.
pub fn sse_midstream_error() -> String {
    let mut body = String::new();
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"role":"assistant"}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"content":"Hel"}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"error":{"message":"upstream failed","code":502},"choices":[{"index":0,"delta":{},"finish_reason":"error"}]}"#,
    ));
    body
}

/// The SSE body for two parallel tool calls, interleaved by `index`.
pub fn sse_parallel_tool_calls() -> String {
    let mut body = String::new();
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"role":"assistant"}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"get_weather","arguments":""}}]}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call_b","type":"function","function":{"name":"get_time","arguments":""}}]}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":\"Paris\"}"}}]}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"function":{"arguments":"{\"zone\":\"UTC\"}"}}]}}]}"#,
    ));
    body.push_str(&frame(
        r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
    ));
    body.push_str("data: [DONE]\n\n");
    body
}

/// Start a mock server for `body` and open a provider stream against it.
/// The returned server must stay alive while the stream is read.
pub async fn stream_body(body: String) -> (rho_core::ProviderStream, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(CHAT_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;
    let config = OpenRouterConfig::new(Secret::new("test-key")).with_base_url(server.uri());
    let provider = OpenRouterProvider::new(config);
    let stream = provider
        .stream(sample_request(), CancelToken::new())
        .await
        .expect("stream started");
    (stream, server)
}

/// The testkit harness for OpenRouter.
pub struct OpenRouterHarness;

#[async_trait]
impl ProviderHarness for OpenRouterHarness {
    async fn run(&self, script: Script) -> HarnessRun {
        let provider_stream;
        let guard: Box<dyn std::any::Any + Send>;
        match script {
            Script::Gated => {
                // Split the text body into a head and a held-back tail.
                let head = ": keep-alive\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"}}]}\n\n"
                    .as_bytes()
                    .to_vec();
                let tail = sse_text().into_bytes();
                let server = rho_provider_testkit::StagedHttpServer::start(
                    head,
                    tail,
                    Duration::from_secs(30),
                )
                .await
                .expect("staged server started");
                let config =
                    OpenRouterConfig::new(Secret::new("test-key")).with_base_url(server.base_url());
                let provider = OpenRouterProvider::new(config);
                provider_stream = provider
                    .stream(sample_request(), CancelToken::new())
                    .await
                    .expect("stream started");
                guard = Box::new(server);
            }
            other => {
                let body = match other {
                    Script::Text => sse_text(),
                    Script::Usage => sse_usage(),
                    Script::ToolCall => sse_tool_call(),
                    Script::Gated => unreachable!(),
                };
                let server = MockServer::start().await;
                Mock::given(method("POST"))
                    .and(path(CHAT_PATH))
                    .respond_with(
                        ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"),
                    )
                    .mount(&server)
                    .await;
                let config =
                    OpenRouterConfig::new(Secret::new("test-key")).with_base_url(server.uri());
                let provider = OpenRouterProvider::new(config);
                provider_stream = provider
                    .stream(sample_request(), CancelToken::new())
                    .await
                    .expect("stream started");
                guard = Box::new(server);
            }
        }
        HarnessRun {
            stream: provider_stream,
            guard,
        }
    }
}
