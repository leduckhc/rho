# D-a-model-descriptor-carries-no-capability-claim

Date: 20260831-201551

## The question

What does `ModelDescriptor` carry? The user asked for a "fast or normal" switch, so is speed
a field?

## The decision

`ModelDescriptor` carries two fields only: `id` and `display_name: Option<String>`.

- No speed field. A listing API does not report speed for a tool-calling run.
- No tool-use field. `docs/verification/models.md` proves a listed model may not call a tool.
- No price, context-length, or region field.

Speed is a separate concept. `/speed fast` sets `ReasoningEffort::Off`. `/speed normal`
restores the effort the session started with. Speed is an alias over the existing effort
ladder, so it stores no new state.

## What this rules out

- A `supports_tools` boolean. rho cannot prove it from a listing.
- A `speed` or `latency` field on the descriptor.
- Any picker badge that claims a capability.

## Why

`AGENTS.md` forbids a claim without a measurement, and says to prefer the smaller interface.
A listing proves a model exists. It never proves the model works for a run. So the descriptor
states existence only. A verified result stays in `docs/verification/models.md`, written by a
human from a real run.
