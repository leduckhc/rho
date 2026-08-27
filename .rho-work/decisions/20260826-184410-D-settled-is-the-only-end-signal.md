# D-settled-is-the-only-end-signal — no RunEnd, and no will_retry field

Date: 20260826. Reference: `D-settled-is-the-only-end-signal`.
Spec: `docs/specs/20260819-102749-SPEC-jsonl-frontend.md`, sections 3.3 and 5.

## The question

The draft spec gave the wire two end events. `RunEnd` closed one low-level run and carried
`will_retry`. `Settled` closed the whole prompt. Does `rho-jsonl` need both?

## What the code showed

`will_retry` can never be true. Retry lives inside a provider crate, below the event
stream. `rho_provider_openrouter::OpenRouterConfig` and `rho_provider_azure::AzureConfig`
each hold a `RetryPolicy`, and each provider retries inside its own `stream` call. The
agent loop never sees a retry, so it emits no event for one.

`rho_core::AgentEvent` has one end variant, `AgentEnd`. The loop emits it once per run that
reaches its end **normally**. A run that fails at the provider emits none at all, which is
what `D-the-frontend-settles-every-prompt` is about.

So for a normal run, `RunEnd` would always carry `will_retry: false` and would always sit one
line before `Settled` with the same reason. For a failed run it would carry no reason worth
having. Neither case earns a second event.

## The decision

`Settled` is the only end signal. `RunEnd` does not exist. `will_retry` does not exist.

A future retry loop above the agent adds a new event variant. A new variant is an
extension, and section 8 of the spec already allows it.

## Why not the alternatives

- **Ship `RunEnd` now, for the future retry loop.** A field no test can reach is dead
  surface, and dead surface is a defect class here. See `D-dead-surface-is-a-defect-class`.
  It is also a trap: the draft spec had to warn a client not to stop reading at `RunEnd`.
  A contract that needs a warning about one of its own events is too big.
- **Emit `RunEnd` only when a retry happens.** Nothing can emit it, so it is the same dead
  surface with a longer explanation.

## What it rules out

- No `RunEnd` event. No `will_retry` field anywhere in `rho-jsonl`.
- No client instruction to prefer one end event over another. There is one.
