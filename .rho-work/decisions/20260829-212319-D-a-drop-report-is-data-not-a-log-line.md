# D-a-drop-report-is-data-not-a-log-line — the provider answers with its refusals

**Question (controller):** `a_dropped_payload_is_reported` can only watch a log, because
`build_messages_for_model` reports a refused reasoning payload by logging it. A log line is a
bad seam for an assertion. What should the seam be?

## The defect in the contract

`build_messages_for_model` returns `Vec<Message>`. Rule 8 of `SPEC-reasoning-across-providers`
says a drop is never silent. The only report was a `tracing::warn!`, so the only test that
could prove rule 8 had to install a subscriber and read text.

That is the shape `D-the-budget-test-needs-an-observable-difference` argues against. A rule
with no seam in the data is proved by a side effect, and a side effect is fragile. It broke:
see `D-a-callsite-caches-interest-globally` for the measured flake.

A substring match is also a weak assertion. `logged.contains("no signature")` passes on any
line that happens to hold those words.

## The decision

**`build_messages_for_model` returns its refusals as data.**

```rust
/// Why one stored reasoning payload did not travel to the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayDropReason {
    AnotherOwner,
    NoSignature,
    UndecodableRedaction,
    UnbuildableBlock,
}

/// One refused payload, named by where it sat and why it stayed behind.
#[derive(Debug, PartialEq, Eq)]
pub struct DroppedReplay {
    pub message_index: usize,
    pub reason: ReplayDropReason,
}

/// The messages a request carries, and the payloads that did not travel.
///
/// No `Default`, on purpose. A default value would say "nothing was refused".
#[derive(Debug)]
pub struct BuiltMessages {
    pub messages: Vec<aws_sdk_bedrockruntime::types::Message>,
    pub dropped_replays: Vec<DroppedReplay>,
}

pub fn build_messages_for_model(messages: &[Message], model: &str) -> BuiltMessages;
```

The log line stays. It is now a courtesy for a reader of a terminal, and no longer the
contract. Both the log and the data read one table, `ReplayDropReason::report`, so the two
can never drift.

## Rules that hold

**The wire does not change.** `apply_request` reads `.messages` and sends exactly the blocks
it sent before. `docs/verification/reasoning-replay.md` holds the live evidence that a
signature block travels, and that evidence still stands.

**No payload sits in the data.** `DroppedReplay` has no field for the payload, so rule 9 holds
by construction and not by care. A test asserts that the debug form of the data carries no
signature, so a later field cannot leak one in silence.

**One log field went away.** The old unbuildable-block path logged `%error` from the AWS SDK
`BuildError`. The four report sites became one, and that field went with them. A security
review judged the loss negligible: a `BuildError` names a missing required field, never the
payload. Dropping it is a small win for rule 9, because one less value crosses into a log.

**`ReplayDropReason` stays exhaustive.** A new refusal must break every reader. A
`#[non_exhaustive]` enum forces a catch-all arm on each reader, and a catch-all is how
`ToolKind::Other` approved every tool that forgot its kind. See
`D-plugin-does-not-classify-itself`.

**A payload outside the current tool loop is not in this list.** Rule 12 drops history by
design, and it is not a refusal. A report on every later turn would be noise, and noise is
how a real warning gets ignored. The doc comment on the field says so.

## The cost, stated plainly

`apply_request` is the only production caller, and it ignores `dropped_replays` today. So the
field is public surface that only a test reads. That is close to what
`D-the-budget-test-needs-an-observable-difference` rules out, and the difference is real but
narrow:

- The field is the function's **own answer**. It is not a collector injected as a parameter,
  not a new trait, and not a closure that only a test passes.
- Nothing in the signature exists for the test alone. A caller that wants to tell a user
  "your reasoning did not replay" reads the field with no change to shared code.

A reviewer was asked this question directly, and the answer is in the report.

## Rules out

**A second builder function.** A review already deleted `build_messages` so that one function
carries every caller. Two builders would undo that. See the comment in
`build_messages_for_model`.

**Passing a collector into the builder.** That is the four-argument mistake of
`D-no-four-argument-session-new`, and it is dead surface.

**Keeping the log as the contract.** A test that reads a log proves the log, and a substring
match proves less than an enum.

**Reporting a refusal only in the data.** A user who runs rho still needs to hear it, so the
log line stays.
