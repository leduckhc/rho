# rho progress ledger

Controller-maintained. One row per stage attempt. The controller writes this,
not the subagents.

| Stage | Attempt | Role | Result | Notes |
| --- | --- | --- | --- | --- |
| S0 | 1 | controller | pass | Workspace created with `cargo new`. Ten crates. Builds clean. |
| S1 | 1 | documenter | fail | Review found 4 blockers, 5 majors, 3 minors. |
| S1 | 2 | documenter | partial | Turn limit. Blockers fixed. Two files left. |
| S1 | 3 | controller | pass | Controller finished `architecture.md` and `comparison.md` inline. Prose check 0 violations. |
| S2 | 1 | architect | fail | Review found the ACP `cancelled` spelling bug. Real interop defect. |
| S2 | 2 | controller | pass | Controller fixed all five findings inline. Verified against the ACP schema. |
| S3 | 1 | tester | pass | 28 tests compile. 12 fail on `todo!()`. Clippy clean. |
| S4 | 1 | developer | partial | 29 pass, tests untouched. But three `todo!()` survived, one a security boundary. |
| S4b | 1 | developer | running | Implements `confine`, both approval policies, and `SessionConfig`. |
| S5 | 1 | tester | running | Provider red tests plus the reusable testkit. |
| S10a | 1 | designer | partial | Turn limit before the recommendation. Three mockups and shots done. |
| S10a | 2 | designer | pass | Recommendation written. Picklist accessibility fixed. |
| S10b | 1 | designer | partial | Site built. Turn limit before visual verification. |
| S10b | 2 | controller | pass | Controller verified in Chrome. Found and fixed the `compressHTML` whitespace bug. |
| S10c | 1 | designer | stall | Wedged 28 minutes on one inference step. Abandoned. |
| S10c | 2 | controller | pass | Controller fixed the narrow-screen overflow inline. Desktop height unchanged. |

## Stall log

Subagents stalled or hit a turn limit in 6 of 12 dispatches. Causes, in order of
frequency:

1. **Turn limit** (4 times: S1 fix, S10a, S10b, S3 wrap-up). The brief asked for
   more work than the turn budget allowed. Fix: split a stage into two smaller
   dispatches, and put the verification step in its own dispatch.
2. **Wedged inference** (1 time: S10c, 28 minutes with no output). No recovery.
   Fix: abandon and do the work inline when the task is small.
3. **Off-task web research** (1 time: S10b took a 429 from GitHub while scraping
   for something it did not need). Fix: state in the brief that everything needed
   is already on disk.

## False claims caught by verification

The controller re-runs every gate and greps the tree. Two false claims so far.

1. **S1 documenter** measured sentence length at 22 words when the limit is 20.
   The reviewer caught it.
2. **S4 developer** reported "no `todo!()` remains". Three remained, and one was
   `confine`, the path-confinement security boundary. The green suite hid it,
   because no test covered it. The controller caught it with a grep.

Lesson now in force: a green suite proves only what the tests assert. The
controller greps for `todo!`, `unimplemented!`, and `panic!(` before it accepts
any green claim.

## Final stage results

| Stage | Attempt | Role | Result | Notes |
| --- | --- | --- | --- | --- |
| S2 | 3 | reviewer | pass | Found the ACP `cancelled` spelling bug. Real interop defect. |
| S4 | 2 | reviewer | fail | Found the fail-open `ToolKind::Other` approval hole. |
| S4b | 1 | developer | partial | Landed `confine` and `SessionConfig`. Kept an insecure `Session::new`. |
| S4c | 1 | tester | partial | Four fixes with before-and-after proof. Turn limit before the coverage tests. |
| S5 | 1 | tester | partial | Testkit plus OpenRouter and Azure. Turn limit before Bedrock. |
| S5b | 1 | tester | pass | Bedrock red tests. 47 provider tests red for the right reason. |
| S6 | 1 | developer | pass | All three providers stream and assemble tool calls. Ran `git stash` by mistake. |
| S7 | 1 | developer | pass | Seven tools, the plugin host, and the end-to-end tool-calling proof. |
| S8 | 1 | developer | pass | TUI, CLI, and the first real benchmark numbers. |
| secops | 1 | secops | fail | Found the unbounded `bash` line reader and the plugin kind hole. |
| S9 | 1 | controller | pass | Live runs against OpenRouter and Bedrock. Found three more defects. |
| S11 | 1 | devops | pass | Footprint script and three CI guards. Failed once on a real portability bug. |

## Defects found, and what found them

Nine real defects landed in sprint 1. **Not one was found by a failing test that
already existed.**

| # | Defect | Found by |
| --- | --- | --- |
| 1 | ACP spells the stop reason `cancelled`; Rust emitted `canceled` | reviewer, against the on-disk schema |
| 2 | `CancelToken::cancelled` lost a wake on a multi-thread runtime | tester, by reasoning about the race |
| 3 | `confine`, the path boundary, was still `todo!()` in a green stage | controller, with a grep |
| 4 | `Session::new` hid a fake model, an accidental root, and blanket approval | controller, reading the diff |
| 5 | `ToolKind::Other` counted as non-mutating, so approval failed open | reviewer, reading the enum |
| 6 | Two TLS stacks, and a panicking `rustls-webpki` | GitHub Dependabot, on the first push |
| 7 | The `bash` line reader had no cap; 8 MB of output took 805 MB of memory | secops, with a driven reproduction |
| 8 | A plugin's self-declared `ToolKind` bypassed the read-only policy | secops, reading the proxy |
| 9 | A tool error killed the whole run, and a tool turn never emitted `TurnEnd` | **a live run against a real model** |

The pattern is consistent. The tests asserted the events they expected to see.
They did not assert the absence of a bad state, nor the completeness of a pairing,
nor a bound on memory. So the fixes added invariant tests rather than example tests.

Defect 9 is the strongest argument for stage S9. 222 tests passed while the product
was unusable, because any missing file ended the session.

## Sprint 2 stage results

`workflow-sprint-2.yaml` drives this sprint. The scope is config, the session log,
and the ACP frontend. ACP is the point of the project, because makit needs a cheap
ACP backend.

| Stage | Attempt | Role | Result | Notes |
| --- | --- | --- | --- | --- |
| T0 | 1 | controller | pass | Doc audit against the code. Nine feature rows were wrong. MCP had no rows at all. |
| T1 | 1 | architect | partial | SPEC-config, SPEC-sessions, SPEC-steering, and ADR-session-format. Session ops and steering added mid-flight by the owner. |
| T1 | 2 | reviewer | fail | Three blockers and five majors. Every one is a repeat of a sprint-1 defect family. |
| T1b | 1 | architect | pass | Eight findings fixed. SPEC-approval written. Five decisions added. |
| T1b | 2 | controller | pass | Controller found one more fail-open: a remembered allow would have covered every `bash` call. |
| T2 | 1 | tester | pass | 29 config tests, all red on `todo!()`. Every SPEC-config test name exists. |
| T4 | 1 | tester | partial | Turn limit. 39 session tests red. The pi import tests were missing. |
| T4 | 2 | tester | pass | Resumed with a narrow brief. Three pi import tests, in a new crate the spec named. |
| T4 | 3 | controller | pass | Controller found the widening rule had no API, so no test could reach it. |
| T2b | 1 | reviewer | fail | Proved three red tests pass against a real defect. Ten findings. |
| T2c | 1 | tester | partial | Turn limit after three fixes. Reported honestly, and named the exact stopping point. |
| T2d | 1 | tester | partial | Turn limit again, in `rho-core`. Most fixes landed, and the F2 proof passed. |
| T2e | 1 | tester | pass | `rho-config` hardening. The F9 proof failed the old test, as it should. |
| T2f | 1 | controller | pass | Controller finished the codec golden vector, and found the `Option` fault in `Config::approval`. |

### The red-stage review, and what proof looks like

The reviewer did not read tests and give an opinion. It wrote the wrong implementation in
a scratch crate under `/tmp` and ran the tests against it. Three tests passed against a
real defect.

| Test | Wrong implementation that passed it |
| --- | --- |
| the four widening tests | `check_resume_permission` that ignores the sandbox |
| `cancel_leaves_no_half_written_tool_pairing` | `record_cancel` that writes nothing |
| `a_giant_line_does_not_exhaust_memory` | `fs::read_to_string`, then reject by length |

The third one is defect 7 again. An assertion on a returned error cannot observe an
allocation. So `SessionReader` gained `read_from`, a `BufRead` seam, and a test drives it
with a reader that counts the bytes it hands out.

Two more claims died. `SPEC-sessions` said CI runs the codec tests with `fast-json` on and off,
and `rho-core` had no such feature. The feature now exists, the CI matrix is a T10 item,
and no document claims it works before then. A golden line now pins the codec bytes, so
the two modes are comparable at all.

The last fault was in a type. `Config::approval` was `ApprovalMode`, and the spec said the
default is unset. A type with no unset state forced `Config::load` to invent `read-only`,
which made the owner's `ask` default unreachable. See decision D-approval-option-not-enum.

### T5b, the pi importer, and why a fixture is not a proof

| Stage | Attempt | Role | Result | Notes |
| --- | --- | --- | --- | --- |
| T5b | 1 | developer | partial | Three fixture tests green. The importer failed on 21 of 60 real pi files. |
| T5b | 2 | controller | pass | Drop with a count, map an image, accept `modelId`. 60 of 60 files import. |

The developer did everything the brief asked. It made the three tests pass, it broke the
implementation on purpose, and it ran the importer against one real file. That one file
held no surprise, so the report was green.

The controller ran the importer over 60 real files. **21 failed.**

| Cause | Records in 60 real files | Old behaviour |
| --- | --- | --- |
| `custom_message` | 45 | error, whole import stopped |
| `compaction` | 5 | error, whole import stopped |
| role `bashExecution` | 1 | error, whole import stopped |
| block `image` | 170 | error, whole import stopped |

The fix is decision D-unmappable-pi-record-drops. An unmappable record drops and the drop is counted, so nothing
is lost in silence and one auxiliary record cannot fail a whole file. A real
`model_change` also names its model `modelId`, where the fixture wrote `model`. After the
fix: 60 of 60 files import, 15646 records map, and every drop is named.

This is the sprint-1 lesson again, in one sentence. **A fixture describes what an author
expects. A real file holds what exists.** So the sample size matters: one real file proved
nothing, and 60 real files found four defects.

### The green stages, and what driving them found

| Stage | Attempt | Role | Result | Notes |
| --- | --- | --- | --- | --- |
| T3 | 1 | developer | pass | 36 config tests green. Two breaks proved the fail-closed parse and the allowlist. |
| T3b | 1 | developer | pass | Closed two gaps the T3 report named. 45 tests. The timeout test returns in 1.00 s. |
| T5 | 1 | developer | pass | 47 session tests green, in both codec modes. Three breaks proved three boundaries. |
| T5c | 1 | developer | pass | The sink seam, the rewritten degrade tests, and one timestamp format. |
| T5d | 1 | controller | pass | A real drive of the operations found a reopen fault and a nameless io error. |

**T3 reported two gaps rather than hiding them.** `ConfigLayer::from_env` mapped two keys
while F-environment-variable-override claimed every key, and the credential command had no timeout although the spec
said it did. So a helper waiting for a biometric prompt would hang rho forever. Both are
closed, test first. The timeout test returns in 1.00 second against a child that never
exits, and it hung for ten seconds with the kill removed.

**T5 flagged a divergence instead of hiding it, and the measurement decided.** The writer
reopened the file per record, because three degrade tests removed the session directory and
an open descriptor on Unix keeps writing to an unlinked inode. Measured: a reopen costs
17567 ns per record, a held sink costs 1034 ns, and an `fsync` per record costs 3076785 ns.
So the tests were wrong, not the design. The writer now holds a sink, a test injects a sink
that fails on demand, and a real append costs 1762 ns per record. See D-writer-holds-one-sink.

**Then the controller drove the operations for real.** Create three sessions, close each,
list, resume, widen, fork, delete, and read a missing file twice. Two faults came out that
190 passing tests did not cover.

| Fault | Why no test saw it |
| --- | --- |
| A resume after a close left `Closed` in the middle of the file | Every test closed a session or resumed one. None did both to one file. |
| An io error said `No such file or directory` and named no path | Every test knew which file it had just created. |

A `Reopened` record now states the reopen, and the invariant is a test: a `Closed` record is
followed by nothing, or by exactly one `Reopened` record. Every io error names its path. See
D-reopen-stated-on-disk.

The lesson repeats with a new face. **A test proves a case. A run proves the product.** The
first version of this lesson came from sprint 1, where 222 tests passed while the product
was unusable.

### A flaky test, and the vacuous family behind it

One rho-core test failed one run in twenty. The hunt took 20 runs to reproduce and one
reading of `tracing` to explain: with no global subscriber, the current level filter is
`OFF`, so a `warn!` takes its fast path and never reaches a thread-local capture. Which
test ran first decided the outcome.

The fix installs one global subscriber per test binary, writing into a thread-local buffer.
25 clean runs of `rho-core`, and 12 of `rho-config`.

The important part is not the flake. `a_resolved_credential_never_reaches_a_log` asserts
that a secret is **absent** from a log. A broken capture makes it pass against an
implementation that prints the credential in full. So every capture helper now emits a probe
line and requires it back, before any absence means anything. See decision D-log-capture-proves-itself.

### The id migration, and the flaky harness it uncovered

The owner asked for one thing: stop the id clashes. Numeric ids need a counter, a counter
needs one allocator, and two worktrees each take the next number and are both right until
they merge. So an artifact is now named by a timestamp and a slug, and no counter exists.
See `D-slug-ids`.

| Change | Count |
| --- | --- |
| Files moved to the timestamp scheme | 21 |
| Decisions split from one file into their own | 62 |
| References rewritten | 1208 across 139 files |
| Duplicate feature rows found and merged | 2 |

Two things are worth naming. The migration ran first in a throwaway worktree, and the diff
was read there before the real tree changed. And the new guard, `bench/check-ids.py`, was
broken four ways on purpose: a numeric id, a reference to nothing, a duplicate slug, and a
file named the old way. It caught all four.

The slug rewrite also found a doc defect that a number had hidden. Two rows in the tier-2
table restated two rows above them, so the catalogue counted 118 features where 116 exist.
A number can repeat quietly. A slug collides, and the guard refuses it.

**Then the repeated runs found a flaky test that predates this sprint.** The provider
contract suite failed about three runs in eight, after 33 seconds, at the point where the
stream starts. The fault was in `rho-provider-testkit`, not in any provider: the staged
server accepted one connection, served it, and ended, so a retry waited in the backlog for
the client timeout. With the one-shot accept restored on purpose, 4 of 12 runs failed. With
the fix, 0 of 10. See `D-staged-server-serves-every-connection`.

That crate is what a third party uses to prove its own provider conforms. A harness that
fails three runs in eight teaches an author to distrust the suite, and a distrusted suite
gets skipped.

## Sprint 3, the terminal interface

`workflow-sprint-3.yaml` drives this sprint. The goal is an interface a critic ranks at or
above pi, codex, claude code, and jcode, in a binary that stays the fastest of them.

The sprint has one rule that the earlier sprints did not. **A critic panel must agree.** Four
critics judge beauty, usability, performance, and accessibility. None of them writes the
code. The stage ends only when every critic returns `top-peak` in the same round, and a
finding accepted as a known limit needs a reason the critic accepts.

| Stage | Attempt | Role | Result | Notes |
| --- | --- | --- | --- | --- |
| U0 | 1 | scout | running | Three scouts read prior art in parallel: pi and jcode, codex and claude code, and Makit. |

The owner named the features: a large paste collapses to one line, a working state animates,
a session and a turn and a tool call and a thinking block each report an elapsed time, a
concise mode collapses a tool call and a thinking block, an attachment pastes, shortcuts and
a guide are discoverable, slash commands list themselves, the header stuns a critic, and a
third party extends the interface.

Each one is judged twice: once by a critic for beauty, and once by a benchmark for cost. An
interface that costs frames or memory is not a win here, because speed and memory are the
reason rho exists.

### Sprint 3 stage results

| Stage | Attempt | Role | Result | Notes |
| --- | --- | --- | --- | --- |
| U0 | 1 | scout | fail | Two scouts lost: one overflowed its context on whole web pages, one hit a connection error. |
| U0 | 2 | controller | pass | Read the sources with `curl` and a filter. Every claim carries a line number. |
| U1 | 1 | designer | pass | Ten frames, exact at 100, 80, and 40 columns. Twelve contrast pairs, all AA. |
| U2 | 1 | architect | pass | The plugin surface, and its panic claim narrowed to what the profile allows. |
| U2b | 1 | architect | lost | The agent vanished with no report and no file. |
| U2b | 2 | architect | pass | The experience spec, written section by section so a loss keeps the work. |
| U3a | 1 | tester | pass | 29 red tests for the ladder and the paste. |
| U3b | 1 | tester | pass | 19 red tests for motion, the theme, concise mode, and discovery. |
| U3c | 1 | controller | pass | The ten frame tests belonged to neither brief. The controller wrote them. |

### What the controller caught in sprint 3

**A wrong attribution.** A scout credited the tool glyphs to pi. They are rho's own, in
`render.rs`. `docs/tui.md` states no glyph set at all. A plausible memory of another tool's
interface is the error that survives a review, so every claim in `docs/tui-prior-art.md` now
carries a line number.

**A byte length used as a width.** The controller's first check of the design frames reported
every line as wrong. The check was wrong, not the frames: a box character is three bytes and
one column. The frame test now measures display width, and it rejects a wide or combining
character, because such a character makes a hand-drawn frame disagree with the terminal in a
way a diff cannot show.

**A claim the release profile forbids.** The plugin spec claimed a panicking view leaves the
session alive. The workspace release profile sets `panic = "abort"`, so a panic aborts the
process before a thread can unwind. The claim now holds only under a profile that unwinds,
and the spec states that a linked view is trusted not to panic. The alternative was to
reopen a measured footprint decision, and a spec does not reverse a measurement.

**A gap in the controller's own split.** Two testers covered six topics, and the ten frame
tests belonged to neither. That is a controller error, not a subagent error, and it is why
the review gate asks which rendered states have no test.

**Contrast verified rather than trusted.** All twelve text-on-background pairs were
re-derived under the WCAG formula. Every claim matches to two decimals, and the tightest pair
is 4.51 to one against a floor of 4.5.

The red suite stands at 48 failing tests, each on an unimplemented body, with 604 tests
passing across the workspace.

### U4, and what a fuzz found that a suite could not

| Stage | Attempt | Role | Result | Notes |
| --- | --- | --- | --- | --- |
| U4a | 1 | developer | pass | The ladder and concise mode. 23 tests green. |
| U4b | 1 | developer | pass | Motion, the theme, and the bindings. 15 tests green. |
| U4a-verify | 1 | controller | pass | 10.8 million values fuzzed against six invariants. |
| U4b-verify | 1 | controller | pass | The sweep proved deterministic, periodic, bounded, and visible. |

A suite of 23 tests can pass on a chain of special cases. So the controller fuzzed the
ladder over every millisecond from zero to three hours, then a sparse sweep to forty days,
which is 10835518 values, against six invariants:

1. No tier prints a full unit in a lower field, so no `60s` and no `60m`.
2. The decimal tier never prints a rounded `.0s`.
3. A tenths field is only ever 1 to 9.
4. No label exceeds the seven-column slot. The widest is exactly seven.
5. The label never goes backwards as the span grows.
6. An open span and a negative span are unrepresentable.

All six hold. So the ladder is right by construction, and not by a lookup of the values the
suite happens to test.

The sweep got the same treatment: determinism per tick, equality across a period boundary,
a band that lights at most nine columns of the eleven its half width allows, a weight
inside zero to one, and a peak that reaches full strength so the motion is visible at all.

**The developer found something the test could not see, and said so.** It broke the sweep by
reading a clock, and a coarse 100 millisecond clock still passed, because every read inside
a fast test lands in one bucket. A nanosecond clock failed, and it failed the band-footprint
test rather than the purity test. So `render_never_reads_a_clock` is the weaker detector of
the two, and the footprint test is what actually guards purity. That is worth knowing before
a critic trusts the wrong test.

### Dispatch statistics for this sprint

Eight subagent dispatches so far. Three hit the turn limit, and all three reported the
exact stopping point, so a narrow follow-up finished the work. The pattern is now clear
enough to act on: **a brief with more than about four fix items will not finish.** So a
hardening pass gets split by crate, and each fix names its file and its test.


### What the controller fixed in the red stages

The tester reports were accurate. The gaps were in the specs and in two test bodies.

1. **The widening rule had no function.** `SPEC-sessions` section 8a said a resume refuses to
   widen a permission. No spec signature expressed the comparison, so the test asserted
   only that the header round-trips. That is the `confine` family again: a rule with no
   API cannot be tested. The spec now states `StoredApproval`, `StoredSandbox`, and
   `check_resume_permission`, and four tests cover the refusal, the narrowing, the
   override, and an unknown mode name. An unknown name parses to the strictest mode, so a
   file from a later build can never widen.
2. **Two failure tests accepted any error.** `a_broken_approval_key_stops_the_run` and
   `a_broken_sandbox_key_stops_the_run` matched `Err(_)`. A read fault would have passed
   them. Both now name the error variant.
3. **The import signature was missing.** The tester flagged that `SPEC-sessions` section 9 named
   a crate and no function. The spec now states `import_pi_session`, and states that it
   returns records and writes no file.

The red suite is 74 failing tests across three crates, and 491 tests still pass. Every
failure is an unimplemented body with a message that names stage T5.

### The T1 review, and why it failed

The reviewer found eight real problems in specs that read well. Each one repeats a
family from sprint 1.

| Severity | Problem | Sprint-1 family |
| --- | --- | --- |
| blocker | The redaction call `redact_json_secrets` does not exist in `rho-redact` | `confine` was `todo!()` in a green stage |
| blocker | The `approval` and `sandbox` defaults were unstated, and the tree default is allow-all | `ToolKind::Other` failed open |
| blocker | A resume could widen a permission, because the header records no policy | `Session::new` hid an insecure default |
| major | The reader had no line cap, so a hostile line could exhaust memory | the `bash` reader cost 805 MB |
| major | The pairing rule covered cancel only, not a crash or a truncated tail | a tool turn never emitted `TurnEnd` |
| major | The record cap covered a tool result only | untested public surface |
| major | The environment layer was read twice, through clap and through config | two sources of truth |
| major | A broken `sandbox` key and an unknown file version had no test | untested public surface |

The scratch-crate check is what proved blocker one. A spec that names a function is
not enough. The reviewer pasted every signature into a crate under `/tmp` and ran
`cargo check`, and the compiler answered `E0425`.

The owner then chose the approval model. rho gains an Ask policy, and it defaults to
Ask wherever a human or a client can answer. It defaults to read-only where nobody can
answer. That is decision D-approval-default-ask, and stages T12 and T13 build it.

One more claim died here. `docs/features.md` F-tool-approval-gate said the TUI wires a confirmation
prompt to the approval gate. `crates/rho-tui/src/` holds no approval code. The row now
states the two policies that exist.

The sprint-2 workflow file also failed to parse. Four list items started with a
backtick, which YAML reserves. A workflow that no parser can read is a defect, so the
file is now quoted and it loads.

### T0 findings, in detail

The code shipped past the catalogue. These rows disagreed with the tree.

| Row | Said | Code shows |
| --- | --- | --- |
| F-background-tasks background tasks | `planned` | `rho-core/src/tasks.rs` and `rho-tools/src/task.rs` |
| F-custom-provider-extension custom provider | `planned` | `rho-provider-testkit`, verified from outside the workspace |
| F-bash-os-sandbox bash sandbox | `sprint-1` | shipped after the sprint-1 retrospective |
| F-skills-filesystem skills | `planned`, owner `rho-core` | `rho-skills`, six test files |
| F-token-and-cost-accounting usage | `planned`, claimed session totals | `rho-core/src/usage.rs`, per turn only |
| F-no-secrets-in-logs redaction | `sprint-1`, owner `rho-config` | `rho-redact` owns redaction |
| MCP, all of it | absent | `rho-mcp`, 1960 lines, four test files |

F-session-statistics stays `planned`. Nothing accumulates a session total today. `Usage::add` exists,
and no caller uses it. The old F-token-and-cost-accounting row claimed the total, so the claim was deleted.

## Process lessons

1. **Run every new regression test against the defect first.** One memory test
   passed against the broken reader, because it asserted the wrong property. A test
   that passes against the broken code is worse than no test.
2. **Grep the tree before believing a green suite.** Three `todo!()` bodies and one
   security boundary survived a stage that reported green. CI now greps.
3. **Split a stage to fit the turn budget.** Six of sixteen dispatches hit a turn
   limit. The verification step belongs in its own dispatch.
4. **A subagent must not run a git command that writes.** One ran `git stash` and
   displaced a parallel agent's files. See D-no-git-writes-by-a-subagent.
5. **Gate a stage on its own crates while a sibling is mid-flight.** A workspace
   gate fails for reasons that have nothing to do with the stage.
6. **Verify a CI guard by breaking the rule on purpose.** Each of the three new
   guards was confirmed to fail on a real violation, not merely to pass today.

## Handover, and a new controller

A new controller session took over the terminal interface work. The previous session was
`01a016a4-48c9-77c7-8203-aac7b6eb36bc`. It ended mid-task, so this section states what the
new controller verified, and what it found open. Every claim below was re-run, not read from
a report.

The tree is `/Users/le/Work/Vibe/rho-altscreen`, on branch `feat/tui-alternate-screen`, at
commit `a72c987`.

### What the gate proves today

The controller ran all five gate commands itself.

| Command | Result |
| --- | --- |
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 warnings |
| `cargo test --workspace --all-features` | 815 pass, 0 fail |
| `cargo build -p rho-cli --no-default-features --features minimal` | ok |
| `bench/check-ids.py`, `bench/check-prose.py` | 0 violations, 0 violations |

`grep -rn 'todo!\|unimplemented!' crates/*/src` finds nothing. The 815 figure in the previous
session's report is true.

### What a live run proves

The controller drove the release binary through a pty, with a terminal emulator reading the
frames. The alternate screen opens once and closes once. Mouse reporting turns on three times
and off three times. The exit code is 0. The help panel draws all 25 key rows. `esc` returns
to the splash and leaves no stale cell. The slash list draws clean.

`TuiError::TooSmall` is correct in the product. A 2-row terminal exits 1 and prints `rho: the
terminal is 2 rows, and rho needs at least 4`. It never opens the alternate screen. A 4-row
terminal runs.

### Open items the new controller found

| Item | Evidence |
| --- | --- |
| The startup warnings are invisible. rho prints them, then opens the alternate screen. | The notices write at byte 5 and byte 320. The alternate screen opens at byte 535. |
| Three `docs/features.md` rows contradict the code. | `F-inline-band` says rho never opens the alternate screen. `F-freeze-upward` claims a machinery that commit `a72c987` deleted. `F-optional-mouse` states the old default. |
| Three tests the spec names do not exist. | `a_terminal_too_short_reports_and_does_not_draw`, `the_composer_keeps_its_ten_row_cap`, and `the_transcript_takes_the_rows_the_composer_leaves`. |
| `plan_screen` is public and has no direct test. | `crates/rho-tui/src/lib.rs:31` exports it. No test calls it. |
| The reasoning spec is untracked. | `../rho-reasoning` holds `20260819-134615-SPEC-reasoning-across-providers.md`, and git does not. |
| Bedrock drops a thinking block from a request. | `crates/rho-provider-bedrock/src/lib.rs:613` is `_ => {}`. No `budget_tokens` field exists. |
| The branch is 29 commits ahead of `origin/main`, and unpushed. | CI has seen none of this work. |

The hidden warning is the one defect here that a test cannot see. It is an ordering rule
between the notices in `rho-cli` and the screen guard in `rho-tui`. One of the hidden lines
says a project skill stays unloaded until the owner trusts it. The owner needs to read that
line.

### What the new controller then closed

| Item | State | Evidence |
| --- | --- | --- |
| The startup warnings are invisible | **fixed** | `d629260`. Startup writes zero bytes to the primary screen. `docs/verification/notices-live.md` |
| Three `docs/features.md` rows contradict the code | **fixed** | `d629260`. The three are now `superseded` rows naming their replacement. Four new rows describe the code |
| Three tests the spec names do not exist | **fixed** | `9599586`. `crates/rho-tui/tests/layout.rs`, ten tests |
| `plan_screen` is public and has no direct test | **fixed** | `9599586`. Four deliberate breaks, and the one that tripped nothing is recorded below |
| The reasoning spec is untracked | **fixed** | `e08b190` in `../rho-reasoning`. 1668 lines, labelled `wip`, unreviewed |
| Bedrock drops a thinking block from a request | **open** | `crates/rho-provider-bedrock/src/lib.rs:613` is still `_ => {}` |
| The branch is unpushed, and CI has seen none of it | **fixed** | Pushed. Pull request 1. All ten CI jobs pass, macOS included |

The test count went from 815 to 838.

Two defects were found after the handover, and neither by a test that existed:

**A review found the notice defect again, at a narrower size.** Every render test used width
100. The notice wrap width reached zero at width 24 or less, so the whole message vanished and
only the label drew. The reviewer put the bound at 21, and measuring put it at 24. The estimate
was optimistic, and the measurement decided.

**The first CI run failed on macOS, and the failure was a false alarm.**
`a_command_child_inherits_only_the_allowlist` probed `SHELL`. On macOS `/bin/sh` fills `SHELL`
from the password database, so the child printed the parent's value with no leak at all. For
`SHELL` a real leak cannot be told apart from the invention, so the probe could never prove
anything. It passed on a developer machine only because `TERM` was set there and came first.

### The lessons this handover adds

7. **A deleted feature leaves a false row behind.** Commit `a72c987` removed the freeze
   machinery and left `F-freeze-upward` claiming it shipped. Step 13 is not paperwork. A row
   that outlives its code misleads the next reader.
8. **A test that never varies one input has not tested that input.** Every render test used
   width 100, and a notice lost its whole message below width 25. Sweep the dimension, do not
   sample it once.
9. **Pick the break that the guard must catch, not the break that is easy.** Deleting the
   panel floor tripped nothing, because the test asked for a height where the floor decides
   nothing. Find the input band where the rule binds, then break it there.
10. **A probe a shell can invent is not a probe.** A credential leak test read `SHELL`, which
   macOS `/bin/sh` fills from the password database. The test now measures a cleared child
   first and refuses to run if the probe is already present.
11. **Building a feature profile does not compile its tests.** A `#[cfg]` on a helper left its
   test module behind, and `cargo build --features minimal` stayed green while the minimal test
   build broke. The gate and CI now run `--no-run` on that profile.

## Aug 19, 2026 — Phase 3 (tables + inline styling), BLOCKED

Inline bold/italic/code delivered (`a0c40af`), all markers gone, **colour required** (not optional modifiers). Rendering passes 885 tests. User requested tables.

**BLOCKER:** Table cell alignment measurement fails. Test expects width 9 ("`crate`" column), gets 27 (entire line). Root cause: `measure_cell()` must strip alignment markers (`:-` → padding) BEFORE measuring width, else padding overshoots. Commit not ready: tables.rs tests written but hanging on assertion. **Next: debug cell width under ratatui's counted-glyph semantics before shipping phase 2.**

