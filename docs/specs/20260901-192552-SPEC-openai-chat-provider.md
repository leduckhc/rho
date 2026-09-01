# SPEC-openai-chat-provider — OpenAI Chat Completions API

Status: draft
Prior art: `rho-provider-openrouter` source, `SPEC-reasoning-across-providers`,
`D-a-provider-names-its-own-credential`.

## 0. The question

Can rho reach xdent's OpenAI-family routes, plus real `api.openai.com`?

rho already has `rho-provider-openrouter`. It speaks OpenAI Chat. OpenRouter is
OpenAI-compatible. So rho has the wire client. The question is crate structure.

## 1. The sides

| Side | Owner | Must agree on |
| --- | --- | --- |
| The wire | the provider | request shape, SSE parsing, error taxonomy |
| The provider trait | `rho-core` | `id() -> &str`, `stream(...)` signatures |
| Config resolution | `rho-config` | credential fallback variable |

Contract kinds touched: public API, wire format, error taxonomy, configuration.

## 2. The crate structure

**Decision:** `rho-provider-openrouter` gains a new `Provider` impl. See
`D-one-openai-client-two-providers`.

```rust
pub struct OpenAi {
    config: OpenAiConfig,
    client: reqwest::Client,
}

impl Provider for OpenAi {
    fn id(&self) -> &str { "openai" }
    // stream() delegates to existing wire logic
}
```

The wire logic is already there. What changes is `Provider::id`, `base_url` default, and
credential variable name.

## 3. The public API

```rust
pub struct OpenAiConfig {
    pub base_url: String,  // default: "https://api.openai.com/v1"
    pub api_key: rho_core::Secret,
    pub retry: rho_core::RetryPolicy,
}

impl OpenAiConfig {
    pub fn new(api_key: rho_core::Secret) -> Self;
    pub fn with_base_url(self, base_url: impl Into<String>) -> Self;
    pub fn with_retry(self, retry: rho_core::RetryPolicy) -> Self;
}

pub struct OpenAi {
    config: OpenAiConfig,
    client: reqwest::Client,
}

impl OpenAi {
    pub fn new(config: OpenAiConfig) -> Self;
    pub fn base_url(&self) -> &str;
}

// `Provider` impl: id() -> "openai", stream(...) -> ProviderStream
```

The request field is `max_completion_tokens`. OpenRouter uses `max_tokens`.

## 4. Wire mapping

### Request

| rho | OpenAI Chat |
| --- | --- |
| `system` | `messages[0] = {role: "system", content}` |
| `Message` | `{role, content, tool_calls?, tool_call_id?}` |
| `ToolSpec` | `{type: "function", function: {name, description, parameters}}` |
| `max_tokens` | `max_completion_tokens` |
| `reasoning` | per `SPEC-reasoning-across-providers` section 9 |

### Response

SSE stream. Terminated by `data: [DONE]`.

| OpenAI Chat | rho |
| --- | --- |
| `choices[0].delta.content` | `TextDelta { index: 0, delta }` |
| `choices[0].delta.tool_calls` | `ToolCallStart`, `ToolCallDelta`, `ToolCallEnd` |
| `choices[0].finish_reason` | Hold until stream ends. Emit `Done` at `[DONE]` |
| `usage.completion_tokens_details.reasoning_tokens` | `Usage.reasoning_tokens` |

**Reasoning text does not stream.** OpenAI returns counts only. No `ThinkingDelta` events.
One `Usage` at the end.

## 5. Error taxonomy

| Status | rho |
| --- | --- |
| 401, 403 | `Auth` |
| 429 | `RateLimited { retry_after_ms }` |
| 500-599 | `Server { status }` |
| other 4xx | `Client { status, advice }` |
| network | `Transport` |
| JSON parse | `Decode` |

The body is not read. See `D-a-client-error-carries-no-peer-body`.

## 6. Reasoning

| `ReasoningEffort` | Wire |
| --- | --- |
| `Off` | no field |
| `Low` | `reasoning_effort: "low"` |
| `Medium` | `reasoning_effort: "medium"` |
| `High`, `XHigh` | `reasoning_effort: "high"` |

**No text stream.** OpenAI returns counts only. One `Usage` event at the end.

## 7. Cost

OpenAI does not report cost. `Usage.cost_usd` stays `None`.

## 8. Forbidden shapes

These return `Decode`:

- Empty `choices` array.
- `finish_reason` of `null`.
- Two `tool_calls` with the same `id`.
- Unparseable `arguments`.

## 9. Contract test

Calls `rho_provider_testkit::contract::run_all`.

## 10. Compatibility

Works with: xdent routes, `api.openai.com`, Ollama, OpenAI-compatible endpoints.

**Not promised:** non-standard quirks, missing fields, deviations.

## 11. Out of scope

Audio, image, structured outputs, Azure Responses API.

## Test cases

- `the_request_names_max_completion_tokens`
- `a_non_200_status_maps_to_provider_error`
- `a_tool_call_assembles_across_deltas`
- `reasoning_tokens_reach_usage`
- `no_reasoning_text_is_streamed`
- `the_finish_chunk_is_held_until_done`
- `empty_choices_is_a_decode_error`
- `unparseable_tool_arguments_is_a_decode_error`
- `the_contract_test_passes`
- `the_provider_id_is_openai`
- `the_default_base_url_is_production_openai`
- `with_base_url_overrides_the_default`
- `a_bad_api_key_is_refused_as_auth_error`
- `a_rate_limit_reads_retry_after`

## F- rows

```
| F-openai-provider | OpenAI provider | rho reaches `api.openai.com` and xdent OpenAI routes with a new `Provider` impl in `rho-provider-openrouter`. Wire: OpenAI Chat Completions API. Reasoning effort sends `reasoning_effort: low/medium/high`. No reasoning text streams; only token counts. Cost is `None`. Config: `OpenAiConfig { base_url, api_key, retry }`. | `rho-provider-openrouter`, `rho-core` | `draft` | Wire client exists. New `impl` changes `Provider::id`, `base_url` default, credential variable. |
```

## Amendments after the contract review, binding

A contract review found three blockers and one major here, and named this spec the one most
likely to be regretted. Where an amendment disagrees with the text above, the amendment
wins.

### 1. The crate is renamed to `rho-provider-openai-chat`

`D-one-openai-client-two-providers` rested on a false premise: that the wire code was 600
shared lines with a 3-line difference. `build_request_body` at
`crates/rho-provider-openrouter/src/lib.rs:684` hardcodes `"max_tokens"`, which is the one
wire difference this spec names. And `stream`/`send_with_retry`/`send_once` are
`impl OpenRouterProvider` methods that read `self.config: OpenRouterConfig`, so a second
impl cannot share them without a refactor. The crate name `rho-provider-openrouter` also
lies the moment it hosts an `OpenAi` provider, which is the shape the parent decision
`D-a-provider-is-named-by-its-wire-protocol` was written to prevent.

So the crate is **renamed** `rho-provider-openai-chat`, and `OpenRouterProvider` becomes
one impl inside it. See `D-provider-openai-chat-owns-the-wire`. `OpenRouterProvider`'s id
stays `"openrouter"`, so no user-visible change follows the rename.

### 2. The wire refactor precedes the second impl

Order matters. The `build_request_body`, `stream`, `send_with_retry` and `send_once` sites
must move to free functions or a shared inner type first. Only then does `OpenAiProvider`
land. Any other order is copy-paste, and the "shared" story is a lie the reader can measure.

Tests: `both_providers_share_the_request_builder`,
`the_openai_provider_sends_max_completion_tokens`,
`the_openrouter_provider_still_sends_max_tokens`.

### 3. Reasoning tokens do not reach `Usage`

The reviewer noted `Usage.reasoning_tokens` does not exist. `SPEC-usage-carries-reasoning`
is draft and its `F-reasoning-token-count` row sits at `considered`, not `planned`, because
the user asked for reasoning **text**, not a count, and the text already ships.

So this spec does not populate `Usage.reasoning_tokens`, and it does not name a test that
reads that field. If a later feature builds the counter, it can add the mapping at
`usage.completion_tokens_details.reasoning_tokens`. The wire path is measured, in
`docs/verification/provider-reasoning-probe.md`, so the follow-up does not re-probe.

### 4. Cost stays `None`, even where the wire carries a cost

The xdent proxy returns `usage.cost_details.upstream_inference_cost`, and OpenRouter
returns `usage.cost`. `OpenAiProvider` sees the OpenAI path only, which reports no cost,
so `Usage.cost_usd` is `None`. `OpenRouterProvider` keeps its existing cost read, because
that is a measured number and this spec must not lose it.

### 5. Persisted shape is not widened

`TranscriptBody::Usage` carries `input` and `output`. So reasoning tokens and cost details
are display-only, exactly as they are on the Anthropic side. A future feature widens the
persisted record with a migration.
