# Verification: bounded teardown in the pty benchmark harness

This page records real commands and their real output. Date: 2026-08-18.

## The defect

`bench/tui_first_frame.py` and `bench/tui_smoke.py` ended each run with
`os.waitpid(pid, 0)`. That call has no bound. It held one agent session open for 30
minutes.

The evidence at the time:

```
  PID  PPID STAT     ELAPSED COMMAND
22680 56237 Ss         30:08 /bin/bash -c ... python3 bench/tui_first_frame.py
22683 22680 S          30:08 python3 bench/tui_first_frame.py
22685 22683 ?Es        30:08 (rho)
```

State `?Es` means the kernel is ending the process and cannot finish. A `sample` of the
harness showed one frame at the top of the stack:

```
Sort by top of stack, same collapsed (when >= 5):
        __wait4  (in libsystem_kernel.dylib)        885
```

## The cause

The harness stops reading the pty master as soon as it has its sample. The buffer then
fills, and the child blocks inside a write to its own terminal. `SIGKILL` cannot finish
while that write sits in the kernel. So the order of teardown decides the result.

Three runs for each order, against `target/release/rho`:

```
close master, then SIGKILL             -> 13ms, 13ms, 13ms
SIGKILL, master open, no drain         -> 11ms, timeout(>8s), timeout(>8s)
SIGKILL, master open, keep draining    -> 0ms, 0ms, 0ms
```

The middle row is the shipped defect. It is not deterministic, which is why the harness
survived earlier use.

## The fix

`bench/ptyharness.py` adds `reap`, `close_pty`, and `teardown`. `teardown` closes the
master first, kills second, and polls with `os.WNOHANG` until a deadline. Both harnesses
call `teardown`. Neither one waits without a bound now.

The old shape never closed the master at all. That leak is measured:

```
old shape (master never closed)    open fds: 4 -> 16   leaked: 12
fixed (teardown closes)            open fds: 16 -> 16   leaked: 0
```

Twelve runs, one leaked descriptor per run, and none after the fix.

## Three shapes, and which number belongs to which

A reviewer found that an earlier draft of this page put the 60.4 second figure against the
shipped defect. That was wrong, so the labels are now exact:

| Code shape | Result over 12 runs |
| --- | --- |
| Shipped: no deadline, kill then close | Hung on the first wedged run. 30 minutes, then a manual kill |
| Half fix: deadline added, kill then close | 60.4 s total, and 12 deadline warnings |
| Fixed: deadline, and close then kill | 0.42 s total, and no warning |

The middle row was measured on the way to the fix. It is worth keeping, because it shows
that a deadline alone only converts a hang into a slow benchmark.

## Proof that each test catches its bug

Six invariants, and each one was broken on purpose. Every mutation was applied to
`bench/ptyharness.py`, and the file was restored from a copy in `/tmp` afterwards, never
with `git checkout`.

**One: the wait has a bound.** The bounded poll was replaced with `os.waitpid(pid, 0)`.
The suite was run bare, with no external timeout:

```
FAIL test_reap_returns_false_and_does_not_hang_on_a_wedged_child: TimeoutError:
test_reap_returns_false_and_does_not_hang_on_a_wedged_child did not finish inside 30s.
A hang is the defect under test, so the watchdog reports it as a failure.
FAIL ...: left 1 child process(es) behind: [18040]
2 failure(s)
```

The watchdog matters. An earlier version of this suite could only stall against this
mutation, so it needed `timeout 20` around it to show anything at all.

**Two: the close comes before the wait.** The two lines in `teardown` were swapped.

```
FAIL test_teardown_closes_the_master_before_it_waits: AssertionError: teardown waited
for the child while the pty master was still open. That is the order that hung
bench/tui_first_frame.py for 30 minutes.
1 failure(s)
```

**Three: the signal cannot be caught.** `SIGKILL` was changed to `SIGTERM`.

```
FAIL test_reap_uses_a_signal_the_child_cannot_catch: AssertionError: reap failed against
a child that ignores SIGTERM. The signal must be SIGKILL, which no process can catch,
block, or ignore.
FAIL ...: left 1 child process(es) behind: [15579]
2 failure(s)
```

**Four: `teardown` reports what `reap` found.** `teardown` was changed to return a fixed
`True`. Both harnesses print their warning behind that value.

```
FAIL test_teardown_reports_a_child_it_could_not_reap: AssertionError: teardown hid a
failed reap behind a True. The warning in both harnesses depends on that value.
1 failure(s)
```

**Five: no signal goes to a pid we no longer own.** The kill was moved ahead of the
ownership probe.

```
FAIL test_reap_accepts_a_child_that_is_already_gone: AssertionError: reap signalled
[(19996, <Signals.SIGKILL: 9>)] for a pid it no longer owns. The operating system reuses
a pid, so that signal can reach an unrelated process.
1 failure(s)
```

**Six: a deadline of zero is refused.** `reap(pid, timeout=0)` cannot honour its own
contract, because a child killed microseconds ago is not reaped yet. Removing the
`ValueError` makes `test_reap_rejects_a_deadline_of_zero` fail.

A first attempt at test two asserted elapsed time instead of order. It passed against the
broken order, because a Python stand-in child reaches only 605 milliseconds and never
wedges. That test was replaced. A test that passes against its own bug is worse than no
test.

## Driving it for real

Twelve runs, twice in a row, with the release binary:

```sh
cargo build --release -p rho-cli
python3 bench/tui_first_frame.py
```

```
runs 12
min    7.5 ms
median 8.1 ms
max    9.5 ms

real	0m0.421s
```

```
runs 12
min    7.5 ms
median 7.9 ms
max    8.7 ms

real	0m0.414s
```

No `rho` process is left behind, which `pgrep -fl 'release/rho'` confirms.

The full interface smoke test also runs to the end, and it exits 0:

```sh
python3 bench/tui_smoke.py
```

That test needs a network and an `openrouter` key, so its screen content is not a
repeatable assertion. On this run the last snapshot held eight rows and the model answered
`banana`. Read that as evidence that teardown does not disturb a real turn, and not as a
fixed expectation.

## A reviewer finding that did not hold

An independent review called the missing `try/finally` in `bench/tui_smoke.py` a blocker,
on the grounds that an exception between the fork and the teardown orphans the `rho` child.
The probe does not support it. An injected `RuntimeError` right after the first snapshot
leaves no orphan in either shape:

```
BEFORE (no finally)    exit=1  orphaned rho: []
AFTER (finally)        exit=1  orphaned rho: []
```

The reason is that the parent exiting closes the master, so the child's next write fails
with `EIO` and the child ends itself. The `try/finally` stays, because it makes teardown
deterministic, keeps the deadline warning, and stops the descriptor leak measured above.
But it is hygiene, and it is not the fix for an orphan defect.

One measurement mistake of my own belongs here too. A first attempt at this probe used
`kill -INT` on a background job, and read "two live children" as "two orphans". A
non-interactive shell sets `SIGINT` to `SIG_IGN` for a background job, so the signal never
arrived and the script simply kept running. Both shapes then took 21 seconds, which is the
full length of the test. The lesson is to make the failure deterministic, and to confirm
the parent is gone before counting an orphan.

## What is not verified here

The numbers above come from a release binary built at 15:13 on 2026-08-18. The working
tree held later, uncommitted changes under `crates/rho-tui/`. So this page does not
restate any sprint figure in `docs/benchmarks.md`, and it makes no claim about a
first-frame trend.
