# D-the-minimal-gate-runs-its-tests — building them proves less than running them

**Question:** the ship gate holds
`cargo test -p rho-cli --no-default-features --features minimal --no-run`. It compiles the
minimal test binaries and never runs them. Is that enough?

**Decision: no. The gate runs them.** The `--no-run` is dropped, and the gate entry is renamed
from `minimal-test-build` to `minimal-test`.

## The evidence

A test review of the credential lane asked whether
`check_provider_name_agrees_with_build_provider` could distinguish anything. It pins two name
lists together: the `match` in `check_provider_name` and the `match` in `build_provider`. A
drift between them makes the JSONL frontend report an unknown provider for a provider that
works, or the reverse.

The default build compiles all three providers, so no name is ever `NotCompiled` there. The
whole `NotCompiled` arm is dead in the default build.

So the rule was broken on purpose. `"bedrock" => cfg!(feature = "bedrock")` became
`"bedrock" => true`, which claims a provider is compiled when it is not:

```
default build:  test result: ok. 1 passed
minimal build:  assertion `left == right` failed: the two name lists disagree about "bedrock"
                test result: FAILED. 0 passed; 1 failed
```

The defect is invisible to the whole default suite and visible in one second under minimal.
`--no-run` compiled that test and threw the answer away.

## What it costs

Nothing measurable. Every minimal test already passes, 233 of them, in about one second.
Running a test binary also compiles it, so the new command is strictly stronger than the old
one and nothing is lost.

## Rules out

**Keeping `--no-run` and adding a second command.** Running the tests compiles them, so two
commands would build the same binaries twice for no gain.

**Making the agreement test cover the arm in the default build.** It cannot. `cfg!` is decided
at compile time, so a build with every provider has no uncompiled provider to ask about. The
coverage needs the other build, not a cleverer test.

**Gating the whole workspace on minimal.** Only `rho-cli` has a minimal feature set, and only
its provider selection has a compiled-in question. A workspace-wide minimal run would build
crates whose features do not vary.

**Leaving it for a later lane.** AGENTS.md forbids leaving a verified bug unfixed, and this is
a verified blind spot in the gate itself, found by breaking the rule on purpose. Step 12 asks
for exactly this.

## What must stay true

`agentic-workflow.yaml` `gate_sets.full` and the `## Gate` block of `AGENTS.md` are compared
command for command by `bench/check-agentic-workflow.py`. Both were changed together, and that
checker is the proof they still agree.
