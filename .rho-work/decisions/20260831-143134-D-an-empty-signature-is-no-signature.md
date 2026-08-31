# D-an-empty-signature-is-no-signature — a value, not only a key

Date: 20260831. Reference: `D-an-empty-signature-is-no-signature`.
Spec: `docs/specs/20260819-134615-SPEC-reasoning-across-providers.md`, rule 8.

## The question

`replay_block` refuses a payload that carries no signature. It read the key like this:

```rust
let Some(signature) = value.get("signature").and_then(Value::as_str) else {
    return Err(ReplayDropReason::NoSignature);
};
```

A key holding `""` makes `Value::as_str` answer `Some("")`. Is that a signature?

## The defect

No. It is not a signature, and Bedrock rejects the whole turn that carries it.

`ReasoningTextBlock::builder().signature("")` builds without complaint, so the block travelled.
A review reproduced it against the public API:

```
[absent-key]   reasoning_blocks_sent=0 signature_on_wire=None      dropped_replays=[NoSignature]
[empty-string] reasoning_blocks_sent=1 signature_on_wire=Some("")  dropped_replays=[]
```

Two things were wrong at once. The block travelled, and the refusal was never reported, so the
reader learned nothing either. The doc comment on `a_state_with_no_signature_is_dropped` already
claimed the opposite: "dropped rather than sent with an empty signature, which Bedrock would
reject". The claim was true of an absent key and false of an empty value.

This is the sprint 1 family. The fixture described a response, and the defect was in the request.

## The decision

**An absent key, an empty value, and a value of blanks are one case.** All three answer
`Err(ReplayDropReason::NoSignature)`, and all three appear in `dropped_replays`.

```rust
let Some(signature) = value
    .get("signature")
    .and_then(Value::as_str)
    .filter(|signature| !signature.trim().is_empty())
else {
    return Err(ReplayDropReason::NoSignature);
};
```

`trim` is in the decision on purpose. A signature of spaces is no more signed than an empty one,
and a provider that pads a field would otherwise reach the wire.

## What it rules out

- No new `ReplayDropReason` variant for an empty signature. A reader that handles `NoSignature`
  handles every shape of it, so no side has to change.
- No test that pins the key and not the value. `a_state_with_an_empty_signature_is_dropped`
  asserts both halves: nothing travelled, and the drop was reported.
- No silent drop. The refusal is data, per `D-a-drop-report-is-data-not-a-log-line`.

## Test cases

- `a_state_with_an_empty_signature_is_dropped` — an empty value sends nothing and reports
  `NoSignature`.
- `a_state_with_a_blank_signature_is_dropped` — a value of blanks does the same. Removing
  `trim` from the filter fails this test alone, so it is sharp.
