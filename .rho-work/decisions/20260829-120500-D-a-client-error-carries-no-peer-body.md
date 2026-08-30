# D-a-client-error-carries-no-peer-body — a 4xx reports its status, and nothing the peer said

**Question:** `ProviderError::Client { status, message }` carried the provider's response body,
and rho prints that error. A host that reflects the `Authorization` header therefore put a
resolved credential on rho's stderr. Reproduced with a loopback stub, a `!command` credential,
and a user-set `base-url`:

```
rho: client error: status 400: {"error": {"message": "bad request; your header was Bearer sk-SECRET-FROM-HELPER"}}
```

**Decision: rho never reads a 4xx response body, and `Client` cannot hold one.** The variant
becomes `Client { status: u16, advice: &'static str }`.

## Why `&'static str`, and not a scrub

The type is the guard. A response body is a `String` built at run time, and it does not coerce
to `&'static str`. So the compiler refuses a peer's bytes in this field, and no author has to
remember a rule.

A scrub was the weaker option the review offered. It fails for the reason `rho-redact` exists:
filtering after the fact is a promise nobody can keep. `Bearer <token>` is one shape. A gateway
may echo the key in a JSON field, a URL, a quoted header dump, or base64. One missed shape is
one leaked key, and the user never learns it happened.

rho also stops calling `response.text()` on a failure. The body is used by no arm of
`status_to_error`, so the bytes never enter the process at all. That is stronger than any
check on a value rho is holding.

## What a user loses, and what they gain

They lose the gateway's own words on a 400. That is a real cost for `base-url`, which exists to
reach Ollama, vLLM, LiteLLM, and LM Studio, and whose most useful signal is often a body saying
"model not found".

So the advice says where the body went, and points at the one place that still has it:

> the provider refused the request. rho does not show the body, because a body can echo the
> credential. Read the host's own log for the reason.

A local gateway is a process the user runs, so its log is theirs to read. A hosted provider has
a dashboard. Neither needs rho to relay untrusted bytes.

## Ownership, stated plainly

The variant predates this work, and `base-url` arrived with `SPEC-wire-the-dead-switches`.
Neither made a credential travel that path from a config file or a shell helper. The credential
lane did, so the fix lands with the credential lane.

`AGENTS.md` states the rule with no exception: "No secret in a log, including at `trace` level.
Redact by construction."

## Rules out

**Keeping the body behind a cap and a scrub.** See above. A cap bounds the damage and does not
stop it.

**Logging the body at `debug` or `trace`.** The rule names `trace` explicitly.

**Deleting `advice` and reporting the status alone.** Bedrock already writes a useful sentence
of its own for a validation failure, and that sentence is rho's own words. A `&'static str`
keeps it and forbids the peer's.

**Fixing OpenRouter alone.** Azure had the identical line. Bedrock was already correct, because
it used a literal. The review found one; all three are checked, and each provider now carries a
`&'static str`.
