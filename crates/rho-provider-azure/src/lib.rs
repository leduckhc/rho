//! The Azure OpenAI provider.
//!
//! It maps the Azure OpenAI `/responses` SSE stream onto the normalised
//! `StreamEvent` model. See `SPEC-02` section 6.
//!
//! The `stream` body and the header builder are `todo!()` on purpose. Stage S5
//! writes the failing tests. Stage S6 fills in the bodies.

use async_trait::async_trait;
use rho_core::{CancelToken, CompletionRequest, Provider, ProviderError, ProviderStream};
use std::fmt;

/// The required Microsoft Entra token audience for Azure OpenAI.
/// The trailing slash is required. Do not change this string.
pub const AZURE_ENTRA_AUDIENCE: &str = "https://cognitiveservices.azure.com/";

/// A credential that never prints itself. See `SPEC-02` section 2.
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

/// The two Azure auth modes. See `SPEC-02` section 6.
#[derive(Clone, Debug)]
pub enum AzureAuth {
    /// API-key mode. It sets the `api-key` header.
    ApiKey(Secret),
    /// Microsoft Entra mode. It sets `Authorization: Bearer <token>`. The token
    /// audience must be `AZURE_ENTRA_AUDIENCE`.
    Entra(Secret),
}

impl AzureAuth {
    /// The one auth header this mode sets: its name and its value.
    ///
    /// API-key mode returns `("api-key", key)`. Entra mode returns
    /// `("Authorization", "Bearer <token>")`. A mode sets only its own header.
    pub fn header(&self) -> (&'static str, String) {
        match self {
            AzureAuth::ApiKey(secret) => {
                let _ = secret;
                todo!("SPEC-02 section 6: build the api-key header")
            }
            AzureAuth::Entra(secret) => {
                let _ = secret;
                todo!("SPEC-02 section 6: build the Authorization Bearer header")
            }
        }
    }
}

/// The Azure provider configuration. It holds the credential in a `Secret`, so
/// its derived `Debug` never leaks the value.
#[derive(Clone, Debug)]
pub struct AzureConfig {
    /// The resource base URL, for example `https://<resource>.openai.azure.com`.
    pub base_url: String,
    /// The deployment name, sent as the request `model`.
    pub deployment: String,
    /// The auth mode.
    pub auth: AzureAuth,
}

impl AzureConfig {
    /// Build a configuration.
    pub fn new(
        base_url: impl Into<String>,
        deployment: impl Into<String>,
        auth: AzureAuth,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            deployment: deployment.into(),
            auth,
        }
    }
}

/// The Azure OpenAI provider.
#[derive(Clone, Debug)]
pub struct AzureProvider {
    config: AzureConfig,
}

impl AzureProvider {
    /// Build the provider from a configuration.
    pub fn new(config: AzureConfig) -> Self {
        Self { config }
    }

    /// The configured base URL.
    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }
}

#[async_trait]
impl Provider for AzureProvider {
    fn id(&self) -> &str {
        "azure"
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError> {
        let _ = (&self.config, &request, &cancel);
        todo!("SPEC-02 section 6: map the Azure Responses SSE stream to StreamEvent")
    }
}
