# D-provider-openai-chat-owns-the-wire — rename before you share

Date: 20260901. Reference: `D-provider-openai-chat-owns-the-wire`.
Specs: `docs/specs/20260901-192552-SPEC-openai-chat-provider.md`,
`docs/specs/20260901-192553-SPEC-named-provider-profiles.md`.
Supersedes `D-one-openai-client-two-providers`, which rested on a false premise.

## The question

The openai-chat spec proposed a second `Provider` impl inside `rho-provider-openrouter`.
That saves duplication, but leaves the crate named for one specific service while it hosts
two. Should the co-location stand, or should the crate be renamed?

## What the earlier decision claimed

`D-one-openai-client-two-providers` said:

> The 600+ lines of wire logic already exist. What changes is 3 lines.

A contract review read the crate and disproved it. `build_request_body` hardcodes
`"max_tokens"`, and the openai-chat spec promises `"max_completion_tokens"` on the wire. So
the one wire difference the spec names lives in the shared code, and the co-location edits
shared code for the second impl. That is the exact shape AGENTS.md rejects.

The `stream`, `send_with_retry` and `send_once` methods are `impl OpenRouterProvider`, and
they read `self.config: OpenRouterConfig` and call OpenRouter-specific helpers like
`chat_url()`. A second impl cannot call them without a refactor. So the reuse is a rewrite,
and the crate's name is a lie the moment a second impl lands.

Meanwhile the parent decision, `D-a-provider-is-named-by-its-wire-protocol`, says a crate
is named by what it speaks, not who it connects to. The rename is the shape that decision
already committed to.

## The decision

**Rename `rho-provider-openrouter` to `rho-provider-openai-chat`.** The crate speaks the
OpenAI Chat Completions wire against any base URL. It hosts:

- One shared wire model, as free functions and a small inner type.
- Two `Provider` impls: `OpenRouterProvider` (config for OpenRouter's `/api/v1` prefix,
  its `HTTP-Referer` and `X-Title` headers, `max_tokens`, and its cost reporting) and
  `OpenAiProvider` (config for a plain OpenAI-compatible endpoint, `max_completion_tokens`,
  no cost field).

Both impls call the same request builder, which now takes the differences as parameters.

The refactor precedes the second impl. That means: lift `build_request_body`, `stream`,
`send_with_retry` and `send_once` out of `impl OpenRouterProvider` into free functions or
an inner type that both providers use. Then add `OpenAiProvider`. In that order. Otherwise
the "reuse" is a copy-paste.

## What it rules out

- **A `rho-provider-openrouter` crate hosting an `OpenAi` provider.** The parent decision
  forbids naming by service; hosting one service's provider under another's crate name is
  the same defect one level down.
- **A second copy of the wire code.** The point of one crate is one wire model.
- **A default that fails open.** A provider config with no base URL and no credential must
  not run. This carries the launch amendment on `SPEC-choose-a-model-and-configure-a-run`.
- **A shared method that reads OpenRouter-specific state.** Every helper is either free or
  takes the difference as an argument. A helper that reads `self.config.openrouter_field`
  cannot serve `OpenAiProvider`.

## What it does not decide

- The exact division between free functions and an inner type. The refactor picks the
  cheapest shape that keeps both providers honest.
- The `openrouter` provider id stays. It is a stable public name.
- The Cargo.toml key stays `rho-provider-openai-chat`. A workspace rename touches the
  workspace file and every `use` in `rho-cli`. Say so in the plan, not in the code.

## Test cases

- `the_openrouter_provider_still_answers_by_id_openrouter` — the rename must not break the
  existing provider id.
- `the_openai_provider_answers_by_id_openai` — new impl, new id.
- `both_providers_share_the_request_builder` — one call to the shared builder, verified by a
  test that asserts the same JSON keys for a shared field and the correct different key for
  the max-tokens field.
- `the_openai_provider_sends_max_completion_tokens` — the wire difference the review
  measured.
- `the_openrouter_provider_still_sends_max_tokens` — the guard against a refactor that
  changes the wire under a live user.
