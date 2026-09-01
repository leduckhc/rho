//! End-to-end wiremock test. rho POSTs to a mock, reads SSE, and yields StreamEvent.
//!
//! This drives every step the live drive drives, at the crate boundary: request build,
//! HTTP POST, response streaming, SSE parsing, StreamEvent yield. A green here means the
//! next step is the tunnel, not more unit tests.

use futures::StreamExt;
use rho_core::{
    CancelToken, CompletionRequest, ContentBlock, Message, Provider, Role, Secret, StopReason,
    StreamEvent,
};
use rho_provider_anthropic::{AnthropicConfig, AnthropicProvider};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn one_user_prompt(text: &str) -> CompletionRequest {
    CompletionRequest {
        model: "claude-sonnet-4-6".to_string(),
        system: None,
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
        }],
        tools: Vec::new(),
        max_tokens: Some(64),
        temperature: None,
        reasoning: None,
    }
}

/// The full SSE body a happy Claude turn returns, hand-written so the test owns every byte.
const HAPPY_SSE: &str = "\
event: message_start
data: {\"type\":\"message_start\",\"message\":{\"role\":\"assistant\"}}

event: content_block_start
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" world\"}}

event: content_block_stop
data: {\"type\":\"content_block_stop\",\"index\":0}

event: message_delta
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":3}}

event: message_stop
data: {\"type\":\"message_stop\"}

";

#[tokio::test]
async fn a_happy_turn_yields_the_normalised_events() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "test-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(HAPPY_SSE)
                .insert_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let provider =
        AnthropicProvider::new(AnthropicConfig::new(server.uri(), Secret::from("test-key")));

    let mut stream = provider
        .stream(one_user_prompt("hi"), CancelToken::new())
        .await
        .expect("the mock accepts the POST and returns SSE");

    let mut events = Vec::new();
    while let Some(item) = stream.next().await {
        events.push(item.expect("the happy path yields no error"));
    }

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
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn
            },
        ]
    );
}

#[tokio::test]
async fn an_empty_credential_never_reaches_the_network() {
    // The mock is not mounted with a matcher, so it 404s any request. If the empty-key
    // check fires, the provider errors before any POST, so the mock records nothing.
    let server = MockServer::start().await;
    let provider = AnthropicProvider::new(AnthropicConfig::new(server.uri(), Secret::from("")));

    let outcome = provider
        .stream(one_user_prompt("hi"), CancelToken::new())
        .await
        .err();

    assert!(
        matches!(&outcome, Some(rho_core::ProviderError::Auth(_))),
        "an empty credential must error before the POST: {outcome:?}"
    );
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "the network must not see a request with an empty key"
    );
}

#[tokio::test]
async fn a_401_maps_to_auth_and_names_the_variable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(401).set_body_string("{\"error\":\"nope\"}"))
        .mount(&server)
        .await;

    let provider =
        AnthropicProvider::new(AnthropicConfig::new(server.uri(), Secret::from("bad-key")));
    let outcome = provider
        .stream(one_user_prompt("hi"), CancelToken::new())
        .await
        .err();

    match outcome {
        Some(rho_core::ProviderError::Auth(message)) => {
            assert!(
                message.contains("ANTHROPIC_API_KEY"),
                "the auth error must name the variable to set: {message}"
            );
            assert!(
                !message.contains("nope"),
                "the auth error must not reflect the peer body: {message}"
            );
        }
        other => panic!("a 401 must map to Auth, got {other:?}"),
    }
}

#[tokio::test]
async fn a_500_maps_to_server_and_carries_the_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(503).set_body_string("outage"))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(AnthropicConfig::new(server.uri(), Secret::from("k")));
    let outcome = provider
        .stream(one_user_prompt("hi"), CancelToken::new())
        .await
        .err();

    match outcome {
        Some(rho_core::ProviderError::Server { status }) => assert_eq!(status, 503),
        other => panic!("a 503 must map to Server, got {other:?}"),
    }
}
