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
| T1 | 1 | architect | partial | SPEC-13, SPEC-14, SPEC-15, and ADR-004. Session ops and steering added mid-flight by the owner. |
| T1 | 2 | reviewer | fail | Three blockers and five majors. Every one is a repeat of a sprint-1 defect family. |
| T1b | 1 | architect | pass | Eight findings fixed. SPEC-16 written. Five decisions, D-051 to D-055. |
| T1b | 2 | controller | pass | Controller found one more fail-open: a remembered allow would have covered every `bash` call. |
| T2 | 1 | tester | pass | 29 config tests, all red on `todo!()`. Every SPEC-13 test name exists. |
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

Two more claims died. `SPEC-14` said CI runs the codec tests with `fast-json` on and off,
and `rho-core` had no such feature. The feature now exists, the CI matrix is a T10 item,
and no document claims it works before then. A golden line now pins the codec bytes, so
the two modes are comparable at all.

The last fault was in a type. `Config::approval` was `ApprovalMode`, and the spec said the
default is unset. A type with no unset state forced `Config::load` to invent `read-only`,
which made the owner's `ask` default unreachable. See decision D-057.

### Dispatch statistics for this sprint

Eight subagent dispatches so far. Three hit the turn limit, and all three reported the
exact stopping point, so a narrow follow-up finished the work. The pattern is now clear
enough to act on: **a brief with more than about four fix items will not finish.** So a
hardening pass gets split by crate, and each fix names its file and its test.


### What the controller fixed in the red stages

The tester reports were accurate. The gaps were in the specs and in two test bodies.

1. **The widening rule had no function.** `SPEC-14` section 8a said a resume refuses to
   widen a permission. No spec signature expressed the comparison, so the test asserted
   only that the header round-trips. That is the `confine` family again: a rule with no
   API cannot be tested. The spec now states `StoredApproval`, `StoredSandbox`, and
   `check_resume_permission`, and four tests cover the refusal, the narrowing, the
   override, and an unknown mode name. An unknown name parses to the strictest mode, so a
   file from a later build can never widen.
2. **Two failure tests accepted any error.** `a_broken_approval_key_stops_the_run` and
   `a_broken_sandbox_key_stops_the_run` matched `Err(_)`. A read fault would have passed
   them. Both now name the error variant.
3. **The import signature was missing.** The tester flagged that `SPEC-14` section 9 named
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
answer. That is decision D-051, and stages T12 and T13 build it.

One more claim died here. `docs/features.md` F-29 said the TUI wires a confirmation
prompt to the approval gate. `crates/rho-tui/src/` holds no approval code. The row now
states the two policies that exist.

The sprint-2 workflow file also failed to parse. Four list items started with a
backtick, which YAML reserves. A workflow that no parser can read is a defect, so the
file is now quoted and it loads.

### T0 findings, in detail

The code shipped past the catalogue. These rows disagreed with the tree.

| Row | Said | Code shows |
| --- | --- | --- |
| F-07 background tasks | `planned` | `rho-core/src/tasks.rs` and `rho-tools/src/task.rs` |
| F-15 custom provider | `planned` | `rho-provider-testkit`, verified from outside the workspace |
| F-31 bash sandbox | `sprint-1` | shipped after the sprint-1 retrospective |
| F-45 skills | `planned`, owner `rho-core` | `rho-skills`, six test files |
| F-101 usage | `planned`, claimed session totals | `rho-core/src/usage.rs`, per turn only |
| F-103 redaction | `sprint-1`, owner `rho-config` | `rho-redact` owns redaction |
| MCP, all of it | absent | `rho-mcp`, 1960 lines, four test files |

F-102 stays `planned`. Nothing accumulates a session total today. `Usage::add` exists,
and no caller uses it. The old F-101 row claimed the total, so the claim was deleted.

## Process lessons

1. **Run every new regression test against the defect first.** One memory test
   passed against the broken reader, because it asserted the wrong property. A test
   that passes against the broken code is worse than no test.
2. **Grep the tree before believing a green suite.** Three `todo!()` bodies and one
   security boundary survived a stage that reported green. CI now greps.
3. **Split a stage to fit the turn budget.** Six of sixteen dispatches hit a turn
   limit. The verification step belongs in its own dispatch.
4. **A subagent must not run a git command that writes.** One ran `git stash` and
   displaced a parallel agent's files. See D-015.
5. **Gate a stage on its own crates while a sibling is mid-flight.** A workspace
   gate fails for reasons that have nothing to do with the stage.
6. **Verify a CI guard by breaking the rule on purpose.** Each of the three new
   guards was confirmed to fail on a real violation, not merely to pass today.
