# Verification — the task event bridge

`SPEC-the-task-event-bridge`. Step 11: drive it for real.

This is the lane where a real user finally sees a background task. Every claim below comes
from a command that ran on this machine, and the output is copied, not summarised.

## The question this had to answer

The task row had a reducer, a renderer, a layout decision, and eleven tests. It reached no
user. Nothing in any shipped binary subscribed to the task registry, and the previous lane's
own harness supplied the missing half so that it could draw anything at all.

So the only verification that means anything here is the real binary, a real model, and a
real command. A harness that bridges the gap it is testing proves nothing.

## The build

```sh
cargo build --release -p rho-cli
```

## The drive

`bench/tui_task_bridge_drive.py` runs the real `rho` interface on a pseudo-terminal, reads
the screen through `pyte`, and sends two prompts to a live model. It asks for a background
command that reports a percentage, and then for a second one that fails.

```sh
AWS_REGION=us-east-1 python3 bench/tui_task_bridge_drive.py
```

Provider `bedrock`, model `us.anthropic.claude-haiku-4-5-20251001-v1:0`.

```
PASS  a background task draws a row in the real interface — 'task for i in 1 2 3 4 5 6 7 8; do echo "progress:… running · 12%'
PASS  the row draws the task's progress — 'task for i in 1 2 3 4 5 6 7 8; do echo "progress:… running · 12%'
PASS  the row finishes while rho sits idle at the composer — 'task for i in 1 2 3 4 5 6 7 8; do echo "progress:… done · 96%'
PASS  a second background task draws its own row — 2 rows
PASS  a task that fails draws a failed state, not a finished one — 'task sleep 2; /usr/bin/false failed (1)'
PASS  the first row is not replaced by the second
FAILED CHECKS 0
```

The screen at the end of the run, as a user sees it:

```
ρ rho  ~/.worktrees/rho/fix-the-task-row-nobody-sees · fix/the-task-row-nobody-sees · us.anthropic.c
❯ Run this in the background with the bash tool, and then tell me the task id. Command: for i in 1
  2 3 4 5 6 7 8; do echo "progress: $((i*12))%"; sleep 1; done
  ✓ bash                                                                                         0s
task for i in 1 2 3 4 5 6 7 8; do echo "progress:… done · 96%
The task id is task-1. You can use the task tool to check its progress or wait for it to finish.
❯ Run this one in the background too, with the bash tool, and tell me its id. Command: sleep 2;
  /usr/bin/false
task sleep 2; /usr/bin/false failed (1)
  ✓ bash                                                                                         0s
The task id is task-2. You can use the task tool to check its progress or wait for it to finish.
```

## The same drive, on the branch without the bridge

This is the measurement that matters. The script, the model, and the commands are identical.
Only the binary changes.

```sh
cd /Users/le/.worktrees/rho/fix-draw-the-task-progress && cargo build --release -p rho-cli
RHO_DRIVE_BINARY=.../fix-draw-the-task-progress/target/release/rho \
  AWS_REGION=us-east-1 python3 bench/tui_task_bridge_drive.py
```

```
FAIL  a background task draws a row in the real interface — None
FAIL  the row draws the task's progress — None
FAIL  the row finishes while rho sits idle at the composer — None
FAIL  a second background task draws its own row — 0 rows
FAIL  a task that fails draws a failed state, not a finished one — None
FAILED CHECKS 5
```

Five failures against the branch that drew the row, and none against the branch that
delivers it. The row was never the missing part.

## What the failure path showed

**A failing task draws a failing row.** `sleep 2; /usr/bin/false` draws `failed (1)`, and the
row does not say `done`.

**The same thing twice.** Two tasks in one session draw two rows, and the second does not
replace the first. "Twice" has caught two defects in this project.

**A task that ends while rho is idle still finishes its row.** The turn had already settled
when the eight-second command exited, and the row moved from `running · 12%` to `done · 96%`
with the composer waiting for input. A run-scoped event stream would have lost that, and the
row would have said `running` for the rest of the session. This is the case that made the
bridge session-lifetime rather than run-lifetime.

## The row harness now drives the shipped bridge

`bench/tui_task_row_drive.py` belongs to the previous lane. It used to call
`TaskRegistry::subscribe` itself, because nothing else did. It now reads
`TaskRegistry::session_events`, which is the exact stream `rho-cli` hands the interface.

```sh
cargo build --release --example task_row_drive -p rho-tui
python3 bench/tui_task_row_drive.py
```

```
FAILED CHECKS 0
```

Eight scenarios, each running its task twice, at widths from 32 to 200 columns. Every row
stayed inside its width, and no control byte reached the terminal.

## Three checks that passed against the bug

Written down because two of them were mine, in this document's own harness.

1. The `rho-cli` call-site guard searched the whole file for a literal that its own assertion
   contained, so it passed with the call site deleted.
2. The live drive counted any line holding the word `task` as a row, so the model's sentence
   "Your task id is task-1" satisfied three checks on a build with no bridge.
3. A failure check matched `exit 3` inside a command string rather than the state word, and
   its replacement could never match `failed (1)`, because a trailing word boundary cannot
   follow a bracket.

Each one was found by breaking the implementation and watching the test fail, which is step 7.
None would have been found by reading the code.

## What this does not verify

**The JSONL frontend.** `rho-jsonl` still maps a task event to `None`. A task event family on
that wire binds every client, so it needs its own spec. See section 8 of the spec.

**A duration for a task row.** The slot stays blank, and `F-duration-ladder` owns it.

**A lag in a live session.** The repair is proved by tests that flood the channel past its 256
events. No real session reached that in this drive, because the interface drains the stream on
every frame.
