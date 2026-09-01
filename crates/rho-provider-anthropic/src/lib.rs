//! The Anthropic Messages API provider.
//!
//! rho reaches the Anthropic Messages API over HTTPS, with a configurable base URL. The
//! same crate speaks real `api.anthropic.com` and any compatible proxy, including the
//! xdent tunnel. See `SPEC-anthropic-messages-provider` for the wire contract.
//!
//! The wire lives in the `wire` module. The public entry points are `AnthropicConfig` and
//! `AnthropicProvider`. Both take a resolved `Secret`, which the CLI resolves through the
//! merged credentials table or the `ANTHROPIC_API_KEY` fallback, per
//! `D-a-provider-names-its-own-credential`.

pub mod sse;

use async_trait::async_trait;
use rho_core::{CancelToken, CompletionRequest, Provider, ProviderError, ProviderStream, Secret};
use serde_json::Value;
use std::time::Duration;

/// The default endpoint. It reaches Anthropic itself. A configured entry overrides it.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// The header this provider always sends. The spec pins this version, and a bump belongs to
/// its own decision.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// The default request timeout. Provisional, per the review amendment on
/// `SPEC-anthropic-messages-provider`. Ninety seconds is a first-byte cap for a reasoning
/// turn; a measurement takes precedence.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(90);

/// Configuration for one Anthropic provider instance.
///
/// Every field is set at construction time. The credential is already resolved: this crate
/// does not read the environment or the credentials table. The named-entry lookup, or the
/// built-in caller with `_or_env` fallback, does the resolution.
#[derive(Clone, Debug)]
pub struct AnthropicConfig {
    /// The base URL, without a trailing slash. `POST /v1/messages` appends to it.
    pub base_url: String,
    /// The resolved API key. Sent as `x-api-key`.
    pub credential: Secret,
    /// Extra HTTP headers, sent on every request. The spec forbids overriding
    /// `x-api-key`, `anthropic-version`, and `content-type`.
    pub headers: Vec<(String, String)>,
    /// The wall-clock request timeout.
    pub timeout: Duration,
}

/// The `max_tokens` this provider sends when the caller supplied none. Anthropic 400s a
/// request that omits the field, so we send a number. Four thousand is enough for a normal
/// answer and small enough that a caller with a tighter budget can lower it explicitly.
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Build the JSON request body from a `CompletionRequest`. Free function, so both the
/// production caller and every test call the same code.
///
/// See `SPEC-anthropic-messages-provider` section 3. The rules that decide the shape:
///
/// - `system` is a top-level field on Anthropic, not a message. A message role is `user`
///   or `assistant`, never `system`.
/// - `max_tokens` is required. An absent value sends `DEFAULT_MAX_TOKENS`, so the request
///   does not 400.
/// - `stream` is always `true`. rho streams every turn.
/// - Optional keys are absent when the caller supplies nothing. A silent `null` is a
///   defect: it can mean "clear the value" on some providers.
pub fn build_request_body(request: &CompletionRequest) -> Value {
    let mut body = serde_json::Map::new();
    body.insert("model".into(), Value::String(request.model.clone()));
    body.insert(
        "max_tokens".into(),
        Value::from(request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS)),
    );
    body.insert("stream".into(), Value::Bool(true));
    if let Some(system) = &request.system {
        body.insert("system".into(), Value::String(system.clone()));
    }
    let messages: Vec<Value> = request.messages.iter().map(message_to_json).collect();
    body.insert("messages".into(), Value::Array(messages));
    Value::Object(body)
}

/// One conversation message, in the Anthropic wire shape.
///
/// A message's `content` is always an array of blocks. A bare string works on Anthropic
/// for a text-only user message, but rho carries structured blocks, so the array form is
/// the honest shape.
fn message_to_json(message: &rho_core::Message) -> Value {
    use rho_core::Role;
    let role = match message.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        // Tool results ride on a user turn on Anthropic. See
        // `SPEC-anthropic-messages-provider` section 4.
        Role::Tool => "user",
    };
    let content: Vec<Value> = message.content.iter().map(content_block_to_json).collect();
    serde_json::json!({
        "role": role,
        "content": content,
    })
}

/// One content block, in the Anthropic wire shape.
fn content_block_to_json(block: &rho_core::ContentBlock) -> Value {
    use rho_core::ContentBlock as B;
    match block {
        B::Text { text } => serde_json::json!({ "type": "text", "text": text }),
        // Every arm the rest of the flow will fill lives here as a placeholder, so a new
        // arm is a compile error instead of a silent drop. Section 4 of the spec fills
        // each one in the next test round.
        other => {
            let _ = other;
            todo!("content block kind not yet mapped: see section 4")
        }
    }
}

impl AnthropicConfig {
    /// A new config against the given base URL, with default headers and timeout.
    pub fn new(base_url: impl Into<String>, credential: Secret) -> Self {
        Self {
            base_url: base_url.into(),
            credential,
            headers: Vec::new(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// A new config against the default endpoint, `api.anthropic.com`.
    pub fn against_anthropic(credential: Secret) -> Self {
        Self::new(DEFAULT_BASE_URL, credential)
    }
}

/// The Anthropic Messages provider.
pub struct AnthropicProvider {
    _config: AnthropicConfig,
}

impl AnthropicProvider {
    pub fn new(config: AnthropicConfig) -> Self {
        Self { _config: config }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> &str {
        "anthropic"
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
        _cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError> {
        todo!("build the request, POST /v1/messages, and map SSE onto StreamEvent")
    }
}
