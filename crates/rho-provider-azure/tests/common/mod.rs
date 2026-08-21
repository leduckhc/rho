//! Shared test support for the Azure OpenAI provider.
//!
//! It builds real Azure `/responses` SSE bytes for each script. It implements
//! the testkit harness.
//!
//! Each integration-test binary uses only part of this module. The allow below
//! silences the false dead-code warning that pattern creates.
#![allow(dead_code)]

use async_trait::async_trait;
use rho_core::{CancelToken, CompletionRequest, Provider};
use rho_provider_azure::{AzureAuth, AzureConfig, AzureProvider, Secret};
use rho_provider_testkit::{HarnessRun, ProviderHarness, Script};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The Responses path on the resource base URL.
pub const RESPONSES_PATH: &str = "/openai/v1/responses";

/// A minimal request. The mock ignores its content.
pub fn sample_request() -> CompletionRequest {
    CompletionRequest {
        model: "gpt-4o".to_string(),
        system: Some("You are a test.".to_string()),
        messages: Vec::new(),
        tools: Vec::new(),
        max_tokens: Some(256),
        temperature: Some(0.0),
        reasoning: None,
    }
}

/// One SSE message. It carries the event name and the JSON data.
fn event(kind: &str, data: &str) -> String {
    format!("event: {kind}\ndata: {data}\n\n")
}

/// The Responses SSE body for the text script. It emits "Hel" then "lo".
pub fn sse_text() -> String {
    let mut body = String::new();
    body.push_str(&event(
        "response.created",
        r#"{"type":"response.created","response":{"id":"resp_1"}}"#,
    ));
    body.push_str(&event(
        "response.output_text.delta",
        r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"Hel"}"#,
    ));
    body.push_str(&event(
        "response.output_text.delta",
        r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"lo"}"#,
    ));
    body.push_str(&event(
        "response.output_text.done",
        r#"{"type":"response.output_text.done","item_id":"msg_1","output_index":0,"content_index":0,"text":"Hello"}"#,
    ));
    body.push_str(&event(
        "response.completed",
        r#"{"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":5},"output":[{"type":"message"}]}}"#,
    ));
    body
}

/// The Responses SSE body for the usage script.
pub fn sse_usage() -> String {
    sse_text()
}

/// The Responses SSE body for the tool-call script. It splits the arguments
/// mid-token across three deltas.
pub fn sse_tool_call() -> String {
    let mut body = String::new();
    body.push_str(&event(
        "response.created",
        r#"{"type":"response.created","response":{"id":"resp_1"}}"#,
    ));
    body.push_str(&event(
        "response.output_item.added",
        r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"get_weather","arguments":""}}"#,
    ));
    body.push_str(&event(
        "response.function_call_arguments.delta",
        r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":0,"delta":"{\"city\":\"Pa"}"#,
    ));
    body.push_str(&event(
        "response.function_call_arguments.delta",
        r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":0,"delta":"ris\",\"unit\":"}"#,
    ));
    body.push_str(&event(
        "response.function_call_arguments.delta",
        r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":0,"delta":"\"celsius\"}"}"#,
    ));
    body.push_str(&event(
        "response.function_call_arguments.done",
        r#"{"type":"response.function_call_arguments.done","item_id":"fc_1","output_index":0,"arguments":"{\"city\":\"Paris\",\"unit\":\"celsius\"}"}"#,
    ));
    body.push_str(&event(
        "response.completed",
        r#"{"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":5},"output":[{"type":"function_call"}]}}"#,
    ));
    body
}

/// The Responses SSE body for a reasoning summary.
pub fn sse_reasoning() -> String {
    let mut body = String::new();
    body.push_str(&event(
        "response.created",
        r#"{"type":"response.created","response":{"id":"resp_1"}}"#,
    ));
    body.push_str(&event(
        "response.reasoning_summary_text.delta",
        r#"{"type":"response.reasoning_summary_text.delta","item_id":"rs_1","output_index":0,"summary_index":0,"delta":"I think"}"#,
    ));
    body.push_str(&event(
        "response.completed",
        r#"{"type":"response.completed","response":{"usage":{"input_tokens":1,"output_tokens":1},"output":[{"type":"reasoning"}]}}"#,
    ));
    body
}

/// Start a mock server for `body` and open a provider stream against it.
pub async fn stream_body(body: String) -> (rho_core::ProviderStream, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(RESPONSES_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;
    let config = AzureConfig::new(
        server.uri(),
        "gpt-4o",
        AzureAuth::ApiKey(Secret::new("test-key")),
    );
    let provider = AzureProvider::new(config);
    let stream = provider
        .stream(sample_request(), CancelToken::new())
        .await
        .expect("stream started");
    (stream, server)
}

/// The testkit harness for Azure.
pub struct AzureHarness;

#[async_trait]
impl ProviderHarness for AzureHarness {
    async fn run(&self, script: Script) -> HarnessRun {
        if let Script::Gated = script {
            let head = "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n"
                .as_bytes()
                .to_vec();
            let tail = sse_text().into_bytes();
            let server =
                rho_provider_testkit::StagedHttpServer::start(head, tail, Duration::from_secs(5))
                    .await
                    .expect("staged server started");
            let config = AzureConfig::new(
                server.base_url(),
                "gpt-4o",
                AzureAuth::ApiKey(Secret::new("test-key")),
            );
            let provider = AzureProvider::new(config);
            let stream = provider
                .stream(sample_request(), CancelToken::new())
                .await
                .expect("stream started");
            return HarnessRun {
                stream,
                guard: Box::new(server),
            };
        }
        let body = match script {
            Script::Text => sse_text(),
            Script::Usage => sse_usage(),
            Script::ToolCall => sse_tool_call(),
            Script::Gated => unreachable!(),
        };
        let (stream, server) = stream_body(body).await;
        HarnessRun {
            stream,
            guard: Box::new(server),
        }
    }
}
