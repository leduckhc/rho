//! Tests for the append-only context and the error taxonomy.

use rho_core::{ContentBlock, Context, Message, ProviderError, Role};

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

// --- Usage: cache hit ratio and cost -------------------------------------

#[test]
fn cache_hit_ratio_reports_the_share_served_from_cache() {
    let usage = rho_core::Usage {
        input_tokens: 250,
        output_tokens: 10,
        cache_read_tokens: 750,
        cache_write_tokens: 0,
        cost_usd: None,
    };
    // 750 of 1000 input tokens came from the cache.
    assert_eq!(usage.cache_hit_ratio(), Some(0.75));
}

#[test]
fn cache_hit_ratio_is_absent_when_there_are_no_input_tokens() {
    // The distinction matters. "No data" must not read as "no cache hits", and a caller
    // must not divide by zero.
    let usage = rho_core::Usage::default();
    assert_eq!(usage.cache_hit_ratio(), None);
}

#[test]
fn cache_hit_ratio_is_zero_when_nothing_was_cached() {
    let usage = rho_core::Usage {
        input_tokens: 100,
        ..Default::default()
    };
    assert_eq!(usage.cache_hit_ratio(), Some(0.0));
}

#[test]
fn usage_add_sums_tokens_and_cost() {
    let mut total = rho_core::Usage {
        input_tokens: 10,
        output_tokens: 1,
        cache_read_tokens: 2,
        cache_write_tokens: 3,
        cost_usd: Some(0.001),
    };
    total.add(&rho_core::Usage {
        input_tokens: 20,
        output_tokens: 2,
        cache_read_tokens: 4,
        cache_write_tokens: 6,
        cost_usd: Some(0.002),
    });
    assert_eq!(total.input_tokens, 30);
    assert_eq!(total.cache_read_tokens, 6);
    assert_eq!(total.cost_usd, Some(0.003));
}

#[test]
fn usage_add_keeps_a_known_cost_when_the_other_side_has_none() {
    // A provider that reports no charge must not erase a charge another one reported. The
    // absent case has to be explicit, or a total silently reads as zero.
    let mut total = rho_core::Usage {
        cost_usd: Some(0.005),
        ..Default::default()
    };
    total.add(&rho_core::Usage::default());
    assert_eq!(total.cost_usd, Some(0.005));

    let mut none = rho_core::Usage::default();
    none.add(&rho_core::Usage::default());
    assert_eq!(none.cost_usd, None, "no data must stay no data, not zero");
}
