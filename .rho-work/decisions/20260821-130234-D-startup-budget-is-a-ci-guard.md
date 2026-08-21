# D-startup-budget-is-a-ci-guard — a latency budget fails the build


**Question (controller):** `docs/benchmarks.md` records a first-frame number and a binary
size. Nothing fails when either one regresses. fx enforces a 2 ms wall-clock mean for
every noninteractive command. Does rho adopt that?

**Decision:** Yes. rho adds a per-command latency budget, checked in CI, that fails the
build when a command's mean exceeds its limit. The commands measured are the ones a script
calls in a loop: version, help, and any future noninteractive status or session listing.
The budget is a number in a file, not a number in a document.

The gate reports a process baseline next to each result. A budget without a baseline
blames rho for the cost of starting any process at all.

**Reason:** AGENTS.md step 12 says to turn a defect into a guard, and a slow start is a
defect that no test can see. `docs/benchmarks.md` is a measurement, and a measurement
rots. rho's own history proves it: two reports claimed work that was not done, and one
measured prose at the wrong limit. A number that CI checks cannot rot in silence.

This matters more for rho than for fx. The owner's goal is 50 concurrent sessions, so a
cost paid at startup is paid 50 times.

**Reason to measure a mean, not a minimum:** a minimum reports the best cache state the
machine ever reached. A user meets the mean.

**Rules out:** a budget that only warns. A budget stated in prose with no runner. Claiming
a startup win from a single run, which AGENTS.md already forbids. Copying fx's 2 ms number
as rho's target, because rho has not measured its own floor on the same host.
