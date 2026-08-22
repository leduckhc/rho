//! The Azure OpenAI provider.
//!
//! It maps the Azure OpenAI `/responses` SSE stream onto the normalised
//! `StreamEvent` model. See `SPEC-provider-interface` section 6.

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
use std::collections::{HashMap, HashSet};

/// The required Microsoft Entra token audience for Azure OpenAI.
/// The trailing slash is required. Do not change this string.
pub const AZURE_ENTRA_AUDIENCE: &str = "https://cognitiveservices.azure.com/";

/// The Responses path on the resource base URL.
const RESPONSES_PATH: &str = "/openai/v1/responses";

// `Secret` lives in `rho-core`. See decision D-secret-in-core. This crate once defined its
// own copy with no `Debug` mask at all, while the OpenRouter copy masked itself.
// That drift is why the type now has one home.
pub use rho_core::{RetryPolicy, Secret};

/// The two Azure auth modes. See `SPEC-provider-interface` section 6.
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
            AzureAuth::ApiKey(secret) => ("api-key", secret.expose().to_string()),
            AzureAuth::Entra(secret) => ("Authorization", format!("Bearer {}", secret.expose())),
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
    /// The retry policy for the initial request.
    pub retry: RetryPolicy,
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
            retry: RetryPolicy::default(),
        }
    }

    /// Override the retry policy.
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }
}

/// The Azure OpenAI provider.
#[derive(Clone, Debug)]
pub struct AzureProvider {
    config: AzureConfig,
    client: reqwest::Client,
}

impl AzureProvider {
    /// Build the provider from a configuration.
    pub fn new(config: AzureConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
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
        let url = format!("{}{RESPONSES_PATH}", self.config.base_url);
        let body = build_request_body(&request, &self.config.deployment);

        let response = self.send_with_retry(&url, &body, &cancel).await?;
        tracing::debug!(
            path = RESPONSES_PATH,
            status = response.status().as_u16(),
            "azure stream open"
        );

        let byte_stream = response.bytes_stream();
        let stream = stream! {
            let mut events = byte_stream.eventsource();
            let mut state = ResponsesState::default();
            loop {
                let next = tokio::select! {
                    biased;
                    () = cancel.cancelled() => break,
                    item = events.next() => item,
                };
                let Some(item) = next else { break };
                match item {
                    Ok(event) => {
                        if event.data.trim().is_empty() || event.data == "[DONE]" {
                            continue;
                        }
                        let parsed: ResponsesEvent = match serde_json::from_str(&event.data) {
                            Ok(parsed) => parsed,
                            Err(error) => {
                                yield Err(ProviderError::Decode(format!(
                                    "the Azure event did not parse as JSON: {error}"
                                )));
                                return;
                            }
                        };
                        let outcome = state.map_event(parsed);
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
        };
        Ok(Box::pin(stream))
    }
}

impl AzureProvider {
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
        let (header_name, header_value) = self.config.auth.header();
        let response = self
            .client
            .post(url)
            .header(header_name, header_value)
            .json(body)
            .send()
            .await
            .map_err(|error| ProviderError::Transport(error.to_string()))?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let retry_after_ms = parse_retry_after(&response);
        let message = response.text().await.unwrap_or_default();
        Err(status_to_error(status.as_u16(), retry_after_ms, message))
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

/// Map an HTTP status to a provider error.
fn status_to_error(status: u16, retry_after_ms: Option<u64>, message: String) -> ProviderError {
    match status {
        429 => ProviderError::RateLimited { retry_after_ms },
        500..=599 => ProviderError::Server { status },
        401 | 403 => ProviderError::Auth(format!(
            "Azure rejected the credential (status {status}). Check the api-key or the Entra token audience {AZURE_ENTRA_AUDIENCE}."
        )),
        _ => ProviderError::Client { status, message },
    }
}

// --- The event shape. ----------------------------------------------------

#[derive(Debug, Deserialize)]
struct ResponsesEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    output_index: Option<u32>,
    #[serde(default)]
    delta: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
    #[serde(default)]
    item: Option<ResponseItem>,
    #[serde(default)]
    response: Option<ResponseObject>,
    #[serde(default)]
    error: Option<ResponseError>,
}

#[derive(Debug, Deserialize)]
struct ResponseItem {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResponseObject {
    #[serde(default)]
    usage: Option<AzureUsage>,
    #[serde(default)]
    output: Vec<OutputItem>,
}

#[derive(Debug, Deserialize)]
struct OutputItem {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct AzureUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    /// Cache counts, when the service reports them.
    ///
    /// The field names come from a live probe of the endpoint, not from memory:
    /// `usage.input_tokens_details.cached_tokens` and `cache_write_tokens`. rho used to
    /// report zero for both. See decision D-measured-cost-and-cache.
    #[serde(default)]
    input_tokens_details: Option<AzureInputTokenDetails>,
}

/// The cache breakdown inside `usage.input_tokens_details`.
#[derive(Clone, Debug, Default, Deserialize)]
struct AzureInputTokenDetails {
    #[serde(default)]
    cached_tokens: u64,
    #[serde(default)]
    cache_write_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct ResponseError {
    #[serde(default)]
    code: Option<u16>,
    #[serde(default)]
    message: Option<String>,
}

// --- The mapping state. --------------------------------------------------

/// The result of mapping one event.
struct EventOutcome {
    events: Vec<StreamEvent>,
    error: Option<ProviderError>,
    done: bool,
}

/// The state the mapping carries across events.
#[derive(Default)]
struct ResponsesState {
    message_started: bool,
    text_started: HashSet<u32>,
    thinking_started: HashSet<u32>,
    tool_inputs: HashMap<u32, String>,
}

impl ResponsesState {
    fn map_event(&mut self, event: ResponsesEvent) -> EventOutcome {
        let mut events = Vec::new();
        let index = event.output_index.unwrap_or(0);

        match event.kind.as_str() {
            "response.created" if !self.message_started => {
                events.push(StreamEvent::MessageStart {
                    role: Role::Assistant,
                });
                self.message_started = true;
            }
            "response.output_item.added" => {
                if let Some(item) = &event.item
                    && item.kind == "function_call"
                {
                    self.tool_inputs.entry(index).or_default();
                    events.push(StreamEvent::ToolCallStart {
                        index,
                        id: item.call_id.clone().unwrap_or_default(),
                        name: item.name.clone().unwrap_or_default(),
                    });
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(delta) = &event.delta {
                    self.tool_inputs.entry(index).or_default().push_str(delta);
                    events.push(StreamEvent::ToolCallDelta {
                        index,
                        delta: delta.clone(),
                    });
                }
            }
            "response.function_call_arguments.done" => {
                let buffer = self
                    .tool_inputs
                    .remove(&index)
                    .or_else(|| event.arguments.clone())
                    .unwrap_or_default();
                match parse_arguments(&buffer) {
                    Ok(arguments) => events.push(StreamEvent::ToolCallEnd {
                        index,
                        arguments,
                        state: None,
                    }),
                    Err(error) => {
                        return EventOutcome {
                            events,
                            error: Some(error),
                            done: true,
                        };
                    }
                }
            }
            "response.output_text.delta" => {
                if let Some(delta) = &event.delta {
                    if self.text_started.insert(index) {
                        events.push(StreamEvent::TextStart { index });
                    }
                    events.push(StreamEvent::TextDelta {
                        index,
                        delta: delta.clone(),
                    });
                }
            }
            "response.output_text.done" if self.text_started.remove(&index) => {
                events.push(StreamEvent::TextEnd { index });
            }
            "response.reasoning_summary_text.delta" => {
                if let Some(delta) = &event.delta {
                    if self.thinking_started.insert(index) {
                        events.push(StreamEvent::ThinkingStart { index });
                    }
                    events.push(StreamEvent::ThinkingDelta {
                        index,
                        delta: delta.clone(),
                    });
                }
            }
            "response.completed" => {
                let mut stop_reason = StopReason::EndTurn;
                if let Some(response) = &event.response {
                    if let Some(usage) = &response.usage {
                        let details = usage.input_tokens_details.clone().unwrap_or_default();
                        events.push(StreamEvent::Usage(Usage {
                            input_tokens: usage.input_tokens,
                            output_tokens: usage.output_tokens,
                            cache_read_tokens: details.cached_tokens,
                            cache_write_tokens: details.cache_write_tokens,
                            // Azure does not report a charge, so the field stays empty
                            // rather than guessing from a price table.
                            cost_usd: None,
                        }));
                    }
                    if response
                        .output
                        .iter()
                        .any(|item| item.kind == "function_call")
                    {
                        stop_reason = StopReason::ToolUse;
                    }
                }
                events.push(StreamEvent::Done { stop_reason });
                return EventOutcome {
                    events,
                    error: None,
                    done: true,
                };
            }
            "response.failed" | "error" => {
                let (code, message) = event
                    .error
                    .map(|error| (error.code, error.message.unwrap_or_default()))
                    .unwrap_or((None, String::new()));
                let status = code.unwrap_or(500);
                return EventOutcome {
                    events,
                    error: Some(status_to_error(status, None, message)),
                    done: true,
                };
            }
            // Other event kinds carry no normalised event.
            _ => {}
        }

        EventOutcome {
            events,
            error: None,
            done: false,
        }
    }
}

/// Parse the assembled tool-call arguments. An empty buffer is an empty object.
fn parse_arguments(buffer: &str) -> Result<Value, ProviderError> {
    if buffer.trim().is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    serde_json::from_str(buffer).map_err(|error| {
        ProviderError::Decode(format!(
            "the tool-call arguments did not parse as JSON: {error}"
        ))
    })
}

// --- The request body. ---------------------------------------------------

/// Build the Responses request body. See `SPEC-provider-interface` section 6.
/// Report once that a set effort level does not reach this provider.
///
/// Once per level, not once per turn. A warning a user learns to scroll past stops working.
fn report_effort_gap(effort: rho_core::ReasoningEffort) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};

    static REPORTED: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let reported = REPORTED.get_or_init(|| Mutex::new(HashSet::new()));
    let first = match reported.lock() {
        Ok(mut set) => set.insert(effort.as_str()),
        Err(_) => true,
    };
    if first {
        tracing::warn!(
            effort = effort.as_str(),
            "this provider does not send a reasoning effort yet, so the level had no effect"
        );
    }
}

pub fn build_request_body(request: &CompletionRequest, deployment: &str) -> Value {
    let mut input = Vec::new();
    if let Some(system) = &request.system {
        input.push(json!({ "role": "system", "content": system }));
    }
    for message in &request.messages {
        input.extend(message_to_items(message));
    }

    let mut body = json!({
        "model": deployment,
        "input": input,
        "stream": true,
    });
    let map = body.as_object_mut().expect("the body is an object");
    // The effort level does not travel here yet, and a review was right that silence is the
    // defect rather than the absence. rho has no Azure account to drive, and the sprint-1
    // lesson is exact about this: every fixture described a response, and every defect was in
    // the request. Guessing a field name earns a 400 for the whole turn, so rho reports the
    // gap once per level instead of inventing a field. See
    // `SPEC-reasoning-across-providers` section 9.
    if let Some(effort) = request.reasoning {
        report_effort_gap(effort);
    }
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                })
            })
            .collect();
        map.insert("tools".to_string(), Value::Array(tools));
    }
    if let Some(max_tokens) = request.max_tokens {
        map.insert("max_output_tokens".to_string(), json!(max_tokens));
    }
    if let Some(temperature) = request.temperature {
        map.insert("temperature".to_string(), json!(temperature));
    }
    body
}

/// Map one normalised message to **one or more** Responses input items.
///
/// One message can produce several items, so this returns a list. An assistant turn
/// with text and two tool calls becomes three items.
///
/// A live run against Azure forced this shape. The Responses API answered 400 with
/// `Invalid value: 'tool'. Supported values are: 'assistant', 'system', 'developer',
/// and 'user'.` Responses has no tool role. It mixes messages and typed items in one
/// `input` array:
///
/// - a message is `{"role": ..., "content": ...}`;
/// - a tool call is `{"type": "function_call", "call_id": ..., "name": ..., "arguments": "<json string>"}`;
/// - a tool result is `{"type": "function_call_output", "call_id": ..., "output": ...}`.
///
/// Two rules are easy to miss, and both cost a 400.
///
/// First, `arguments` is a JSON **string**, not an object.
///
/// Second, a `function_call` item must appear before its own `function_call_output`,
/// and the `call_id` values must match. The provider used to drop assistant tool
/// calls entirely, so an output referenced a call the service had never seen.
fn message_to_items(message: &Message) -> Vec<Value> {
    let mut items = Vec::new();
    let mut text = String::new();

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
                // Flush any prose that came before the call, so order survives.
                if !text.is_empty() {
                    items.push(json!({ "role": role_name(message.role), "content": text }));
                    text = String::new();
                }
                items.push(json!({
                    "type": "function_call",
                    "call_id": id,
                    "name": name,
                    "arguments": arguments.to_string(),
                }));
            }
            ContentBlock::ToolResult {
                tool_call_id,
                content,
                ..
            } => {
                let mut output = String::new();
                for inner in content {
                    if let ContentBlock::Text { text: piece } = inner {
                        output.push_str(piece);
                    }
                }
                items.push(json!({
                    "type": "function_call_output",
                    "call_id": tool_call_id,
                    "output": output,
                }));
            }
            // A trace is for the reader, so it never travels.
            ContentBlock::ReasoningTrace { .. } => {}
            // Azure returns a reasoning summary and no replay token on this path, so rho
            // stores no payload and has nothing to send back. The named arm replaces a
            // `_ => {}` that a review found: a wildcard in a request builder hides the next
            // block kind, and that is how a reasoning block was dropped in silence before.
            ContentBlock::ReasoningReplay { .. } => {}
            // Image input in a request is out of scope for sprint 1.
            ContentBlock::Image { .. } => {}
        }
    }

    // Emit trailing prose. Skip an empty message, because an empty content string
    // adds nothing and some models reject it.
    if !text.is_empty() {
        items.push(json!({ "role": role_name(message.role), "content": text }));
    }
    items
}

/// The Responses role name for a normalised role.
///
/// `Role::Tool` never reaches this function through a normal path, because a tool
/// result becomes a `function_call_output` item. It maps to `user` as a safe
/// fallback, since Responses would reject `tool`.
fn role_name(role: Role) -> &'static str {
    match role {
        Role::User | Role::Tool => "user",
        Role::Assistant => "assistant",
    }
}
