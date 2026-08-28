# D-the-frontend-settles-every-prompt — rho-core ends a failed run with no AgentEnd

Date: 20260826. Reference: `D-the-frontend-settles-every-prompt`.
Spec: `docs/specs/20260819-102749-SPEC-jsonl-frontend.md`, section 5.

## The question

The draft spec promised a client one rule: "the agent is done only after `Settled`". Does
`rho-core` always give `rho-jsonl` an end event to map onto `Settled`?

## What the code showed

No. `crates/rho-core/src/agent.rs` handles a provider failure like this:

```rust
Err(error) => {
    let _ = self.tx.send(Err(Error::from(error))).await;
    return TurnOutcome::Failed;
}
```

and the driver loop then does `TurnOutcome::Failed | TurnOutcome::Closed => return`. That
`return` skips the `AgentEnd` emit at the end of `run`. So a run that fails at the provider
ends with one `Err` on the stream and no `AgentEnd`.

A client that waits for `Settled` would wait for ever. That is the promise the draft made,
and the code cannot keep it.

## The decision

`rho-jsonl` owns the pairing. It emits exactly one `Settled` for every accepted `Prompt`,
whatever the event stream does.

**The trigger is the error, not the end of the stream.** When `rho-jsonl` reads an `Err`, it
writes a `Fault` and then `Settled`, and it stops reading that run. Waiting for the stream to
close instead would hang, because the same early return that skips `AgentEnd` also skips
`queue.unobserve()`, so a live sender clone holds the channel open for ever. See
`D-an-error-on-the-event-stream-ends-the-run`, which was written after this one and records
the hang.

A stream that ends with no `AgentEnd` and no error is the remaining case. That one settles on
closure, with `FaultKind::Incomplete`.

`Settled` needs a reason for that case, and `AgentStopReason` has no variant for it. So
`rho-jsonl` declares one local wire enum, `SettleReason`. It mirrors `AgentStopReason`
value for value and adds `faulted`. The map from `AgentStopReason` is exhaustive with no
wildcard arm, so a new core variant is a compile error. See
`D-the-wire-reuses-the-core-stop-reason`.

A test pins the pairing, not the example. It asserts that a faulting run still yields
exactly one `Settled`.

## Why not the alternatives

- **Change `rho-core` to emit `AgentEnd` after a failure.** It is the better long-term fix,
  and it is not this lane. Another worktree holds the agent loop. A frontend that depends on
  an unlanded core change is a frontend that does not work today.
- **Let the client time out.** A timeout is not a contract. It also hides the difference
  between a slow model and a dead run.
- **Reuse `refusal` or `end_turn` as the reason.** Both are lies, and a client would then
  treat a failed run as a finished answer.

## What it rules out

- No code path in `rho-jsonl` where an accepted prompt writes no `Settled`.
- No second `Settled` for one prompt. Exactly one.
- No use of a core stop reason to describe a fault.
