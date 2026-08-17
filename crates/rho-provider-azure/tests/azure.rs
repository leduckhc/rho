//! Azure-specific tests, from `SPEC-02` section 7.
//!
//! Every test uses a mock server or a pure header builder. No test reaches the
//! network.

mod common;

use common::{sse_reasoning, sse_text, sse_tool_call, stream_body};
use futures::StreamExt;
use rho_core::StreamEvent;
use rho_provider_azure::{AZURE_ENTRA_AUDIENCE, AzureAuth, AzureConfig, Secret};
use std::time::Duration;
use tokio::time::timeout;

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
