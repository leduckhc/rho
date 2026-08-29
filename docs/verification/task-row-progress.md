# Verification: the task row draws its progress

Date: 2026-08-29. Machine: macOS 26.5.2, arm64. Toolchain: rustc 1.95.0.
Feature: F-task-progress-row. Spec: `SPEC-the-task-row-draws-its-progress`.

This is step 11 for the task row. No `ratatui` test backend can prove what is below, and one
class of defect is invisible to that backend: `ratatui` skips a zero-width grapheme, so an
escape byte never reaches a cell even when rho forgets to filter, while the visible payload
of the sequence, `[2J`, does reach the screen. So the checks read the raw bytes from the
master side of a pseudo-terminal.

## What ran

```sh
cargo build --release -p rho-cli
cargo build --release --example task_row_drive -p rho-tui
python3 bench/tui_task_row_drive.py
```

`crates/rho-tui/examples/task_row_drive.rs` starts a real command through the real `bash`
tool. The real `TaskRegistry` broadcasts real events, the real reducer folds them, and the
real renderer writes to a real terminal. Every scenario runs its task **twice**, because the
same thing twice has caught two defects in this project.

Six scenarios, two widths:

| Scenario | The command | What it covers |
| --- | --- | --- |
| `progress` | a shell loop printing five `RHO_PROGRESS` lines | a task that really reports progress |
| `quiet` | `sleep 0.2; echo working` | a task that reports none |
| `hostile` | a `RHO_PROGRESS` line whose message decodes to `ESC [2J`, `ESC [H`, and an OSC window retitle | hostile bytes from a real child |
| `raw` | none | a row another frontend built, with raw escape bytes and ten thousand characters |
| `missing` | `cat /nope/nope/nope` | the absent file path, exit 1 |
| `denied` | a file with mode `000` | the denied permission path, exit 126 |

## One defect this step found, before any check ran

**Shipped rho cannot show a task row at all.** Nothing subscribes to
`TaskRegistry::subscribe`, and `ToolContext::agent_events` lives for one tool call while a
background task outlives the turn. So `TaskStart`, `TaskProgressed`, and `TaskEnd` reach no
frontend. The harness subscribes itself, which is the only reason this row could be driven.

This is the same family as the defect this lane fixes, and it is a sixth dead switch. The fix
needs a session-lifetime event path in `rho-core`, a call site in `rho-cli`, and a frontend
stream that outlives one prompt. That is three crates and a contract, so this lane reports it
instead of half-building it. `rho-cli` also belongs to another lane this week.

## Two defects the drive found in this change

Both are layout defects, and no test found either. A test asserts what its author imagined.

1. **A long command pushed the state word off the row.** A model writes the command. The real
   loop command is 118 characters, and the row drew the command alone: no state word, and no
   progress. Fixed by cutting the command with a marked ellipsis.
2. **A bounded but greedy command starved the progress.** With the command cut only by what
   was left, the progress had four columns and read `100…`. Fixed by giving the command at
   most half the columns left of the duration slot.

The tests `a_long_command_never_pushes_the_state_or_the_progress_off_the_row` and
`a_narrow_task_row_drops_the_progress_whole` now pin both rules.

## The real output

Every check passed. The row lines below are the drawn rows, verbatim.

```text
--- progress at 100 columns: 2423 bytes, 2 task rows
    'task for i in 1 2 3 4 5; do echo "RHO_PROGRESS {\\… done · 100% 5/5 compiling'
    'task for i in 1 2 3 4 5; do echo "RHO_PROGRESS {\\… done · 100% 5/5 compiling'
PASS  progress at 100 columns: the run drew a task row — 2 rows
PASS  progress at 100 columns: every row is inside the width — widest 76
PASS  progress at 100 columns: no OSC payload on the wire
PASS  progress at 100 columns: no bell on the wire
PASS  progress at 100 columns: no bare clear-screen payload
PASS  progress at 100 columns: no stripped escape payload of any kind
PASS  progress at 100 columns: no eight-bit control byte
PASS  progress at 100 columns: the percent draws
PASS  progress at 100 columns: the separator draws

--- quiet at 100 columns: 1491 bytes, 2 task rows
    'task sleep 0.2; echo working done'
    'task sleep 0.2; echo working done'
PASS  quiet at 100 columns: the run drew a task row — 2 rows
PASS  quiet at 100 columns: every row is inside the width — widest 33
PASS  quiet at 100 columns: no OSC payload on the wire
PASS  quiet at 100 columns: no bell on the wire
PASS  quiet at 100 columns: no bare clear-screen payload
PASS  quiet at 100 columns: no stripped escape payload of any kind
PASS  quiet at 100 columns: no eight-bit control byte
PASS  quiet at 100 columns: no separator with no progress

--- hostile at 100 columns: 1868 bytes, 2 task rows
    'task printf \'RHO_PROGRESS {"percent": 50, "messag… done · 50% wiped'
    'task printf \'RHO_PROGRESS {"percent": 50, "messag… done · 50% wiped'
PASS  hostile at 100 columns: the run drew a task row — 2 rows
PASS  hostile at 100 columns: every row is inside the width — widest 67
PASS  hostile at 100 columns: no OSC payload on the wire
PASS  hostile at 100 columns: no bell on the wire
PASS  hostile at 100 columns: no bare clear-screen payload
PASS  hostile at 100 columns: no stripped escape payload of any kind
PASS  hostile at 100 columns: no eight-bit control byte

--- raw at 100 columns: 1875 bytes, 4 task rows
    'task raw running · wiped pass 1'
    'task long running · 50% yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy…'
    'task raw running · wiped pass 2'
    'task long running · 50% yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy…'
PASS  raw at 100 columns: the run drew a task row — 4 rows
PASS  raw at 100 columns: every row is inside the width — widest 84
PASS  raw at 100 columns: no OSC payload on the wire
PASS  raw at 100 columns: no bell on the wire
PASS  raw at 100 columns: no bare clear-screen payload
PASS  raw at 100 columns: no stripped escape payload of any kind
PASS  raw at 100 columns: no eight-bit control byte

--- missing at 100 columns: 1819 bytes, 2 task rows
    'task cat /nope/nope/nope failed (1)'
    'task cat /nope/nope/nope failed (1)'
PASS  missing at 100 columns: the run drew a task row — 2 rows
PASS  missing at 100 columns: every row is inside the width — widest 35
PASS  missing at 100 columns: no OSC payload on the wire
PASS  missing at 100 columns: no bell on the wire
PASS  missing at 100 columns: no bare clear-screen payload
PASS  missing at 100 columns: no stripped escape payload of any kind
PASS  missing at 100 columns: no eight-bit control byte
PASS  missing at 100 columns: the row says it failed
PASS  missing at 100 columns: the failed row draws in the error role

--- denied at 100 columns: 1915 bytes, 2 task rows
    "task printf 'echo hi\\n' > blocked.sh; chmod 000 blocked.sh; ./blocked.sh failed (126)"
    "task printf 'echo hi\\n' > blocked.sh; chmod 000 blocked.sh; ./blocked.sh failed (126)"
PASS  denied at 100 columns: the run drew a task row — 2 rows
PASS  denied at 100 columns: every row is inside the width — widest 85
PASS  denied at 100 columns: no OSC payload on the wire
PASS  denied at 100 columns: no bell on the wire
PASS  denied at 100 columns: no bare clear-screen payload
PASS  denied at 100 columns: no stripped escape payload of any kind
PASS  denied at 100 columns: no eight-bit control byte
PASS  denied at 100 columns: the row says it failed
PASS  denied at 100 columns: the failed row draws in the error role

--- progress at 32 columns: 1061 bytes, 2 task rows
    'task for i in 1 2… done'
    'task for i in 1 2… done'
PASS  progress at 32 columns: the run drew a task row — 2 rows
PASS  progress at 32 columns: every row is inside the width — widest 23
PASS  progress at 32 columns: no OSC payload on the wire
PASS  progress at 32 columns: no bell on the wire
PASS  progress at 32 columns: no bare clear-screen payload
PASS  progress at 32 columns: no stripped escape payload of any kind
PASS  progress at 32 columns: no eight-bit control byte
PASS  progress at 32 columns: the progress leaves a narrow row
PASS  progress at 32 columns: the state word survives

--- raw at 32 columns: 738 bytes, 4 task rows
    'task raw running'
    'task long running'
    'task raw running'
    'task long running'
PASS  raw at 32 columns: the run drew a task row — 4 rows
PASS  raw at 32 columns: every row is inside the width — widest 17
PASS  raw at 32 columns: no OSC payload on the wire
PASS  raw at 32 columns: no bell on the wire
PASS  raw at 32 columns: no bare clear-screen payload
PASS  raw at 32 columns: no stripped escape payload of any kind
PASS  raw at 32 columns: no eight-bit control byte

FAILED CHECKS 0
```

## The guard trips when the filter goes

Step 12 asks for proof that the guard catches the defect. The row filter was removed on
purpose, `let progress_text = progress.to_string();`, and the drive was run again:

```text
FAIL  raw at 100 columns: no OSC payload on the wire
FAIL  raw at 100 columns: no bare clear-screen payload
FAIL  raw at 100 columns: no stripped escape payload of any kind — [b'\x1b[18;20H[2J[H]0;', b'8;20H[2J[H]0;pwn', b'20H[2J[H]0;pwned']
FAILED CHECKS 3
```

The terminal received `[2J[H]0;pwned` as text, and the row read
`task raw running · [2J[H]0;pwnedwiped pass 1`. The file was then copied back, and the drive
reported `FAILED CHECKS 0` again. The good file was copied to `/tmp` first, and `git checkout`
was never used, because it would throw away the whole change.

A security review called the first check set a canary, because it matched three literals:
`pwned`, a bell, and `[2J`. It would have missed an OSC 52 clipboard write, another window
title, a bare cursor move, and a DCS string. The check is generic now. It finds any CSI or OSC
payload that no escape byte introduces, and any C1 control character.

## What the review round changed

Four reviewers with one lens each, plus `codex review`, read the commit. Two findings were
defects in shipped code that this change would have built upon:

**The scroll rail took the last column of every duration.** `draw_rail` writes at column
`width - 1` whenever the transcript overflows, and both the task row and the tool row justified
to the frame width rather than to the measure. `RAIL_COLUMN` reserves that column for exactly
this reason. Measured at width 60 with an overflowing transcript:

```text
before: "task build done                                       1m 12│"
after:  "task build done                                      1m 12s "
```

The tool row's case was live, not latent, because a tool row settles a real duration today.
Both rows are fixed, and the design fixtures `100-idle.txt` and `100-tool-run.txt` moved one
column with the fix.

**An unbounded state word could take the reserved slot.** `Row` is public, so the state is
untrusted like the command and the progress, and the row counted its width without ever cutting
it. A five hundred character state filled the row including the slot. The state is bounded now.

Twenty-one mutations were applied one at a time, and each was killed by at least one test. The
two that survived the first round are both closed: a command that filters to an empty string
left a double space, and the half share could change from two to three in silence.

## What is not proved here

- **No task span.** The duration slot is reserved and blank, because nothing settles a task
  duration yet. The slot was proved with a written span in
  `a_task_row_keeps_the_progress_out_of_the_duration_slot`, not on a real terminal.
- **No provider ran.** This change draws a row from events, so no provider is involved. The
  release binary was built, and `cargo build --release -p rho-cli` is in the gate below.
- **The row text in the output above** comes from a second render of the same state through a
  test backend, because reading a row back out of a live terminal needs a screen scraper. The
  byte-level claims come from the live stream.
