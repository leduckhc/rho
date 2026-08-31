//! The OpenRouter provider.
//!
//! It maps the OpenRouter chat-completions SSE stream onto the normalised
//! `StreamEvent` model. See `SPEC-provider-interface` section 4.

use async_stream::stream;
use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use rho_core::{
    CancelToken, CompletionRequest, ContentBlock, Message, Provider, ProviderError, ProviderStream,
    Role, StopReason, StreamEvent, Usage,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The production OpenRouter base URL.
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai";

/// The chat-completions path on the base URL.
const CHAT_PATH: &str = "/api/v1/chat/completions";
/// True when a base url names a loopback host, so no proxy may stand between rho and it.
///
/// A proxy variable such as `HTTP_PROXY` can arrive in a `.devcontainer` file or a CI `env:`
/// block, which travel with a clone. `reqwest` honours those variables, so a request to
/// `http://127.0.0.1` was routed to the proxy **with the bearer token in clear text**. A live
/// probe captured `auth=PRESENT: Bearer sk-...` at an attacker's proxy. The rule that allows
/// plain http to a loopback host rests on the traffic never leaving the machine, so rho must
/// make that true rather than assume it.
pub fn bypasses_proxy(base_url: &str) -> bool {
    let Ok(url) = url::Url::parse(base_url) else {
        return false;
    };
    match url.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        Some(url::Host::Domain(name)) => name == "localhost",
        None => false,
    }
}

/// The HTTP client for a base url. A loopback host gets a client with no proxy at all.
fn build_client(base_url: &str) -> reqwest::Client {
    // **No redirect is followed.** The request carries a bearer token. `reqwest` strips the
    // header across origins, and a same-host `https` to `http` downgrade is not a different
    // origin, so a redirect could put the credential on the wire in clear text and defeat
    // `check_base_url`, which exists to prevent exactly that. A security review named it.
    //
    // A 3xx therefore becomes an error the user sees, which is the right answer: an endpoint
    // that redirects a chat request is not the endpoint they named.
    let mut builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
    if bypasses_proxy(base_url) {
        // A loopback host must not go through a proxy. A live probe captured
        // `Bearer sk-...` at an attacker's proxy before this.
        builder = builder.no_proxy();
    }
    // Fail loudly, never fall back to an insecure client. `Client::new()` follows up to
    // ten redirects and honours `HTTP_PROXY`, which are the two behaviours the comment
    // above forbids, each named by a security review or a live probe. A build error here
    // is not a per-request condition a caller can handle: it means the TLS backend did
    // not initialise, which is process-wide and unrecoverable, and no secure fallback
    // exists. So this panics rather than degrade. `OpenRouterProvider::new` is infallible
    // and its two callers in `rho-cli` wrap it in `Ok(Arc::new(..))` with no error arm,
    // so returning a `Result` would only push an unhandleable error up one level. See
    // the `ToolKind::Other` fail-open defect in AGENTS.md step 8.
    builder
        .build()
        .expect("the reqwest TLS backend failed to initialise")
}

/// The path every other OpenAI-compatible host serves.
///
/// OpenRouter puts its chat endpoint under `/api`. Ollama, vLLM, LiteLLM, and LM Studio do
/// not. Appending OpenRouter's path to a local host gives a 404, and a live probe caught
/// exactly that. See `D-a-provider-base-url-is-a-config-key`.
const OPENAI_CHAT_PATH: &str = "/v1/chat/completions";

// `Secret` and `RetryPolicy` live in `rho-core`. See decision D-secret-in-core.
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
    /// The retry policy for the initial request.
    pub retry: RetryPolicy,
    /// The chat path appended to `base_url`. OpenRouter's own path by default, and the
    /// standard OpenAI one when a caller set a base url.
    chat_path: String,
}

impl OpenRouterConfig {
    /// Build a configuration for the production endpoint.
    pub fn new(api_key: Secret) -> Self {
        Self {
            base_url: OPENROUTER_BASE_URL.to_string(),
            chat_path: CHAT_PATH.to_string(),
            api_key,
            retry: RetryPolicy::default(),
        }
    }

    /// Override the base URL. Tests use this to target a mock server.
    /// The full chat endpoint this config sends to.
    pub fn chat_url(&self) -> String {
        format!("{}{}", self.base_url, self.chat_path)
    }

    /// Point at an OpenAI-compatible host, such as Ollama, vLLM, or LM Studio.
    ///
    /// It sets the standard OpenAI chat path, because OpenRouter serves its endpoint under
    /// `/api` and no other host does. A live probe against a stub that answered any path
    /// hid this, so a stub must be strict. See `D-a-provider-base-url-is-a-config-key`.
    ///
    /// A local host's own documentation usually shows the `/v1` suffix, so both forms work
    /// and neither doubles the segment.
    pub fn with_openai_host(mut self, base_url: impl Into<String>) -> Self {
        let given = base_url.into();
        let trimmed = given.trim_end_matches('/');
        let root = trimmed.strip_suffix("/v1").unwrap_or(trimmed);
        self.base_url = root.to_string();
        self.chat_path = OPENAI_CHAT_PATH.to_string();
        self
    }

    /// Point at another URL that serves OpenRouter's own path. A test uses it for a mock.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Override the retry policy.
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }
}

/// The OpenRouter provider.
#[derive(Clone, Debug)]
pub struct OpenRouterProvider {
    config: OpenRouterConfig,
    client: reqwest::Client,
}

impl OpenRouterProvider {
    /// Build the provider from a configuration.
    pub fn new(config: OpenRouterConfig) -> Self {
        let client = build_client(&config.base_url);
        Self { config, client }
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
        if self.config.api_key.is_empty() {
            return Err(ProviderError::Auth(
                "the OpenRouter API key is empty. Set OPENROUTER_API_KEY.".to_string(),
            ));
        }
        let url = self.config.chat_url();
        let body = build_request_body(&request);

        // Retry only before the first event. Once the response head arrives, a
        // mid-stream error ends the stream and is never retried.
        let response = self.send_with_retry(&url, &body, &cancel).await?;
        tracing::debug!(
            path = CHAT_PATH,
            status = response.status().as_u16(),
            "openrouter stream open"
        );

        let byte_stream = response.bytes_stream();
        let stream = stream! {
            let mut events = byte_stream.eventsource();
            let mut state = SseState::default();
            loop {
                let next = tokio::select! {
                    biased;
                    () = cancel.cancelled() => break,
                    item = events.next() => item,
                };
                let Some(item) = next else { break };
                match item {
                    Ok(event) => {
                        if event.data == "[DONE]" {
                            break;
                        }
                        let chunk: Chunk = match serde_json::from_str(&event.data) {
                            Ok(chunk) => chunk,
                            Err(error) => {
                                yield Err(ProviderError::Decode(format!(
                                    "the OpenRouter chunk did not parse as JSON: {error}"
                                )));
                                return;
                            }
                        };
                        let outcome = state.map_chunk(chunk);
                        for event in outcome.events {
                            yield Ok(event);
                        }
                        if let Some(error) = outcome.error {
                            yield Err(error);
                            return;
                        }
                        if outcome.done {
                            return;
                        }
                    }
                    Err(error) => {
                        yield Err(ProviderError::Transport(error.to_string()));
                        return;
                    }
                }
            }
            // The stream ended. Emit the turn end now, so a usage chunk that arrived
            // after the finish chunk has already been reported. See decision D-measured-cost-and-cache.
            if let Some(stop_reason) = state.pending_stop.take() {
                yield Ok(StreamEvent::Done { stop_reason });
            }
        };
        Ok(Box::pin(stream))
    }
}

impl OpenRouterProvider {
    /// Send the request, and retry a transient failure before the first event.
    async fn send_with_retry(
        &self,
        url: &str,
        body: &Value,
        cancel: &CancelToken,
    ) -> Result<reqwest::Response, ProviderError> {
        let policy = self.config.retry;
        let mut attempt: u32 = 1;
        loop {
            let error = match self.send_once(url, body).await {
                Ok(response) => return Ok(response),
                Err(error) => error,
            };
            let hint = match &error {
                ProviderError::RateLimited { retry_after_ms } => *retry_after_ms,
                _ => None,
            };
            if !policy.should_retry(&error, attempt) {
                return Err(error);
            }
            let Some(delay) = policy.backoff(attempt, hint) else {
                return Err(error);
            };
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(ProviderError::Canceled),
                () = tokio::time::sleep(delay) => {}
            }
            attempt += 1;
        }
    }

    /// Send one request. Map a non-200 status to a provider error.
    async fn send_once(&self, url: &str, body: &Value) -> Result<reqwest::Response, ProviderError> {
        let response = self
            .client
            .post(url)
            .bearer_auth(self.config.api_key.expose())
            .json(body)
            .send()
            .await
            .map_err(|error| ProviderError::Transport(error.to_string()))?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let retry_after_ms = parse_retry_after(&response);
        // The body is not read. No arm of `status_to_error` uses it, and a host that reflects
        // the `Authorization` header would otherwise put the credential on rho's stderr
        // through `ProviderError::Client`. Not reading it is stronger than scrubbing it, because
        // the bytes never enter the process. See `D-a-client-error-carries-no-peer-body`.
        Err(status_to_error(status.as_u16(), retry_after_ms))
    }
}

/// Read a `Retry-After` header, in seconds, and convert it to milliseconds.
fn parse_retry_after(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(|seconds| seconds * 1_000)
}

/// Map an HTTP status to a provider error. See `SPEC-provider-interface` section 4.
/// What rho tells a user about a 4xx, instead of the peer's body.
///
/// It is a `&'static str`, so the peer's bytes cannot take its place. A local gateway is a
/// process the user runs, so its own log holds the reason and rho does not have to relay
/// untrusted bytes to find it. See `D-a-client-error-carries-no-peer-body`.
const CLIENT_ADVICE: &str = "the provider refused the request. rho does not show the body, \
     because a body can echo the credential. Read the host's own log for the reason.";

/// Map an HTTP status to a provider error.
fn status_to_error(status: u16, retry_after_ms: Option<u64>) -> ProviderError {
    match status {
        429 => ProviderError::RateLimited { retry_after_ms },
        500..=599 => ProviderError::Server { status },
        401 | 403 => ProviderError::Auth(format!(
            "OpenRouter rejected the API key (status {status}). Set a valid OPENROUTER_API_KEY."
        )),
        _ => ProviderError::Client {
            status,
            advice: CLIENT_ADVICE,
        },
    }
}

// --- The chunk shape. ----------------------------------------------------

#[derive(Debug, Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<OpenRouterUsage>,
    #[serde(default)]
    error: Option<ChunkError>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    #[serde(default)]
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    /// The name pi added for llama.cpp. Some hosts send this instead of `reasoning`.
    #[serde(default)]
    reasoning_content: Option<String>,
    /// A third name some hosts use for the same reasoning text.
    #[serde(default)]
    reasoning_text: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallFragment>>,
}

impl Delta {
    /// The reasoning text for this delta, reading the accepted field names in order and
    /// taking the first non-empty one. One host sends two fields with the same text, so a
    /// reader that added every field would double it. An empty field is skipped, so an
    /// empty reasoning delta starts no block. See SPEC-reasoning-across-providers section 3.
    fn first_reasoning(&self) -> Option<&str> {
        [
            self.reasoning_content.as_deref(),
            self.reasoning.as_deref(),
            self.reasoning_text.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find(|text| !text.is_empty())
    }
}

#[derive(Debug, Deserialize)]
struct ToolCallFragment {
    index: u32,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<FunctionFragment>,
}

#[derive(Debug, Deserialize)]
struct FunctionFragment {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    /// Cache counts, when the service reports them.
    ///
    /// The field names come from a live probe of the API, not from memory:
    /// `usage.prompt_tokens_details.cached_tokens` and `cache_write_tokens`. rho used to
    /// report zero for both, so a user could not see the cache saving that this project
    /// works to earn. See decision D-measured-cost-and-cache.
    #[serde(default)]
    prompt_tokens_details: Option<OpenRouterPromptDetails>,
    /// The real cost of the call, in dollars, as the service charged it.
    ///
    /// This is measured, not estimated from a price table, so it stays right when a price
    /// changes or a request falls back to another model.
    #[serde(default)]
    cost: Option<f64>,
}

/// The cache breakdown inside `usage.prompt_tokens_details`.
#[derive(Debug, Default, Deserialize)]
struct OpenRouterPromptDetails {
    #[serde(default)]
    cached_tokens: u64,
    #[serde(default)]
    cache_write_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct ChunkError {
    #[serde(default)]
    code: Option<u16>,
}

// --- The mapping state. --------------------------------------------------

/// The buffered state for one tool call, keyed by its `index`.
#[derive(Default)]
struct ToolAccum {
    buffer: String,
}

/// The state the mapping carries across chunks.
#[derive(Default)]
struct SseState {
    message_started: bool,
    /// The stop reason seen on the finish chunk, held until the stream really ends.
    /// See the comment in `map_chunk`, and decision D-measured-cost-and-cache.
    pending_stop: Option<StopReason>,
    text_open: bool,
    thinking_open: bool,
    tool_calls: BTreeMap<u32, ToolAccum>,
}

/// The result of mapping one chunk.
struct ChunkOutcome {
    events: Vec<StreamEvent>,
    error: Option<ProviderError>,
    done: bool,
}

impl SseState {
    /// The single text-block index. OpenRouter carries text on choice index 0.
    const TEXT_INDEX: u32 = 0;

    fn map_chunk(&mut self, chunk: Chunk) -> ChunkOutcome {
        let mut events = Vec::new();

        // A mid-stream error must surface as `Err`, not a silent truncation.
        if let Some(error) = &chunk.error {
            let status = error.code.unwrap_or(500);
            return ChunkOutcome {
                events,
                error: Some(ProviderError::Server { status }),
                done: true,
            };
        }

        if !self.message_started {
            events.push(StreamEvent::MessageStart {
                role: Role::Assistant,
            });
            self.message_started = true;
        }

        let choice = chunk.choices.into_iter().next();
        if let Some(choice) = &choice {
            if let Some(text) = &choice.delta.content {
                if !self.text_open {
                    events.push(StreamEvent::TextStart {
                        index: Self::TEXT_INDEX,
                    });
                    self.text_open = true;
                }
                events.push(StreamEvent::TextDelta {
                    index: Self::TEXT_INDEX,
                    delta: text.clone(),
                });
            }
            if let Some(reasoning) = choice.delta.first_reasoning() {
                if !self.thinking_open {
                    events.push(StreamEvent::ThinkingStart {
                        index: Self::TEXT_INDEX,
                    });
                    self.thinking_open = true;
                }
                events.push(StreamEvent::ThinkingDelta {
                    index: Self::TEXT_INDEX,
                    delta: reasoning.to_string(),
                });
            }
            if let Some(fragments) = &choice.delta.tool_calls {
                for fragment in fragments {
                    self.map_tool_fragment(fragment, &mut events);
                }
            }
        }

        if let Some(usage) = chunk.usage {
            let details = usage.prompt_tokens_details.unwrap_or_default();
            events.push(StreamEvent::Usage(Usage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
                cache_read_tokens: details.cached_tokens,
                cache_write_tokens: details.cache_write_tokens,
                cost_usd: usage.cost,
            }));
        }

        if let Some(finish) = choice.and_then(|choice| choice.finish_reason) {
            if let Some(error) = self.close_blocks(&mut events) {
                return ChunkOutcome {
                    events,
                    error: Some(error),
                    done: true,
                };
            }
            // Hold the stop reason. Do not end the stream here.
            //
            // OpenRouter sends the whole `usage` object in a **later** chunk, after this
            // one. rho used to return `done: true` at this point, so it never saw usage
            // for any call: no tokens, no cost, no cache. A fifty-session live run
            // reporting zero tokens is what exposed it. See decision D-measured-cost-and-cache.
            //
            // `Done` is emitted at `[DONE]`, or on the next chunk if the service sends no
            // `[DONE]`, so usage always precedes it.
            self.pending_stop = Some(finish_reason_to_stop(&finish));
            return ChunkOutcome {
                events,
                error: None,
                done: false,
            };
        }

        ChunkOutcome {
            events,
            error: None,
            done: false,
        }
    }

    /// Assemble one tool-call fragment into its per-index accumulator.
    fn map_tool_fragment(&mut self, fragment: &ToolCallFragment, events: &mut Vec<StreamEvent>) {
        let index = fragment.index;
        let is_new = !self.tool_calls.contains_key(&index);
        let accum = self.tool_calls.entry(index).or_default();
        if is_new {
            // The start fragment carries the id and the function name.
            let id = fragment.id.clone().unwrap_or_default();
            let name = fragment
                .function
                .as_ref()
                .and_then(|function| function.name.clone())
                .unwrap_or_default();
            events.push(StreamEvent::ToolCallStart { index, id, name });
        }
        if let Some(function) = &fragment.function
            && let Some(arguments) = &function.arguments
            && !arguments.is_empty()
        {
            accum.buffer.push_str(arguments);
            events.push(StreamEvent::ToolCallDelta {
                index,
                delta: arguments.clone(),
            });
        }
    }

    /// Close the open blocks at the end of the turn. Return a decode error when a
    /// tool-call buffer does not parse.
    fn close_blocks(&mut self, events: &mut Vec<StreamEvent>) -> Option<ProviderError> {
        if self.text_open {
            events.push(StreamEvent::TextEnd {
                index: Self::TEXT_INDEX,
            });
            self.text_open = false;
        }
        if self.thinking_open {
            events.push(StreamEvent::ThinkingEnd {
                index: Self::TEXT_INDEX,
                // This wire carries no replay token, so the reducer keeps a trace.
                state: None,
            });
            self.thinking_open = false;
        }
        let tool_calls = std::mem::take(&mut self.tool_calls);
        for (index, accum) in tool_calls {
            let arguments = if accum.buffer.trim().is_empty() {
                Value::Object(serde_json::Map::new())
            } else {
                match serde_json::from_str(&accum.buffer) {
                    Ok(value) => value,
                    Err(error) => {
                        return Some(ProviderError::Decode(format!(
                            "the tool-call arguments did not parse as JSON: {error}"
                        )));
                    }
                }
            };
            events.push(StreamEvent::ToolCallEnd {
                index,
                arguments,
                state: None,
            });
        }
        None
    }
}

/// Map an OpenRouter finish reason to a normalised stop reason.
fn finish_reason_to_stop(reason: &str) -> StopReason {
    match reason {
        "stop" => StopReason::EndTurn,
        "length" => StopReason::MaxTokens,
        "tool_calls" => StopReason::ToolUse,
        "content_filter" => StopReason::ContentFiltered,
        _ => StopReason::EndTurn,
    }
}

// --- The request body. ---------------------------------------------------

/// Build the chat-completions request body. See `SPEC-provider-interface` section 4.
/// The `reasoning` field for one effort level, or `None` when rho must send nothing.
///
/// A review found that this crate ignored the level entirely: the agent carried it, Bedrock
/// consumed it, and here `unset` and `set` collapsed to the same wire. Silence is the defect,
/// so the level now travels.
///
/// OpenRouter accepts `minimal`, `low`, `medium`, and `high`. It has no `xhigh`, and rho must
/// not invent a value a host rejects, because an unknown field is a failed turn. So `xhigh`
/// maps to `high`, and the mapping lives here rather than in four call sites.
fn reasoning_field(effort: Option<rho_core::ReasoningEffort>) -> Option<Value> {
    use rho_core::ReasoningEffort;
    match effort? {
        // An instruction, not a silence. A host that thinks by default is told to stop.
        ReasoningEffort::Off => Some(json!({ "enabled": false })),
        ReasoningEffort::Low => Some(json!({ "effort": "low" })),
        ReasoningEffort::Medium => Some(json!({ "effort": "medium" })),
        ReasoningEffort::High | ReasoningEffort::XHigh => Some(json!({ "effort": "high" })),
    }
}

pub fn build_request_body(request: &CompletionRequest) -> Value {
    let mut messages = Vec::new();
    if let Some(system) = &request.system {
        messages.push(json!({ "role": "system", "content": system }));
    }
    for message in &request.messages {
        messages.push(message_to_json(message));
    }

    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "stream": true,
        // Ask for the accounting, or none arrives.
        //
        // OpenRouter omits `usage` from a streamed response unless the request opts in.
        // So rho parsed the cache and cost fields correctly and never received them. A
        // live run of fifty sessions reported zero tokens and no cost, which is what
        // exposed it. This is the same shape as the timeout guidance that never fired:
        // the code was right and unreachable. See decision D-measured-cost-and-cache.
        "usage": { "include": true },
    });
    let map = body.as_object_mut().expect("the body is an object");
    if let Some(reasoning) = reasoning_field(request.reasoning) {
        map.insert("reasoning".to_string(), reasoning);
    }
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema,
                    }
                })
            })
            .collect();
        map.insert("tools".to_string(), Value::Array(tools));
    }
    if let Some(max_tokens) = request.max_tokens {
        map.insert("max_tokens".to_string(), json!(max_tokens));
    }
    if let Some(temperature) = request.temperature {
        map.insert("temperature".to_string(), json!(temperature));
    }
    body
}

/// Map one normalised message to the OpenAI chat message shape.
fn message_to_json(message: &Message) -> Value {
    let role = match message.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut tool_call_id: Option<String> = None;
    for block in &message.content {
        match block {
            ContentBlock::Text { text: piece } => text.push_str(piece),
            ContentBlock::ToolCall {
                id,
                name,
                arguments,
                // No replay payload travels on this wire yet. Gemini binds one to a call.
                state: _,
            } => {
                tool_calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments.to_string(),
                    }
                }));
            }
            ContentBlock::ToolResult {
                tool_call_id: id,
                content,
                ..
            } => {
                tool_call_id = Some(id.clone());
                for inner in content {
                    if let ContentBlock::Text { text: piece } = inner {
                        text.push_str(piece);
                    }
                }
            }
            // A trace is for the reader, so it never travels.
            ContentBlock::ReasoningTrace { .. } => {}
            // A replay block does not travel to OpenRouter yet. The hosts behind this one
            // wire format disagree: Kimi and DeepSeek require the field back, and Mistral
            // answers 422 when it is present. rho has no host row and no live proof for
            // either, so it sends nothing and says so here. A wrong guess is a broken turn
            // in both directions. See SPEC-reasoning-across-providers section 3 "One".
            ContentBlock::ReasoningReplay { .. } => {}
            ContentBlock::Image { .. } => {}
        }
    }

    let mut value = json!({ "role": role, "content": text });
    let map = value.as_object_mut().expect("the message is an object");
    if !tool_calls.is_empty() {
        map.insert("tool_calls".to_string(), Value::Array(tool_calls));
    }
    if let Some(id) = tool_call_id {
        map.insert("tool_call_id".to_string(), Value::String(id));
    }
    value
}
