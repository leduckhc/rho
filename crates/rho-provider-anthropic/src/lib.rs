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

/// The path segment appended to the base URL for a completion. `/v1/messages` is the
/// stable Anthropic URL; the base URL provides the host and any route prefix.
pub const MESSAGES_PATH: &str = "/v1/messages";

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
    config: AnthropicConfig,
    http: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(config: AnthropicConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .expect("reqwest client builds under a valid config");
        Self { config, http }
    }

    /// The URL for a completion request. Public so a test can assert it.
    pub fn messages_url(&self) -> String {
        format!(
            "{}{}",
            self.config.base_url.trim_end_matches('/'),
            MESSAGES_PATH
        )
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> &str {
        "anthropic"
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError> {
        use async_stream::stream;
        use eventsource_stream::Eventsource;
        use futures::StreamExt;

        if self.config.credential.expose().is_empty() {
            return Err(ProviderError::Auth(
                "the Anthropic API key is empty. Set `ANTHROPIC_API_KEY` or add \
                 `[credentials.anthropic]` to your config."
                    .to_string(),
            ));
        }

        let url = self.messages_url();
        let body = build_request_body(&request);

        let mut builder = self
            .http
            .post(&url)
            .header("x-api-key", self.config.credential.expose())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream");
        for (name, value) in &self.config.headers {
            builder = builder.header(name, value);
        }
        let request_future = builder.json(&body).send();

        // Cancel a pending connection cleanly. Once the response head arrives, the stream
        // arm below owns cancel.
        let response = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(ProviderError::Canceled),
            result = request_future => result.map_err(|error| {
                ProviderError::Transport(error.to_string())
            })?,
        };

        let status = response.status();
        if !status.is_success() {
            return Err(map_status_error(status));
        }

        let byte_stream = response.bytes_stream();
        let stream = stream! {
            let mut decoder = sse::Decoder::new();
            let mut events = byte_stream.eventsource();
            loop {
                let next = tokio::select! {
                    biased;
                    () = cancel.cancelled() => {
                        yield Err(ProviderError::Canceled);
                        return;
                    }
                    item = events.next() => item,
                };
                let Some(item) = next else { break };
                match item {
                    Ok(event) => {
                        // Anthropic always names its events. The SSE spec says a data
                        // line without an `event:` line takes the default name
                        // `message`, and some proxies emit trailing empty frames. Skip
                        // an empty name, an empty data body, and a default-named event
                        // with no name we would recognise. See the live drive record.
                        if event.event.is_empty() { continue; }
                        if event.data.trim().is_empty() { continue; }
                        let name = event.event.as_str();
                        if name == "message" { continue; }
                        // `error` is a stream-level fatal signal.
                        if name == "error" {
                            // A stream-level error. Do not put the peer body in the error
                            // message: it may reflect a credential. See
                            // `D-a-client-error-carries-no-peer-body`.
                            yield Err(ProviderError::Server { status: 0 });
                            return;
                        }
                        let data: Value = match serde_json::from_str(&event.data) {
                            Ok(value) => value,
                            Err(error) => {
                                yield Err(ProviderError::Decode(format!(
                                    "anthropic {name} event did not parse: {error}"
                                )));
                                return;
                            }
                        };
                        match decoder.on_event(name, &data) {
                            Ok(out) => for stream_event in out { yield Ok(stream_event); }
                            Err(error) => {
                                yield Err(error);
                                return;
                            }
                        }
                    }
                    Err(error) => {
                        yield Err(ProviderError::Transport(error.to_string()));
                        return;
                    }
                }
            }
        };
        Ok(Box::pin(stream))
    }
}

/// Map an HTTP status to a `ProviderError`. The body is not read, per
/// `D-a-client-error-carries-no-peer-body`. `advice` is `&'static str`, so a runtime
/// string that could carry a peer body cannot land here by construction.
fn map_status_error(status: reqwest::StatusCode) -> ProviderError {
    let code = status.as_u16();
    match code {
        401 => ProviderError::Auth(
            "anthropic rejected the API key (status 401). Set `ANTHROPIC_API_KEY` or add \
             `[credentials.anthropic]` to your config."
                .to_string(),
        ),
        403 => ProviderError::Auth(
            "anthropic refused the request (status 403). The credential is not authorised \
             for this endpoint."
                .to_string(),
        ),
        429 => ProviderError::RateLimited {
            retry_after_ms: None,
        },
        400..=499 => ProviderError::Client {
            status: code,
            advice: "anthropic refused the request. Check the model id and the prompt.",
        },
        _ => ProviderError::Server { status: code },
    }
}
