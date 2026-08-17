//! The provider interface.
//!
//! A provider turns a `CompletionRequest` into a stream of `StreamEvent`. The
//! core owns the trait. Each provider crate implements it.

use crate::{CancelToken, Message, ProviderError, StreamEvent, ToolKind};
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

/// A boxed, sendable stream of normalised events.
pub type ProviderStream =
    Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>;

/// One tool, as advertised to the model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// The ACP tool category. See `ToolKind` in `SPEC-03`.
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
}
