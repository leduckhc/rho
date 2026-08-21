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
3. The value carries no credential and no readable text. It holds opaque provider bytes only:
   a signature, an encrypted blob, an id, or a status. Readable text lives in `text`, where
   the session cap and the reader both already reach it.
4. The value never reaches a log, at any level. It is written to the session file verbatim,
   because a rewritten payload cannot replay. `redact_block` and `cap_block` each get a named
   arm, so no wildcard covers a reasoning block in silence.
5. A value over `MAX_RECORD_BYTES` is dropped whole. A truncated opaque token is useless.
6. An old rho reads the text and ignores the value. A new rho reads a missing value as
   "nothing to replay", and it reports a `replay: true` record that carries no value.
7. The value is `serde_json::Value`, and it round-trips unchanged through the session file.

**The owner tag is accident protection, and not authentication.** It stops an honest mismatch
after a model switch. A crafted session file can set any owner, because the tag sits beside
the payload it describes. A security review named this, and the claim is corrected here rather
than left standing. rho trusts a session file exactly as much as it trusts the rest of that
file.

## The review changed two things

A reviewer and a security pass both read this decision before any code. Two findings landed:

- **A tool call now carries a `ProviderState` too.** The draft kept a typed
  `thought_signature: Option<String>`. No stream event carried it, so Gemini would have had to
  edit shared code, and a bare string had no owner, so it replayed after a model switch with
  nothing checking it. That was the exact hole this decision exists to close, on a second path.
- **The payload is bounded, and its text is not opaque.** See guards 3 and 5.

## What this settles elsewhere

The owner tag names the provider and the model. So the same-model rule of behaviour rule 6
reads the tag on the block, and `Message` needs no new field. That narrows the open question
about message provenance, and it removes a change to the persisted `Message` shape.

## What happens next

The block shape, the owner type, and the drop rule go into the spec before either side writes
code. The contract then gets a review of its own, as `AGENTS.md` step 3 requires.
