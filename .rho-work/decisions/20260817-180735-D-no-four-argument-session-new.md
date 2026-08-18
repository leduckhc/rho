# D-no-four-argument-session-new — Remove the four-argument `Session::new`


**What the controller found.** The S4b developer kept a four-argument
`Session::new` so the existing tests would compile without edits. That
constructor silently supplied three values:

- the model id `"test-model"`,
- the current directory as the session root,
- `AllowAllPolicy`, which approves every tool call.

So the most discoverable constructor in the crate carried an insecure default
and a fake model id. A production caller would send `model: "test-model"` to a
real provider, and would confine tool paths to whatever directory the process
happened to start in. That is the exact accident D-session-config forbids.

**Decision:** delete the four-argument `Session::new`. `Session::with_config` is
the only constructor. The two test helpers now build an explicit config through a
new `common::test_config()` helper, which documents every permissive choice it
makes and says that production code must not copy it.

**The controller authorised a test edit here.** The rule that a developer must
not edit a test exists to stop a developer bending a test to fit a broken
implementation. This was the opposite case: the shape of the test was forcing a
defect into the public API. The rule says to escalate that instead of working
around it. The developer did escalate. So the controller decided, and made the
change itself.

**Result:** 47 tests still pass. Clippy is clean. No `todo!()` remains.
