//! The provider interface.
//!
//! A provider turns a `CompletionRequest` into a stream of `StreamEvent`. The
//! core owns the trait. Each provider crate implements it.

use crate::{CancelToken, Message, ProviderError, StreamEvent, ToolKind};
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Duration;

/// A boxed, sendable stream of normalised events.
pub type ProviderStream = Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>;

/// One tool, as advertised to the model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// The ACP tool category. See `ToolKind` in `SPEC-tool-interface`.
    pub kind: ToolKind,
    /// A JSON Schema object for the tool arguments.
    pub input_schema: serde_json::Value,
}

/// One inference request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// The provider-specific model id or ARN.
    pub model: String,
    /// The system prompt. Part of the stable prefix.
    pub system: Option<String>,
    /// The full conversation, oldest first.
    pub messages: Vec<Message>,
    /// The full tool list. Fixed for the session. Part of the stable prefix.
    pub tools: Vec<ToolSpec>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    /// How hard the model should think. `None` means the provider's own default, so the
    /// provider sends no field. See `SPEC-reasoning-across-providers` section 9.
    pub reasoning: Option<crate::ReasoningEffort>,
}

#[async_trait]
pub trait Provider: Send + Sync {
    /// A stable id, for example `openrouter`, `bedrock`, or `azure`.
    fn id(&self) -> &str;

    /// Start a streaming completion. The future resolves once the response
    /// headers arrive. The stream then yields events. The provider must select
    /// against `cancel.cancelled()` and stop the request when it fires.
    async fn stream(
        &self,
        request: CompletionRequest,
        cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError>;

    /// The model catalogue for this provider, or `None` when the provider cannot list.
    ///
    /// Azure returns `None`, because it names deployments, not models. Bedrock and
    /// OpenRouter return `Some(self)`.
    fn catalog(&self) -> Option<&dyn ModelCatalog>;

    /// A fingerprint for the model catalogue cache. It changes when the endpoint changes,
    /// so a new base URL or a new region writes a new cache entry. The default is the
    /// provider id; providers with configurable endpoints override it.
    fn catalog_fingerprint(&self) -> String {
        self.id().to_string()
    }
}

/// The most models rho keeps from one listing.
pub const MAX_MODELS: usize = 2_000;
/// The longest rho waits for a listing.
pub const LIST_TIMEOUT: Duration = Duration::from_secs(10);

/// One model a provider offers. It carries the wire id and an optional label.
///
/// It carries no capability claim. A listing proves a model exists. It never proves the
/// model calls tools. See `SPEC-choose-a-model-and-configure-a-run` section 5.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    /// The exact value rho puts in `CompletionRequest.model`. Nothing transforms it.
    pub id: String,
    /// A human label from the provider, when it gives one. `None` shows `id`. rho never
    /// invents a label, because an invented label is a claim rho cannot prove.
    pub display_name: Option<String>,
}

/// A provider that can list its models. A provider that cannot does not implement this.
#[async_trait]
pub trait ModelCatalog: Send + Sync {
    /// List the models this provider offers. This is a network call.
    ///
    /// An empty `Ok(vec)` means the provider listed nothing. It is not the same as a
    /// provider that cannot list, which returns `None` from `Provider::catalog`.
    ///
    /// The call must select against `cancel.cancelled()` and stop when it fires.
    async fn list_models(&self, cancel: CancelToken)
    -> Result<Vec<ModelDescriptor>, ProviderError>;

    /// Return any cached models for the current fingerprint, stale or fresh, without a
    /// network call. Providers without a cache return `None`. The default is `None`,
    /// because most catalog implementations have no on-disk cache.
    fn peek_cached_models(&self) -> Option<Vec<ModelDescriptor>> {
        None
    }
}
