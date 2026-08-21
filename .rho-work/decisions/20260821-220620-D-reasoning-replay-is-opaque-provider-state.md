# A provider replays its own reasoning through one opaque, owner-tagged state value

Date: 20260821. Reference: `D-reasoning-replay-is-opaque-provider-state`.
Spec: `docs/specs/20260819-134615-SPEC-reasoning-across-providers.md`.

## The question

rho reads reasoning from three providers today. Azure sends an OpenAI-shaped reasoning item
with an id, a summary list, an encrypted blob, and a status. The spec's replay block is
`ReasoningReplay { text, signature: Option<String> }`. That shape cannot carry an id or an
encrypted blob, so rho can read a thing it can never send back.

Three options were put to the owner:

- **(a)** add a third typed variant, as jcode does with
  `OpenAIReasoning { id, summary, encrypted_content, status }`.
- **(b)** carry one opaque provider-state value per block, as fx does with
  `provider_state_json`.
- **(c)** keep the two-field block, and state that Azure replays nothing.

## The decision

Option **(b)**. The owner chose it on 20260821.

A replay block carries one opaque value. The provider crate that wrote the value is the only
code that reads it. Shared code never looks inside.

## Why

A typed variant per provider puts every provider's wire shape into `rho-core`. `rho-core`
then changes for each new host, and that is the opposite of the extension point rho promises.
One opaque value keeps the wire shape inside the crate that owns it.

fx proves the shape works. It sends `include: ["reasoning.encrypted_content"]` and replays the
returned blob from `provider_state_json`. See `fx-src/src/core/shared/types.zig:909`.

## What it rules out

- No typed OpenAI, xAI, or Gemini reasoning variant in `rho-core`.
- No shared code that reads the value, branches on it, or repairs it.
- No untagged blob. See the guards below.

## The guards, because an opaque value can fail open

fx does not tag its blob, and its own code cannot tell one provider's state from another's.
A model switch then sends a foreign blob. rho must not copy that hole.

1. The value carries its **owner**: the provider name and the model id that produced it.
2. A provider reads a value only when the owner matches. Otherwise it drops it and sends
   nothing. The drop is fail-closed, and it is a named arm.
3. The value never carries a credential. `rho-redact` covers it on the way to a file and to a
   log.
4. An old rho reads the text and ignores the value. A new rho reads a missing value as
   "nothing to replay".
5. The value is `serde_json::Value`, and it round-trips unchanged through the session file.

## What this settles elsewhere

The owner tag names the provider and the model. So the same-model rule of behaviour rule 6
reads the tag on the block, and `Message` needs no new field. That narrows the open question
about message provenance, and it removes a change to the persisted `Message` shape.

## What happens next

The block shape, the owner type, and the drop rule go into the spec before either side writes
code. The contract then gets a review of its own, as `AGENTS.md` step 3 requires.
