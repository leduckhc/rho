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
