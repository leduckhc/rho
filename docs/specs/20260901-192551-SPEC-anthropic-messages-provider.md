# SPEC-anthropic-messages-provider — Anthropic Messages API provider

Status: draft, for review before any implementation.
Prior art: `SPEC-reasoning-across-providers`, `D-measured-cost-and-cache`,
`D-a-provider-names-its-own-credential`, `D-an-empty-signature-is-no-signature`.

## 0. The problem

The user needs a provider that speaks Anthropic Messages API over HTTPS. It replaces
Bedrock for Claude models. It also reaches real `api.anthropic.com`.

## 1. The sides

| Side | Owner | Must agree on |
| --- | --- | --- |
| The wire, read | `rho-provider-anthropic` | Anthropic SSE event shapes |
| The wire, write | `rho-provider-anthropic` | request shape and headers |
| `Provider` trait | `rho-core` | the two methods and `ProviderError` taxonomy |
| Error handling | `rho-core` | which HTTP status retries |
| Event mapping | `rho-provider-anthropic` | which SSE event becomes which `StreamEvent` |
| Credentials | `rho-config` | the fallback environment variable |

Contract kinds touched: public API, wire format, error taxonomy, configuration.

## 2. The crate structure

```rust
// crates/rho-provider-anthropic/Cargo.toml
[package]
name = "rho-provider-anthropic"

[dependencies]
async-stream = "0.3"
async-trait = "0.1.92"
eventsource-stream = "0.2.3"
futures = "0.3.34"
reqwest = { version = "0.13.4", default-features = false, features = ["rustls", "json", "stream", "http2", "charset", "system-proxy"] }
rho-core = { path = "../rho-core" }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "2.0"
tokio = { version = "1.53", features = ["rt", "macros", "time", "sync"] }
tracing = "0.1"

[dev-dependencies]
rho-provider-testkit = { path = "../rho-provider-testkit" }
tokio = { version = "1.53", features = ["full"] }
wiremock = "0.6"
```

The features on `reqwest` match `rho-provider-openrouter`. This repo enforces one TLS
stack. See `D-one-tls-stack`.

## 3. The public config type

```rust
/// Configuration for the Anthropic Messages API provider.
#[derive(Clone, Debug)]
pub struct AnthropicConfig {
    /// The base URL. Default: `https://api.anthropic.com`.
    pub base_url: String,
    /// The credential. Resolves to an `x-api-key` header.
    pub credential: rho_core::Credential,
    /// Extra headers, such as `anthropic-beta`. Empty by default.
    pub headers: Vec<(String, String)>,
    /// Request timeout. Default: 90 seconds.
    pub timeout: std::time::Duration,
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.anthropic.com".to_string(),
            credential: rho_core::Credential::Env("ANTHROPIC_API_KEY".to_string()),
            headers: Vec::new(),
            timeout: std::time::Duration::from_secs(90),
        }
    }
}
```

The credential resolves through `Config::resolve_credential_or_env` at build time. The
builder passes `"anthropic"` as the name and `"ANTHROPIC_API_KEY"` as the fallback. See
`D-a-provider-names-its-own-credential`.

## 4. Wire mapping: SSE events to `StreamEvent`

| Anthropic SSE event | `StreamEvent` | Notes |
| --- | --- | --- |
| `message_start` | `MessageStart { role: Role::Assistant }` | Carries seed `usage`. Emit a `Usage` event. |
| `content_block_start` (type `text`) | `TextStart { index }` | Map `index` verbatim. |
| `content_block_start` (type `tool_use`) | `ToolCallStart { index, id, name }` | Read `id` and `name` from the block. |
| `content_block_start` (type `thinking`) | `ThinkingStart { index }` | Only when `thinking.type` was `enabled`. |
| `content_block_delta` (type `text_delta`) | `TextDelta { index, delta }` | Read `delta` from the event. |
| `content_block_delta` (type `input_json_delta`) | `ToolCallDelta { index, delta }` | Read `partial_json` as the delta. |
| `content_block_delta` (type `thinking_delta`) | `ThinkingDelta { index, delta }` | Read `thinking` as the delta. |
| `content_block_stop` (text or tool) | `TextEnd { index }` or `ToolCallEnd { index, arguments, state: None }` | For tools, parse the assembled JSON. |
| `content_block_stop` (thinking) | `ThinkingEnd { index, state }` | Extract `signature` from the assembled block. See section 6. |
| `message_delta` | `Usage(usage)` | Carries final `usage`. Add it to the seed. |
| `message_stop` | `Done { stop_reason }` | Map `stop_reason`. See section 5. |
| `error` | Yield `Err(ProviderError::Decode(_))` | Read `error.message`. Stop the stream. |
| unknown event type | Log at `debug` level. Ignore. | A new event must not break an old client. |

An `input_json_delta` arrives as fragments. Assemble them into one string, then parse with
`serde_json::from_str` before emitting `ToolCallEnd`. A parse failure is a decode error.

## 5. Stop reason mapping

| Anthropic `stop_reason` | rho `StopReason` |
| --- | --- |
| `end_turn` | `EndTurn` |
| `tool_use` | `ToolUse` |
| `max_tokens` | `MaxTokens` |
| `stop_sequence` | `StopSequence` |
| (absent) | Yield `Err(ProviderError::Decode("no stop_reason"))`. |
| (other) | Yield `Err(ProviderError::Decode("unknown stop_reason: ..."))`. |

A new stop reason must not silently become `EndTurn`. It fails the stream.

## 6. Error taxonomy: HTTP status to `ProviderError`

| HTTP status / JSON error type | `ProviderError` | Retryable | Advice |
| --- | --- | --- | --- |
| 401 | `Auth("invalid API key")` | No | User message: "Set `ANTHROPIC_API_KEY` or add `[credentials.anthropic]` to your config." |
| 403 | `Auth("permission denied")` | No | Same as 401. |
| 429 | `RateLimited { retry_after_ms }` | Yes | Read `Retry-After` header in seconds. Multiply by 1000. |
| 500, 502, 503, 504, 529 | `Server { status }` | Yes | None. |
| 400, 404, other 4xx | `Client { status, advice: "invalid request" }` | No | **Never include the response body.** See note. |
| Network error, DNS, timeout | `Transport(message)` | Yes | Wrap the `reqwest::Error`. |
| JSON decode error | `Decode(message)` | No | `"failed to parse event: ..."`. |
| Canceled | `Canceled` | No | Stream stops. |

**Auth failure user message:** when `resolve_credential_or_env` returns `Auth(_)`, the CLI
prints: "Authentication failed. Set `ANTHROPIC_API_KEY` or add `[credentials.anthropic]` to
your config."

**Note:** `Client::advice` is `&'static str`, so a response body can never reach it. A
reflected `Authorization` header would put a credential on stderr. The compiler enforces
this. See `D-a-client-error-carries-no-peer-body`.

## 7. Reasoning replay: empty signature handling

When a stored `thinking` block carries a `signature`, the provider must replay it. Rule:

```rust
let signature = payload
    .get("signature")
    .and_then(|v| v.as_str())
    .filter(|s| !s.trim().is_empty());

match signature {
    Some(sig) => {
        // Build a thinking content block with this signature and send it.
    }
    None => {
        // Drop this block. Report the drop. See D-a-drop-report-is-data-not-a-log-line.
    }
}
```

An absent key, an empty string, and a string of blanks are all `None`. All three drop the
block and report `ReplayDropReason::NoSignature`. See
`D-an-empty-signature-is-no-signature`.

When the block has a signature, assemble the full thinking content block and send it in the
next request. Anthropic rejects an empty signature with a 400 error.

## 8. Cost and reasoning tokens

Anthropic does not report a per-call cost. `Usage.cost_usd` stays `None`. See
`D-measured-cost-and-cache`.

Anthropic does not report a reasoning token count. `Usage.reasoning_tokens` stays `None`.
Live probe confirmed: the xdent proxy returned no reasoning token field.

## 9. Cache control

Out of scope for this spec. Anthropic supports `cache_control: {type:"ephemeral"}` on
content blocks. A later feature may expose it. This crate does not send it.

The response still carries cache counts. Map them:

```rust
usage.cache_read_tokens = anthropic_usage
    .cache_creation
    .as_ref()
    .and_then(|c| Some(c.ephemeral_5m_input_tokens + c.ephemeral_1h_input_tokens))
    .unwrap_or(0);

usage.cache_write_tokens = anthropic_usage.cache_creation_input_tokens;
```

The ephemeral fields may be absent. Default to zero.

## 10. Forbidden wire shapes

These are decode errors. Yield `Err(ProviderError::Decode(_))` and stop the stream.

- An assistant message with an empty `content` array.
- Two `tool_use` blocks with the same `id`.
- A `stop_reason` of `tool_use` when no tool block was emitted.
- A `tool_use` block followed by no matching `tool_result` in the next user message
  (checked at build time, not decode time).

The last one is a request-building rule, not a decode rule. The provider must not build an
invalid request.

## 11. The contract test

Every provider crate calls `rho_provider_testkit::contract::run_all` in a test. This crate
adds:

```rust
#[tokio::test]
async fn contract_all_checks_pass() {
    let harness = make_test_harness();
    rho_provider_testkit::contract::run_all(&harness).await;
}
```

The testkit has no caller today. See `SPEC-usage-carries-reasoning` section 2.

## Test cases

| Test | Assertion |
| --- | --- |
| `maps_message_start` | First SSE event becomes `MessageStart`. |
| `maps_text_block` | Text events become `TextStart`, `TextDelta`, `TextEnd`. |
| `maps_tool_call` | Tool events become `ToolCallStart`, `ToolCallDelta`, `ToolCallEnd` with parsed JSON. |
| `maps_thinking_block` | Thinking events become `ThinkingStart`, `ThinkingDelta`, `ThinkingEnd` with state. |
| `maps_stop_reason_end_turn` | `end_turn` becomes `StopReason::EndTurn`. |
| `maps_stop_reason_tool_use` | `tool_use` becomes `StopReason::ToolUse`. |
| `maps_stop_reason_max_tokens` | `max_tokens` becomes `StopReason::MaxTokens`. |
| `unknown_stop_reason_fails` | A new stop reason yields a decode error. |
| `maps_401_to_auth_error` | HTTP 401 becomes `ProviderError::Auth`. |
| `maps_429_to_rate_limited` | HTTP 429 becomes `RateLimited` with `retry_after_ms`. |
| `maps_500_to_server_error` | HTTP 500 becomes `Server { status: 500 }`. |
| `maps_400_to_client_error` | HTTP 400 becomes `Client` with static advice. |
| `empty_signature_is_dropped` | A stored thinking block with `signature: ""` sends nothing. |
| `blank_signature_is_dropped` | A signature of blanks sends nothing. Removing `trim` fails this alone. |
| `no_signature_is_dropped` | A block with no `signature` key sends nothing. |
| `valid_signature_is_sent` | A block with a signature builds a thinking content block. |
| `usage_carries_cache_counts` | Map `cache_creation` fields to `cache_read_tokens`. |
| `cost_usd_is_none` | `Usage.cost_usd` stays `None`. Anthropic does not report it. |
| `reasoning_tokens_is_none` | `Usage.reasoning_tokens` stays `None`. Anthropic does not report it. |
| `unknown_event_is_ignored` | An unknown SSE event logs at debug. Stream continues. |
| `network_error_is_transport` | A network fault yields `Transport(_)`. |
| `json_parse_error_is_decode` | Malformed JSON yields `Decode(_)`. |
| `contract_all_checks_pass` | Calls `rho_provider_testkit::contract::run_all`. |

## Out of scope

- Image input. Anthropic supports it. This spec does not.
- Prompt caching UI. Cache control is not exposed in the config or the request builder.
- Provider list at runtime. Providers are feature flags, so the list is static.
- Streaming tool calls mid-assembly. `ToolCallEnd` carries the full parsed object.

## Extension point

A third party adds a new credential source by implementing `rho_core::CredentialSource`.
No edit to `rho-provider-anthropic` is needed. The provider already reads
`rho_core::Credential`.

## Questions settled

- **Which TLS stack?** `rustls`, matching `rho-provider-openrouter`. One stack per repo.
- **Default base URL?** `https://api.anthropic.com`.
- **Fallback credential variable?** `ANTHROPIC_API_KEY`.
- **Does it report cost?** No. `Usage.cost_usd` stays `None`.
- **Does it report reasoning tokens?** No. Live probe found none.
- **How does it handle cache counts?** Sum the two ephemeral fields. Default absent to zero.
- **Empty signature?** Dropped, matching `D-an-empty-signature-is-no-signature`.

## Amendments after the contract review, binding

A contract review returned two blockers and three majors against this spec, plus one
independent finding on the cache mapping. Where an amendment and the text above disagree,
the amendment wins.

### 1. The credential type is `Secret`, not `Credential`

`rho_core::Credential` does not exist. The real types are `rho_core::Secret` (a resolved
value) and `rho_config::CredentialSource` (an unresolved reference). The provider takes an
already-resolved secret:

```rust
pub struct AnthropicConfig {
    pub base_url: String,
    pub credential: rho_core::Secret,
    pub headers: Vec<(String, String)>,
    pub timeout: std::time::Duration,
}
```

The named-entry lookup, in `SPEC-named-provider-profiles`, is the one place that resolves.
A built-in caller still uses `Config::resolve_credential_or_env("anthropic", "ANTHROPIC_API_KEY")`
against the merged credentials, and hands the resolved `Secret` to this config. That is one
resolution site, not two.

### 2. Cache counts map straight through, no sum

`D-anthropic-cache-fields-sum` shipped the wrong direction: it assigned Anthropic's **write**
breakdown to rho's **read** field. See `D-anthropic-cache-fields-map` for the correction.
The mapping is:

```rust
usage.cache_read_tokens  = anthropic_usage.cache_read_input_tokens;
usage.cache_write_tokens = anthropic_usage.cache_creation_input_tokens;
```

Both default to zero. The ephemeral breakdown is out of scope.

### 3. Cache and reasoning are display-only at launch

`TranscriptBody::Usage` at `crates/rho-core/src/transcript.rs:68` carries only `input` and
`output`. So cache counts, cost, and any reasoning-token count are display-only. A fork or a
resume writes the record without them. This launch does not widen the persisted shape; a
later feature does that with a migration.

The spec states this out loud, per the rule that a claim rho cannot prove is not made.
`docs/guide/status.md` may not say cache counts persist.

### 4. An unknown SSE event fails closed for content

Section 4 said "unknown event type → log at debug, ignore". A future content-bearing event
would vanish silently, which is the `ToolKind::Other` family. The rule is now split by
purpose:

- **Framing events** (`ping`, `error` at stream-scope, any future non-content event) may be
  ignored, and only for these the log-and-continue rule applies. Named as a closed set.
- **Any other unknown event** returns `ProviderError::Decode`, with the event name in the
  message. rho does not guess whether a new event carries content.

Test: `an_unknown_content_event_fails_decode`.

### 5. The tool_use input accumulator has a byte cap

Section 4 said "assemble them into one string, then parse". The `input_json_delta`
accumulator was unbounded. A large or hostile `tool_use.input` grew a string without limit,
which is the 8 MB → 805 MB family.

The accumulator is capped at 1 MiB, the same shape as `MAX_CONFIG_BYTES`:

```rust
pub const MAX_TOOL_INPUT_BYTES: usize = 1024 * 1024;
```

An overflow returns `ProviderError::Decode`, with the tool name in the message. The
`content` array itself gains a count cap of 256 blocks per turn, refused rather than
truncated.

Tests: `a_tool_use_input_over_the_cap_fails_decode`,
`a_content_array_over_the_cap_fails_decode`.

### 6. The extension-point claim was wrong

The earlier "extension point" said a third party implements `rho_core::CredentialSource`.
That type is a concrete enum in `rho-config`, not a trait in `rho-core`. There is nothing
to implement.

The real extension point of this crate is the same as every provider: a fourth crate
implements `rho_core::Provider`, and named profiles carry the new protocol string. Nothing
in this crate is a trait a third party plugs into.

### 7. Numbers marked provisional

The 90 second timeout is unmeasured. A first-byte p99 for `api.anthropic.com` under a
reasoning turn is the number the spec needs, and it is not in hand. So the value is
provisional, and a later measurement takes precedence. Same for `MAX_TOOL_INPUT_BYTES` and
the 256-block content cap.
