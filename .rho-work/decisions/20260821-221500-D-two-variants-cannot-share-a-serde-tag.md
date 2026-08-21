# Two enum variants cannot share one serde tag, and the reader says nothing

Date: 20260821. Reference: `D-two-variants-cannot-share-a-serde-tag`.
Spec: `docs/specs/20260819-134615-SPEC-reasoning-across-providers.md`, section 4.

## The question

The spec promised that an old rho can load a new session file. `ContentBlock` is a serde
tagged enum, so the new `ReasoningTrace` and `ReasoningReplay` variants had to serialise under
the old `"type": "thinking"` tag. The spec stated that as a fact. Nobody compiled it.

## What the compile showed

Both variants took `#[serde(rename = "thinking")]`. The code compiled with no warning. Then:

```text
replay -> {"type":"thinking","thinking":"a","replay":true,"state":null}
read replay back -> Ok(ReasoningTrace { thinking: "a" })
```

serde returns the **first** matching variant. So every replay payload came back as a trace.
The provider then sent no reasoning, the model rejected the tool loop, and no error named the
cause. This is the silent drop that `AGENTS.md` step 8 tells us to hunt.

## The decision

The file shape is its own private type, `DiskBlock`. `ContentBlock` carries
`#[serde(from = "DiskBlock", into = "DiskBlock")]`, and one `From` impl each way holds the
rules. `DiskBlock::Thinking` has `thinking`, `replay`, `state`, and a read-only `signature`.

`replay: true` with a state reads as `ReasoningReplay`. Every other case reads as
`ReasoningTrace`. That is fail-closed, so a half-written record never replays.

## Why not the alternatives

- **Two tags, `reasoning_trace` and `reasoning_replay`.** An old rho then fails the whole
  load, which the spec forbids.
- **One in-memory variant with a `replay` flag.** The compiler then stops proving that a
  trace never travels, and that proof is the reason for the split.

## What it rules out

- No `#[serde(untagged)]` on `ContentBlock`. An untagged reader guesses, and a guess here is
  the same silent drop by another road.
- No second place that maps a block to a file record. `DiskBlock` is the one boundary.

## The evidence

Transcribed to a scratch crate outside the repository, at `/tmp/rho-contract-scratch`. Four
tests pass: `a_state_round_trips_through_the_session_file`,
`an_old_thinking_block_imports_as_a_trace`, `a_replay_key_with_no_state_reads_as_a_trace`, and
`a_new_block_is_readable_by_an_old_rho`. The four names are now in the spec's test list.

## The lesson

A persisted format is a contract with a future version of rho. This one was wrong in the
reviewed spec, and only a compile found it. So a spec that states a serde shape must carry a
scratch-crate round trip before either side writes code.
