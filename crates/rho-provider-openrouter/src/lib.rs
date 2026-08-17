//! The OpenRouter provider.
//!
//! It maps the OpenRouter chat-completions SSE stream onto the normalised
//! `StreamEvent` model. See `SPEC-02` section 4.
//!
//! The `stream` body is `todo!()` on purpose. Stage S5 writes the failing
//! tests. Stage S6 fills in the body.

use async_trait::async_trait;
use rho_core::{CancelToken, CompletionRequest, Provider, ProviderError, ProviderStream};

/// The production OpenRouter base URL.
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai";

// `Secret` and `RetryPolicy` live in `rho-core`. See decision D-014.
//
// `Secret` was defined here and also in the Azure crate, and the two copies had
// already drifted: this one masked its `Debug`, the other had no `Debug` at all.
// A leak needs only one weak copy, so the type that guards a secret now has one
// definition and one test suite. `RetryPolicy` moved for the same reason: a policy
// that retries a 401 burns a rate limit on a wrong key, and that rule is stated
// once.
//
// These re-exports keep `rho_provider_openrouter::Secret` valid for a caller.
pub use rho_core::{RetryPolicy, Secret};

/// The OpenRouter provider configuration. It holds the credential in a `Secret`,
/// so its derived `Debug` never leaks the key.
#[derive(Clone, Debug)]
pub struct OpenRouterConfig {
    /// The base URL. Tests point this at a local mock server.
    pub base_url: String,
    /// The API key, wrapped so it never prints.
    pub api_key: Secret,
}

impl OpenRouterConfig {
    /// Build a configuration for the production endpoint.
    pub fn new(api_key: Secret) -> Self {
        Self {
            base_url: OPENROUTER_BASE_URL.to_string(),
            api_key,
        }
    }

    /// Override the base URL. Tests use this to target a mock server.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

/// The OpenRouter provider.
#[derive(Clone, Debug)]
pub struct OpenRouterProvider {
    config: OpenRouterConfig,
}

impl OpenRouterProvider {
    /// Build the provider from a configuration.
    pub fn new(config: OpenRouterConfig) -> Self {
        Self { config }
    }

    /// The configured base URL.
    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }
}

#[async_trait]
impl Provider for OpenRouterProvider {
    fn id(&self) -> &str {
        "openrouter"
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError> {
        // Touch the inputs so the red build has no dead-code warning.
        let _ = (&self.config, &request, &cancel);
        todo!("SPEC-02 section 4: map the OpenRouter SSE stream to StreamEvent")
    }
}
