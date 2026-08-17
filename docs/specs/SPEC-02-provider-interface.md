# SPEC-02 — Provider interface

Status: draft for sprint 1.
Owning crates: `rho-core` (the trait), `rho-provider-openrouter`,
`rho-provider-bedrock`, `rho-provider-azure`.

A provider turns a `CompletionRequest` into a stream of `StreamEvent`. Three wire
formats normalise onto the one event model from `SPEC-01`. This spec defines the
trait, the request type, the three mappings, the retry policy, and the secret
redaction rule.

Features covered: F-10 (provider trait), F-11 (OpenRouter), F-12 (Bedrock),
F-13 (Azure), F-05 (auto-retry), F-103 (no secrets in logs), F-113 (per-provider
feature flags).

## 1. The trait

```rust
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
```

The stream must yield the first event without buffering the whole response body.
Tool calls must arrive as structured `ToolCallStart`, `ToolCallDelta`, and
`ToolCallEnd` events, never as raw text.

## 2. Secret redaction by construction

A secret must never reach a log, including at `trace` level. The design forbids
it by construction, not by a filter at the end.

Rule:
- Wrap every credential in `Secret`. `Secret` has no `Debug` or `Display` that
  reveals the value. It exposes the bytes only through `expose`.
- Never place a `Secret` in a struct that derives `Debug`. Never log a request
  header map. Log a fixed allow-list of header names instead.
- `CompletionRequest` derives `Debug`. It holds no credential. The credential is
  held by the provider, resolved at construction.

```rust
/// A credential value that never prints itself.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    /// The only way to read the value. Callers must not log the result.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}
```

## 3. Retry policy

The retry policy wraps a provider. It reads `ProviderError::is_retryable`.

Rules:
- Retry on `Transport`, `Server` (5xx), and `RateLimited` (429).
- Never retry `Client` (4xx other than 429), `Decode`, or `Auth`.
- Use exponential backoff with full jitter. Base 500 ms, factor 2, cap 30 s.
- On `RateLimited { retry_after_ms: Some(ms) }`, wait `ms` and skip the jitter.
- Retry only before the first `StreamEvent`. Once a token is emitted, an error
  ends the stream. A mid-stream error is not retried.
- The default maximum is 5 attempts.

```rust
#[derive(Clone, Copy, Debug)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_attempts: 5, base_delay_ms: 500, max_delay_ms: 30_000 }
    }
}

impl RetryPolicy {
    /// The delay before attempt `attempt` (1-based), given an optional server hint.
    /// Returns `None` when the caller must not retry.
    pub fn backoff(&self, attempt: u32, retry_after_ms: Option<u64>) -> Option<std::time::Duration>;
}
```

## 4. OpenRouter mapping

Crate: `rho-provider-openrouter`. Endpoint: `POST /api/v1/chat/completions` on
`https://openrouter.ai`. Transport: `reqwest` with `rustls`, HTTP/2, streaming.
Auth: `Authorization: Bearer <key>`. The stream is SSE, parsed by
`eventsource-stream`.

Request shape:
- `model`, `messages`, `stream: true`.
- `tools`: an array of `{ "type": "function", "function": { "name",
  "description", "parameters" } }`. `parameters` is the tool `input_schema`.
- Pass `reasoning` through unchanged when the model returns it.

SSE parsing:
- Each SSE line is `data: <json>`.
- Skip a line that starts with `:`. It is an OpenRouter keep-alive comment such
  as `: OPENROUTER PROCESSING`. It is not JSON.
- The terminal line is `data: [DONE]`.

Chunk shape: `choices[0].delta`. The delta carries text, tool-call fragments, or
reasoning.

Event mapping:
- First chunk: emit `MessageStart { role: Assistant }`.
- `delta.content` string: emit `TextStart` on first sight of index 0, then
  `TextDelta`, then `TextEnd` at stream end.
- `delta.reasoning` string: map to `ThinkingStart` / `ThinkingDelta` /
  `ThinkingEnd`.
- `delta.tool_calls[]`: each element has an `index`, and on the first fragment an
  `id` and `function.name`. Later fragments carry `function.arguments` string
  pieces. Assemble by `index`. Emit `ToolCallStart { index, id, name }` once,
  then `ToolCallDelta { index, delta }` per fragment, then `ToolCallEnd` with the
  parsed object when the tool-call block closes.
- `usage` on the final chunk: emit `Usage`.
- `choices[0].finish_reason`: map `stop` to `EndTurn`, `length` to `MaxTokens`,
  `tool_calls` to `ToolUse`, `content_filter` to `ContentFiltered`. Emit `Done`.

Error mapping:
- A non-200 response before any token: read the JSON `error` object. Map by HTTP
  status: 429 to `RateLimited`, 5xx to `Server`, other 4xx to `Client`.
- A mid-stream error arrives as a normal `data:` chunk with a top-level `error`
  field and `choices[0].finish_reason: "error"`. Yield `Err(ProviderError::Server)`
  and stop. Do not retry.

## 5. AWS Bedrock mapping

Crate: `rho-provider-bedrock`. Uses `aws-sdk-bedrockruntime` and `aws-config`.
Operations: `Converse` for the non-streaming path, `ConverseStream` for the
streaming path. SigV4 signing comes from the standard credential chain through
`aws-config`: environment, shared profile, SSO cache, and IMDS. The provider does
not sign requests by hand. The SDK signs them.

The SDK gives typed event enums. The provider matches on `ConverseStreamOutput`
rather than parsing JSON by hand.

`ConverseStream` event kinds and their mapping:
- `messageStart { role }`: emit `MessageStart`.
- `contentBlockStart { contentBlockIndex, start }`: when `start` is a `toolUse`
  with a `toolUseId` and `name`, emit `ToolCallStart { index, id, name }`.
- `contentBlockDelta { contentBlockIndex, delta }`:
  - `delta.text`: emit `TextStart` on first delta for the index, then `TextDelta`.
  - `delta.toolUse.input`: a JSON fragment. Emit `ToolCallDelta`.
  - `delta.reasoningContent.text`: emit `ThinkingDelta`.
- `contentBlockStop { contentBlockIndex }`: close the block. For a text block emit
  `TextEnd`. For a tool block parse the buffered input and emit `ToolCallEnd`. For
  a reasoning block emit `ThinkingEnd`.
- `messageStop { stopReason }`: map `end_turn` to `EndTurn`, `tool_use` to
  `ToolUse`, `max_tokens` to `MaxTokens`, `stop_sequence` to `StopSequence`,
  `content_filtered` and `guardrail_intervened` to `ContentFiltered`. Emit `Done`.
- `metadata { usage }`: read `inputTokens`, `outputTokens`,
  `cacheReadInputTokens`, `cacheWriteInputTokens`. Emit `Usage`.

Error mapping:
- `throttlingException`: `RateLimited`.
- `serviceUnavailableException` and `internalServerException`: `Server`.
- `modelStreamErrorException`: `Server`.
- `validationException`: `Client { status: 400 }`.
- A credential or signing failure from `aws-config`: `Auth`.

## 6. Azure OpenAI mapping

Crate: `rho-provider-azure`. Endpoint: the Azure OpenAI `/responses` API on the
resource base URL, for example
`https://<resource>.openai.azure.com/openai/v1/responses`. Transport: `reqwest`
with `rustls`, streaming. The stream is SSE parsed by `eventsource-stream`.

Two auth modes:
- API key: header `api-key: <key>`.
- Microsoft Entra token: header `Authorization: Bearer <token>`. The token
  audience must be exactly:

  ```text
  https://cognitiveservices.azure.com/
  ```

This exact audience string, with the trailing slash, is a known real-world bug
source. The provider pins it as a constant. A test asserts the constant.

```rust
/// The required Microsoft Entra token audience for Azure OpenAI.
/// The trailing slash is required. Do not change this string.
pub const AZURE_ENTRA_AUDIENCE: &str = "https://cognitiveservices.azure.com/";
```

Responses API request shape:
- `model` is the deployment name.
- `input` is the conversation as Responses items.
- `tools` is an array of function tool definitions.
- `stream: true`.

Responses API stream events and their mapping (the `type` field names the event):
- `response.created`: emit `MessageStart { role: Assistant }`.
- `response.output_item.added` with a `function_call` item: emit
  `ToolCallStart { index, id, name }`, where `id` is the item `call_id` and
  `name` is the function name.
- `response.function_call_arguments.delta`: emit `ToolCallDelta { index, delta }`.
- `response.function_call_arguments.done`: parse the arguments and emit
  `ToolCallEnd`.
- `response.output_text.delta`: emit `TextStart` on the first delta for the item,
  then `TextDelta`.
- `response.output_text.done`: emit `TextEnd`.
- `response.reasoning_summary_text.delta`: emit `ThinkingDelta`. A reasoning item
  maps to a `Thinking` block.
- `response.completed`: read `response.usage` (`input_tokens`, `output_tokens`),
  emit `Usage`, then emit `Done { stop_reason: EndTurn }`, or `ToolUse` when the
  output holds a function call.
- `response.failed` and a top-level `error` event: map by status to `Server`,
  `RateLimited`, or `Client`.

## 7. Test cases

`rho-core` shared contract (in `crates/rho-core/tests/provider_contract.rs`):
- `provider_contract_emits_message_start_first` — every provider under a fake
  transport emits `MessageStart` as its first event.
- `provider_contract_emits_done_last` — every provider ends a good turn with
  `Done`.
- `provider_contract_yields_first_event_before_stream_end` — the stream yields its
  first event while the fake transport still holds later bytes back. This proves
  the provider streams and does not buffer the whole response. The fake transport
  sends one chunk, then blocks until the test observes an event, then sends the
  rest. A provider that buffers deadlocks the test and fails it.
- `provider_contract_text_deltas_in_order` — text deltas arrive in index order.
- `provider_contract_tool_call_end_has_parsed_arguments` — `ToolCallEnd` carries
  a parsed JSON object, not a string.

OpenRouter (in `crates/rho-provider-openrouter/tests/`):
- `provider_openrouter_streams_text_deltas` — a recorded SSE stream yields ordered
  `TextDelta` events.
- `provider_openrouter_assembles_split_tool_call_fragments` — tool-call fragments
  split across chunks assemble into one `ToolCallEnd` with the full arguments.
- `provider_openrouter_skips_processing_comment_lines` — a `: OPENROUTER
  PROCESSING` line is ignored and does not break the stream.
- `provider_openrouter_reports_usage` — the final chunk `usage` becomes a `Usage`
  event.
- `provider_openrouter_midstream_error_yields_err` — a chunk with a top-level
  `error` and `finish_reason: "error"` ends the stream with `Err`.
- `provider_openrouter_maps_finish_reason_tool_calls_to_tool_use` — a
  `tool_calls` finish reason maps to `StopReason::ToolUse`.

Azure (in `crates/rho-provider-azure/tests/`):
- `provider_azure_entra_audience_is_pinned` — `AZURE_ENTRA_AUDIENCE` equals
  `https://cognitiveservices.azure.com/`, trailing slash included.
- `provider_azure_streams_output_text_delta` — `response.output_text.delta`
  events become `TextDelta`.
- `provider_azure_assembles_function_call_items` — a `function_call` item plus its
  `function_call_arguments` deltas assemble into one `ToolCallEnd`.
- `provider_azure_maps_reasoning_summary_to_thinking` — a
  `response.reasoning_summary_text.delta` becomes a `ThinkingDelta`.
- `provider_azure_api_key_sets_api_key_header` — API-key mode sets the `api-key`
  header and no `Authorization` header.
- `provider_azure_entra_sets_bearer_header` — Entra mode sets `Authorization:
  Bearer` and no `api-key` header.

Bedrock (in `crates/rho-provider-bedrock/tests/`):
- `provider_bedrock_maps_content_block_delta_text` — a `contentBlockDelta` text
  delta becomes a `TextDelta`.
- `provider_bedrock_assembles_tool_use_input` — a `toolUse` start plus input
  fragments assemble into one `ToolCallEnd`.
- `provider_bedrock_maps_stop_reason_tool_use` — a `messageStop` with
  `stopReason: "tool_use"` maps to `StopReason::ToolUse`.
- `provider_bedrock_maps_throttling_to_rate_limited` — a `throttlingException`
  maps to `ProviderError::RateLimited`.

Retry and secrecy (in `rho-core`):
- `retry_policy_backs_off_on_server_error` — a `Server` error triggers a retry
  within `max_attempts`.
- `retry_policy_never_retries_client_error` — a `Client` error is not retried.
- `retry_policy_honours_retry_after` — `RateLimited { retry_after_ms }` waits the
  hint and skips the jitter.
- `secret_debug_is_redacted` — `format!("{:?}", secret)` yields `Secret(***)` and
  never the value.

All provider tests use `wiremock` or a recorded fixture. No test reaches the
network. Bedrock tests drive the SDK against a stubbed transport or a recorded
event fixture.

## 8. Out of scope for sprint 1

- The non-streaming `Converse` path. Sprint 1 streams only.
- Prompt caching cache-point markers for Bedrock and Anthropic.
- Image input to Bedrock and Azure. Sprint 1 sends text and tool calls.
- Provider auto-discovery from a registry file. The provider is constructed in
  code.
- OAuth login flows. Sprint 1 uses env, profile, and static keys only.
- OpenRouter stream cancellation billing behaviour. rho cancels by dropping the
  request; billing is the provider's concern.
