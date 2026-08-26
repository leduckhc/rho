//! The wire shape of a request, where a fixture cannot help.
//!
//! Every defect this file guards was in a **request**, and every fixture describes a
//! **response**, which is why sprint 1 shipped three providers that no test could clear.

use rho_core::Secret;
use rho_provider_openrouter::OpenRouterConfig;

// ---- A custom base url uses the standard OpenAI path. --------------------

#[test]
fn a_custom_base_url_uses_the_openai_chat_path() {
    // OpenRouter's own path is `/api/v1/chat/completions`. Every other OpenAI-compatible
    // host serves `/v1/chat/completions`, so appending OpenRouter's path to a local host
    // gives a 404. A live probe caught this against a stub that answered any path, which
    // is why a stub must be strict. See `D-a-provider-base-url-is-a-config-key`.
    let config = OpenRouterConfig::new(Secret::new("k")).with_openai_host("http://127.0.0.1:11434");
    assert_eq!(
        config.chat_url(),
        "http://127.0.0.1:11434/v1/chat/completions"
    );
}

#[test]
fn a_custom_base_url_may_carry_the_v1_suffix() {
    // Docs for a local host usually show `http://localhost:11434/v1`, so both forms work
    // and neither doubles the segment.
    for base in [
        "http://127.0.0.1:11434/v1",
        "http://127.0.0.1:11434/v1/",
        "http://127.0.0.1:11434",
    ] {
        let config = OpenRouterConfig::new(Secret::new("k")).with_openai_host(base);
        assert_eq!(
            config.chat_url(),
            "http://127.0.0.1:11434/v1/chat/completions",
            "base {base} must resolve to one endpoint"
        );
    }
}

#[test]
fn the_default_base_url_keeps_the_openrouter_path() {
    let config = OpenRouterConfig::new(Secret::new("k"));
    assert_eq!(
        config.chat_url(),
        "https://openrouter.ai/api/v1/chat/completions"
    );
}
