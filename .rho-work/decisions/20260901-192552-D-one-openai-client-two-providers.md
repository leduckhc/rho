# D-one-openai-client-two-providers — `rho-provider-openrouter` gains a second `Provider`

**Question:** rho needs to reach xdent's OpenAI routes and real `api.openai.com`.
`rho-provider-openrouter` already speaks OpenAI Chat Completions. Does rho ship a new crate
or a new `impl` in the existing one?

**Decision: a new `Provider` impl in the existing crate, not a new crate.**

```rust
// in rho-provider-openrouter/src/lib.rs
pub struct OpenAi {
    config: OpenAiConfig,
    client: reqwest::Client,
}

impl Provider for OpenAi {
    fn id(&self) -> &str { "openai" }
    async fn stream(...) -> Result<ProviderStream, ProviderError> { /* same wire */ }
}
```

The wire logic is already there: `build_request_body`, `map_chunk`, `status_to_error`. What
changes is `Provider::id`, the default `base_url`, and the credential variable name. No
duplication.

## Why not a new crate

A new crate would copy the entire request builder, SSE parser, retry loop, error mapper, and
tool-call assembler. That is 600+ lines. The difference is 3 lines: `id()`, `base_url`
default, credential name.

**The protocols are identical.** OpenRouter speaks OpenAI Chat. A live probe confirmed:
xdent's `/chat/openai/v1` and OpenRouter's `/api/v1/chat/completions` take the same POST
body and return the same SSE shape. The difference is the URL.

## Why not an environment variable or config flag to switch the ID

That breaks the `Provider` contract. `Provider::id()` is a method, not a field. The id is
stable per provider, not per config.

A provider with two ids would also break credential resolution, which uses `Provider::id()`
as the config key. See `D-a-provider-names-its-own-credential`.

## What it costs

One new public type, `OpenAi`. One new public config type, `OpenAiConfig`. Both live in
`rho-provider-openrouter/src/lib.rs`, beside `OpenRouterProvider` and `OpenRouterConfig`.

The crate description becomes "OpenRouter and OpenAI Chat Completions provider for rho." It
already says "OpenRouter and OpenAI-compatible provider", so this tightens that.

**The testkit.** `rho-provider-openrouter` does not call
`rho_provider_testkit::contract::run_all` today. The reason is in the defect list of
`.rho-work/plans/20260831-the-launch-interface.md` item 3: the testkit has no caller. That
is true for both `Provider` impls. The spec says the crate calls it once this lands, with
both providers.

## Rules out

**A new crate.** That duplicates the wire client.

**A config flag that changes `Provider::id()`.** That breaks the contract and the credential
resolution.

**Merging the two configs into one with a toggle.** That hides which default `base_url` and
credential variable apply. Separate types make each rule clear.
