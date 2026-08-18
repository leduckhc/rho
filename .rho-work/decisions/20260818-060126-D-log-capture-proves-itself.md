# D-log-capture-proves-itself — A log capture in a test must prove itself first

**Question (controller, flakiness hunt):** a warning test failed one run in twenty. Why,
and what stops the whole family of log assertions from lying?

**Decision:** A test capture installs a global subscriber once per test binary, and it
writes into a thread-local buffer. Every capture helper emits a known probe line and
asserts the probe arrived, before a caller trusts the captured text.

**Reason:** A thread-local subscriber alone is not enough. With no global subscriber,
`tracing` reports the current level filter as `OFF`, so a `warn!` takes its fast path and
never reaches the capture. Whether that happened depended on which test ran first, so the
failure was one run in twenty. The global subscriber sets the filter, and the thread-local
buffer keeps parallel tests apart. After the fix: 25 clean runs of `rho-core`, and 12 of
`rho-config`.

The probe matters more than the flakiness. `a_resolved_credential_never_reaches_a_log`
asserts that a secret is **absent** from a log. A broken capture makes that test pass
against an implementation that prints the credential in full. That is the vacuous-test
family from decision D-bash-line-cap, in a new place: a test that cannot fail is worse than no test.
So the capture proves itself, and then the absence means something.

**Rules out:** A thread-local-only capture. An assertion on an empty log with no proof that
the log could have held anything. A flaky test left in the suite because it usually passes.
