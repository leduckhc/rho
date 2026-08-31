# D-a-failed-run-releases-the-queue-observer — one exit, one release

Date: 20260831. Reference: `D-a-failed-run-releases-the-queue-observer`.
Spec: `docs/specs/20260818-014343-SPEC-steering.md`, and
`docs/specs/20260817-164906-SPEC-core-runtime.md` section 4.
Closes the debt `D-an-error-on-the-event-stream-ends-the-run` recorded and deferred.

## The question

`Driver::run` released the steering-queue observer on its last line. Six early returns never
reached that line. Two of them end a failed run. What releases the observer then?

## The defect

Nothing did. `Session::prompt` spawns a task that forwards queue announcements into the run's
event channel, and that task holds a clone of the event sender. It ends only when the queue
drops its observer. So a failed run left the channel open for ever:

- one leaked task per failed run, for the life of the process,
- a stale observer pointing at a dead run,
- and no end to the event stream, so a consumer that reads to the end waits for ever.

A probe against the public API measured it: `stream_closed_cleanly=false`.

Four consumers already worked around it, each with its own comment: `rho-cli` breaks out of the
loop on the error item, `rho-jsonl` settles the run on it, `rho-tui` calls `end_run` itself, and
`collect_report` treats it as a child failure. **Four workarounds for one defect.** A fifth
frontend would have met the hang, and the workarounds only hold while every `Failed` return is
preceded by an error item. Nothing pinned that.

The earlier decision also under-counted the sites. It says "both sites that send an error".
There are three: a failed `stream` call, an error mid-stream, and a stream that ends with no
`Done` event.

## The decision

**The release belongs to the exit, not to the exits.** `run` became a two-line wrapper:

```rust
async fn run(self, input: Vec<ContentBlock>) {
    self.run_inner(input).await;
    self.inner.queue.unobserve();
}
```

`run_inner` holds the loop and may return from anywhere. A seventh early return cannot forget
the release, because it cannot reach the line that performs it.

The alternative was to add `unobserve()` to each of the six returns. That is the shape that
produced the defect, so it is rejected: a rule every caller must remember is a rule a caller
will forget. This is the same argument that made `SessionWriter::seed_ids` private.

## What this does not change

A failed run still emits **no** `AgentEnd`. An error on the event stream is terminal, and a
frontend settles on the error item, per `D-an-error-on-the-event-stream-ends-the-run`. This
decision changes when the stream **ends**, and not what it carries.

## What it rules out

- No `unobserve()` call inside the loop, and none at an early return.
- No frontend that must wait for the stream to close to learn a run failed. The error item is
  still the signal; the stream merely ends now as well.
- No test for this that waits without a timeout. A regression must fail the suite, not hang it.

## Test cases

In `crates/rho-core/tests/run_failure.rs`:

- `a_failed_provider_call_ends_the_event_stream`
- `two_failed_runs_each_end_their_stream` — the leak was per run, so the second one is where a
  per-run leak shows.
- `a_stream_that_ends_without_done_ends_the_event_stream` — the third error site, reached by a
  different route, because a guard that names one site cannot see a fourth.
- `a_successful_run_still_ends_with_agent_end` — the good path, so the leak is not fixed by
  closing the channel early.
- `a_message_pushed_after_a_failed_run_reaches_the_next_run` — the reason the release exists.
