//! Tests for the append-only context and the error taxonomy.

use rho_core::{Context, ContentBlock, Message, ProviderError, Role};

#[test]
fn context_append_only_preserves_order() {
    let mut context = Context::new(Some("system".to_string()), Vec::new());
    let first = Message {
        role: Role::User,
        content: vec![ContentBlock::Text {
            text: "one".to_string(),
        }],
    };
    let second = Message {
        role: Role::Assistant,
        content: vec![ContentBlock::Text {
            text: "two".to_string(),
        }],
    };
    context.append(first.clone());
    context.append(second.clone());

    assert_eq!(context.messages(), &[first, second]);
    assert_eq!(context.system(), Some("system"));
}

#[test]
fn provider_error_transport_is_retryable() {
    let err = ProviderError::Transport("reset".to_string());
    assert!(err.is_retryable());
}

#[test]
fn provider_error_client_is_not_retryable() {
    let err = ProviderError::Client {
        status: 400,
        message: "bad request".to_string(),
    };
    assert!(!err.is_retryable());
}

#[test]
fn provider_error_rate_limited_is_retryable() {
    let err = ProviderError::RateLimited {
        retry_after_ms: Some(1000),
    };
    assert!(err.is_retryable());
}
