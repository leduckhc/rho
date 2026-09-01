//! The request body is one shape, and this file pins it.
//!
//! The Anthropic Messages API is close to OpenAI's Chat Completions, and close is enough
//! for a defect. Two differences bite: `system` is a top-level field on Anthropic, not a
//! message with role `system`. And `max_tokens` is required.
//!
//! Every test here asserts against a small, hand-written JSON value. A single call to
//! `build_request_body` produces it, and the test reads the fields it cares about. So a
//! future field is a compile error at the assertion site, not a silent drop.

use rho_core::{CompletionRequest, ContentBlock, Message, Role};
use rho_provider_anthropic::build_request_body;
use serde_json::json;

fn one_user_turn(text: &str) -> CompletionRequest {
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
        max_tokens: Some(1024),
        temperature: None,
        reasoning: None,
    }
}

#[test]
fn a_bare_prompt_carries_only_the_message_and_the_required_fields() {
    let body = build_request_body(&one_user_turn("hello"));
    assert_eq!(body["model"], "claude-sonnet-4-6");
    assert_eq!(body["max_tokens"], 1024);
    assert_eq!(body["stream"], true);
    assert_eq!(
        body["messages"],
        json!([{"role": "user", "content": [{"type": "text", "text": "hello"}]}])
    );
    assert!(
        body.get("system").is_none(),
        "no system prompt means no system field: {body}"
    );
    assert!(
        body.get("tools").is_none(),
        "no tools means no tools field: {body}"
    );
    assert!(
        body.get("thinking").is_none(),
        "no reasoning means no thinking field: {body}"
    );
}

#[test]
fn a_system_prompt_is_top_level_never_a_message() {
    // A drive against the earlier spec-authoring lesson: the wire is not OpenAI, and a
    // system message would be refused. The field lives at the top level.
    let mut request = one_user_turn("hi");
    request.system = Some("you are a test".to_string());
    let body = build_request_body(&request);
    assert_eq!(body["system"], "you are a test");
    let messages = body["messages"].as_array().expect("messages is an array");
    assert!(
        messages.iter().all(|message| message["role"] != "system"),
        "no message may carry role `system`: {body}"
    );
}

#[test]
fn max_tokens_is_required_and_absent_is_a_provider_default() {
    // Anthropic 400s a request with no `max_tokens`. rho carries `Option<u32>`, so an
    // absent value must still send a number. The spec picks 4096 as the safe default,
    // matching the sibling providers.
    let mut request = one_user_turn("hi");
    request.max_tokens = None;
    let body = build_request_body(&request);
    assert!(
        body["max_tokens"].as_u64().is_some(),
        "an absent max_tokens must send the default, not the field's absence: {body}"
    );
    assert_eq!(body["max_tokens"], 4096, "the spec's default is 4096");
}
