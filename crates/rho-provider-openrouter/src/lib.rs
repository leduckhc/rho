//! The OpenRouter provider.
//!
//! It maps the OpenRouter chat-completions SSE stream onto the normalised
//! `StreamEvent` model. See `SPEC-02` section 4.
//!
//! The `stream` body is `todo!()` on purpose. Stage S5 writes the failing
//! tests. Stage S6 fills in the body.

use async_trait::async_trait;
use rho_core::{CancelToken, CompletionRequest, Provider, ProviderError, ProviderStream};
use std::fmt;
use std::time::Duration;

/// The production OpenRouter base URL.
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai";

/// A credential that never prints itself. See `SPEC-02` section 2.
///
/// The type has no `Display`. Its `Debug` prints a fixed mask. A test proves the
/// mask holds. This redacts secrets by construction, not by a late filter.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    /// Wrap a credential value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Read the value. The caller must never log the result.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

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

/// The auto-retry policy. See `SPEC-02` section 3.
///
/// The policy wraps a provider. It reads `ProviderError::is_retryable`. It
/// retries `Transport`, `Server`, and `RateLimited`. It never retries `Client`,
/// `Decode`, or `Auth`.
///
/// This type belongs to the provider layer. `SPEC-02` places it in `rho-core`,
/// but `rho-core` did not export it and stage S5 may not edit `rho-core`. The
/// report records this. A later refactor may hoist it.
#[derive(Clone, Copy, Debug)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay_ms: 500,
            max_delay_ms: 30_000,
        }
    }
}

impl RetryPolicy {
    /// The delay before attempt `attempt`, one-based, given an optional hint.
    ///
    /// A server hint skips the jitter. A return of `None` means do not retry.
    pub fn backoff(&self, attempt: u32, retry_after_ms: Option<u64>) -> Option<Duration> {
        let _ = (
            self.base_delay_ms,
            self.max_delay_ms,
            attempt,
            retry_after_ms,
        );
        todo!("SPEC-02 section 3: exponential backoff with full jitter")
    }
}
