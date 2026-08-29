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
--- progress at 100 columns: 2447 bytes, 2 task rows
    'task for i in 1 2 3 4 5; do echo "RHO_PROGRESS {\\"… done · 100% 5/5 compiling'
    'task for i in 1 2 3 4 5; do echo "RHO_PROGRESS {\\"… done · 100% 5/5 compiling'
PASS  progress at 100 columns: the run drew a task row — 2 rows
PASS  progress at 100 columns: every row is inside the width — widest 77
PASS  progress at 100 columns: no OSC payload on the wire
PASS  progress at 100 columns: no bell on the wire
PASS  progress at 100 columns: no bare clear-screen payload
PASS  progress at 100 columns: the percent draws
PASS  progress at 100 columns: the separator draws

--- quiet at 100 columns: 1497 bytes, 2 task rows
    'task sleep 0.2; echo working done'
    'task sleep 0.2; echo working done'
PASS  quiet at 100 columns: the run drew a task row — 2 rows
PASS  quiet at 100 columns: every row is inside the width — widest 33
PASS  quiet at 100 columns: no OSC payload on the wire
PASS  quiet at 100 columns: no bell on the wire
PASS  quiet at 100 columns: no bare clear-screen payload
PASS  quiet at 100 columns: no separator with no progress

--- hostile at 100 columns: 1870 bytes, 2 task rows
    'task printf \'RHO_PROGRESS {"percent": 50, "message… done · 50% wiped'
    'task printf \'RHO_PROGRESS {"percent": 50, "message… done · 50% wiped'
PASS  hostile at 100 columns: the run drew a task row — 2 rows
PASS  hostile at 100 columns: every row is inside the width — widest 68
PASS  hostile at 100 columns: no OSC payload on the wire
PASS  hostile at 100 columns: no bell on the wire
PASS  hostile at 100 columns: no bare clear-screen payload

--- raw at 100 columns: 1869 bytes, 4 task rows
    'task raw running · wiped pass 1'
    'task long running · 50% yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy…'
    'task raw running · wiped pass 2'
    'task long running · 50% yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy…'
PASS  raw at 100 columns: the run drew a task row — 4 rows
PASS  raw at 100 columns: every row is inside the width — widest 84
PASS  raw at 100 columns: no OSC payload on the wire
PASS  raw at 100 columns: no bell on the wire
PASS  raw at 100 columns: no bare clear-screen payload

--- missing at 100 columns: 1819 bytes, 2 task rows
    'task cat /nope/nope/nope failed (1)'
    'task cat /nope/nope/nope failed (1)'
PASS  missing at 100 columns: the run drew a task row — 2 rows
PASS  missing at 100 columns: every row is inside the width — widest 35
PASS  missing at 100 columns: no OSC payload on the wire
PASS  missing at 100 columns: no bell on the wire
PASS  missing at 100 columns: no bare clear-screen payload
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
PASS  denied at 100 columns: the row says it failed
PASS  denied at 100 columns: the failed row draws in the error role

--- progress at 32 columns: 1049 bytes, 2 task rows
    'task for i in 1 2 … done'
    'task for i in 1 2 … done'
PASS  progress at 32 columns: the run drew a task row — 2 rows
PASS  progress at 32 columns: every row is inside the width — widest 24
PASS  progress at 32 columns: no OSC payload on the wire
PASS  progress at 32 columns: no bell on the wire
PASS  progress at 32 columns: no bare clear-screen payload
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

FAILED CHECKS 0
```

## The guard trips when the filter goes

Step 12 asks for proof that the guard catches the defect. The row filter was removed on
purpose, `let progress_text = progress.to_string();`, and the drive was run again:

```text
--- raw at 100 columns: 1901 bytes, 4 task rows
    'task raw running · [2J[H]0;pwnedwiped pass 1'
    'task long running · 50% yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy…'
    'task raw running · [2J[H]0;pwnedwiped pass 2'
    'task long running · 50% yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy…'
FAIL  raw at 100 columns: no OSC payload on the wire
FAIL  raw at 100 columns: no bare clear-screen payload
FAILED CHECKS 2
```

The terminal received `[2J[H]0;pwned` as text. The file was then copied back, and the drive
reported `FAILED CHECKS 0` again. The good file was copied to `/tmp` first, and `git checkout`
was never used, because it would throw away the whole change.

## What is not proved here

- **No task span.** The duration slot is reserved and blank, because nothing settles a task
  duration yet. The slot was proved with a written span in
  `a_task_row_keeps_the_progress_out_of_the_duration_slot`, not on a real terminal.
- **No provider ran.** This change draws a row from events, so no provider is involved. The
  release binary was built, and `cargo build --release -p rho-cli` is in the gate below.
- **The row text in the output above** comes from a second render of the same state through a
  test backend, because reading a row back out of a live terminal needs a screen scraper. The
  byte-level claims come from the live stream.
