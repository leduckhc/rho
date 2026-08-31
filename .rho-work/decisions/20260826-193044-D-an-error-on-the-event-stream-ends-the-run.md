# D-an-error-on-the-event-stream-ends-the-run

Date: 20260826. Reference: `D-an-error-on-the-event-stream-ends-the-run`.
Spec: `docs/specs/20260819-102749-SPEC-jsonl-frontend.md`, section 6.

## The question

`pump_run` reads a run's events until the stream ends. What should it do when an error
arrives on that stream: keep reading, or settle the run?

The first answer was "keep reading, and settle when the stream ends". A test hung for
more than sixty seconds.

## The rho-core defect that decided it

`crates/rho-core/src/agent.rs` line 569:

```rust
TurnOutcome::Failed | TurnOutcome::Closed => return,
```

That `return` skips line 593, `self.inner.queue.unobserve()`.

`Session::prompt` spawns a second task that forwards steering-queue announcements into
the run's event channel, and that task holds a **clone of the event sender**. It ends
only when the queue drops its observer, which is what `unobserve` does. So on a provider
failure:

1. The driver sends `Err(..)` and returns, with no `AgentEnd`.
2. `unobserve` never runs, so the queue keeps the observer sender.
3. The forwarder task lives, so the event channel never closes.
4. **The event stream never ends.**

Any consumer that reads to the end of the stream hangs for ever after a provider
failure. That is not only this crate. It is one leaked task per failed run as well.

A live probe against Bedrock and a scripted test both showed it. See
`docs/verification/jsonl-frontend.md`.

## The decision

**An error on the event stream ends the run.** `pump_run` writes the `Fault`, then writes
`Settled { stop_reason: faulted }`, and returns. It does not wait for the stream to close.

This is true of rho-core today, and not only a way around the hang. Both sites that send
an error return `TurnOutcome::Failed`, and the driver loop then leaves the run. So an
error really is terminal, and the contract now says so out loud.

## What this decision does not do

**It does not fix the rho-core defect.** The leaked forwarder task and the stream that
never closes are still there. Another worktree owns the steering queue and its observer,
so this lane must not edit it. The fix is small and it belongs there:

```rust
TurnOutcome::Failed | TurnOutcome::Closed => {
    self.inner.queue.unobserve();
    return;
}
```

Until that lands, `rho-tui` and any later `rho-acp` bridge have the same hang waiting for
them, because both read the same stream. This decision is the record that the defect is
known, reproduced, and unfixed here on purpose.

## Amended 20260831: the rho-core defect is fixed

The queue worktree merged, so the debt above came due and is now paid. See
`D-a-failed-run-releases-the-queue-observer`. Two corrections to what this decision claimed:

- It says "both sites that send an error". There are **three**: a failed `stream` call, an
  error mid-stream, and a stream that ends with no `Done` event.
- The fix is not the snippet above. Six early returns skipped the release, so adding a line
  to two of them would leave four, and a seventh return could still forget. `run` now wraps
  `run_inner` and owns the release, so no exit can miss it.

What this decision decided still holds unchanged: an error on the event stream ends the run,
and a frontend settles on the error item rather than waiting for the stream to close. The
stream now ends as well, so a frontend that waits is merely slower, not stuck.

## What it rules out

- No frontend code that waits for the event stream to close before it settles a run.
- No second `Fault` for one run after the first error.
- No claim that the rho-core leak is fixed.
