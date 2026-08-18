# D-tui-plugin-render-budget — A terminal view render has an 8 ms budget and a worker thread


**Question.** A terminal view render runs in rho's process. A slow or panicking view could
freeze or crash the interface. How does rho bound it, and how does the session survive?

**Decision.** rho runs each view render on a worker thread and reads the result with a
deadline of `PLUGIN_RENDER_BUDGET_MS`, which is 8 milliseconds. rho treats a timeout and a
panic the same way:

- On a timeout, rho drops the output, draws a dim `plugin <name> skipped` placeholder, and
  disables the view for the rest of the session. rho cannot force a runaway worker to stop,
  so it stops feeding it and ignores its result.
- On a panic, the worker thread unwinds and closes its result channel. rho sees the closed
  channel, draws `plugin <name> disabled`, and disables the view. rho never calls `unwrap`
  on a view result.

So the session survives a slow or panicking view, as it survives a Tier-1 tool plugin
today.

**Reason.** 8 milliseconds leaves headroom under a 16 millisecond frame for rho's own
render. A redraw happens on a state change, not on a timer, so a worker thread per view is
cheap enough. The budget makes the drop deterministic in a test, with a fake clock in the
harness while the view still reads no clock.

**The `panic = "abort"` caveat.** The worker-thread isolation needs unwinding. The release
profile in the workspace `Cargo.toml` sets `panic = "abort"`, which turns any panic into a
process abort before the thread can unwind. So the survival guarantee holds under the test
profile, which unwinds, and it does not hold in a release build until the plugin path
unwinds or renders out of process. `SPEC-tui-plugins` section 10 defers a release-safe
isolation and names this caveat.

**What it rules out.**

- An unbounded render that can hang the interface.
- A view that reads a clock. rho passes a `tick` counter instead, so motion stays pure.
- A slow view that keeps costing time. rho disables it after one timeout.
- A claim that a release build survives a view panic. That waits for the isolation fix.
