# D-a-callsite-caches-interest-globally — a thread-local log capture cannot be repaired, so it goes

**Question (controller, flake hunt):** `a_dropped_payload_is_reported` in
`rho-provider-bedrock` fails once in a while under a whole-workspace parallel run. Why? And
is the cause the one `D-log-capture-proves-itself` already named?

## What was known

The test failed twice, on two trees, in two people's runs. It passed six times alone, and
three full-workspace runs on `main` did not reproduce it. So it is rare, and load bound.

## The mechanism, measured

A scratch crate outside the repository copied the shape of the helper. It holds a
thread-local subscriber, a canary line, and a library function that warns. Beside it sit 32
other tests. Each one reaches the same callsite with no subscriber installed.

`tracing` caches each callsite's interest in a process-global atomic. `Dispatchers::rebuilder`
returns `JustOne` whenever one dispatch or fewer is registered. `JustOne` then asks **the
calling thread** for its default subscriber. A thread with no subscriber answers
`NoSubscriber`, and `NoSubscriber::register_callsite` returns `Interest::never()`. That
answer is stored globally, for every thread. The capture thread then skips its own `warn!`,
because the macro reads the cached interest before it asks any subscriber.

So the first test to reach a callsite decides whether every later test can see it.

| Harness | Runs at `--test-threads=16` | Failures |
| --- | --- | --- |
| thread-local subscriber, as `rho-provider-bedrock` had it | 400 | 6 |
| global subscriber, thread-local buffer | 400 | 0 |

Six in four hundred is 1.5 percent. That matches two sightings in a few dozen runs.

## What the measurement refutes

The failing assertion was reported as the canary, "the capture is live". **The canary never
failed once in 400 runs.** Every failure lost the drop report and kept the canary. The level
filter read `trace` at the moment of failure.

The canary line sits inside the closure, so only a thread that holds the subscriber can
reach it. `NoSubscriber` can never register it, so its interest cannot be poisoned this way.
Either the report of which assertion failed was imprecise, or a second mechanism exists that
this work did not find. The record says so, rather than claiming a proof it does not have.

`D-log-capture-proves-itself` named a different cause: with no global subscriber the level
filter reads `OFF`, so a `warn!` takes its fast path. That cause is real, and it is not this
one. `MAX_LEVEL` starts at `OFF`, and the first `Dispatch::new` raises it to `TRACE` before
the closure runs. So the level filter explains a loss before the first capture, and the
interest cache explains a loss inside one.

## The decision

`D-log-capture-proves-itself` already stated the shape of a capture. It was never applied in
`rho-provider-bedrock`. It is applied now, in `crates/rho-provider-bedrock/src/lib.rs`:

- One global subscriber, installed once per test binary through a `OnceLock`.
- It writes into a thread-local buffer, so parallel tests never read each other's lines.
- Each capture still emits a probe and asserts the probe arrived.

A global default subscriber makes the poisoning impossible, and not merely unlikely. Every
thread now has a default that wants every callsite, so `Interest::never()` has no source.

**Both mechanisms need the same cure**, so one fix ends both.

## Rules out

**A thread-local capture, anywhere in this tree.** `tracing::subscriber::with_default` alone
cannot hold a log assertion, whatever probe sits beside it.

**`rebuild_interest_cache` as the fix.** It repairs the cache after the subscriber is
installed. It leaves a window: another thread may register a fresh callsite right after the
repair. A global subscriber removes the source instead of repairing the damage.

**Calling this fixed because it passed once.** The proof is a run count, and the report
carries it.

## Still open, and not this lane's to fix

The same thread-local shape sits in `crates/rho-core/tests/reasoning_replay.rs`,
`crates/rho-config/tests/redaction.rs`, and `crates/rho-provider-azure/tests/azure.rs`.
Two of those crates belong to other lanes right now. Each one is the same latent flake.
