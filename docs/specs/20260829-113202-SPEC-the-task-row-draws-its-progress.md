# SPEC-the-task-row-draws-its-progress — the fifth dead switch, and a guard for its class

Status: delivered. Driven for real; see `docs/verification/task-row-progress.md`.
Owning crate: `rho-tui`
Features: F-task-progress-row, F-background-tasks, F-duration-slot
Decisions this spec implements: D-progress-follows-the-state-and-never-moves-it,
D-a-row-pattern-names-every-field.

## 1. The problem

A background task reports progress. The reducer folds it into `Row::Task.progress`. The
renderer throws it away:

```rust
Row::Task { command, state: task, .. } => { ... }
```

So `cargo build` runs for four minutes and the row says `running` the whole time. The user
learns nothing.

`SPEC-wire-the-dead-switches` found this defect and named it the fifth of five dead
switches. It put the row out of scope on purpose, because the fix needs a row layout
decision that spec did not make. This spec makes the decision and draws the row.

## 1a. Why no guard saw it

`bench/check-dead-surface.py` finds a public function that no code calls. This is a field
that a reader never reads, and the dead-surface guard says so in its own header. Three
mechanisms hold this class instead:

1. **The compiler.** The row pattern names every field of `Row::Task`, with no `..`, so a
   new field fails the build until somebody decides whether the row draws it.
2. **A source guard test.** `..` compiles, so a contributor could put it back. One test
   reads the renderer source and fails on a `Row::Task` pattern that ends in `..`.
3. **An invariant test, not an example test.** The row test asserts a bound over many
   inputs: any progress string, of any length and any bytes, leaves the row inside the
   width with no control character and with the duration slot intact.

## 2. The contract

### The row grammar

```
task <command> <state> · <progress>                                      <duration>
```

Left to right:

- `task `, then the command, then one space, then the state word.
- The progress cell: one space, a `·` separator, one space, then the progress text. The
  whole cell is absent when the progress is empty.
- The duration slot: seven columns, right aligned, from `F-duration-slot`. It is blank
  until a task span is written.

### The rank under pressure

The state word draws first, and it is bounded by the text columns less the label. The command
is cut with a marked ellipsis, and it takes at most half the text columns while a progress
draws. The text columns are what remains after the scroll rail, the duration slot, and the gap
beside it. The progress is dropped whole when the command cannot keep
`TASK_COMMAND_MIN_COLUMNS`. A command that can only draw as a lone ellipsis goes too, and the
space that carried it goes with it.

Both command rules came from the live drive, not from a test. A three hundred character
command drew alone and pushed the state word off the right edge. A greedy bounded command
left the progress four columns, which read `100\u{2026}`.

### The data model bound

`Row::Task.progress` is short. The reducer keeps at most `PROGRESS_SUMMARY_COLUMNS`
display columns of it, and it is sanitised. `Row` is public, so another frontend reads the
same field and may trust the same bound.

```rust
/// The most display columns a stored task progress summary keeps.
const PROGRESS_SUMMARY_COLUMNS: usize = 64;
```

### The row respects the rail

A row that carries a right-aligned slot justifies to the **measure**, which is the frame width
less `RAIL_COLUMN`. The scroll rail draws over the last column of the transcript whenever it
overflows, so a row that used the whole width lost the last character of its duration. The
tool row had the same defect and it was live, because a tool row settles a real duration. Both
rows are fixed here. The design fixtures `100-idle.txt` and `100-tool-run.txt` moved one column.

### A duration is whole or absent

At a pathological width the row text and the slot together exceed the row. The row then drops
the slot rather than draw a cut duration, because `1m 12s` clipped to `1m 12` reads as a
different span. One helper, `justify_slot`, holds the rule for both rows.

### The renderer

```rust
/// The columns a task row keeps between its text and the duration slot.
const TASK_GAP_COLUMNS: usize = 1;

/// The narrowest progress cell worth drawing.
const TASK_PROGRESS_MIN_COLUMNS: usize = 4;

/// The narrowest command a row keeps before it drops the progress instead.
const TASK_COMMAND_MIN_COLUMNS: usize = 8;

/// The share of the text columns a command may take while a progress draws. Two is half.
const TASK_COMMAND_SHARE: usize = 2;

/// The label, the cut command, and the state word.
fn task_head(command: &str, state_text: &str, command_columns: usize) -> String;

/// The whole text of a task row, left of the duration slot.
fn task_row_text(command: &str, task_state: &str, progress: &str, width: usize) -> String;
```

Every part is sanitised in `task_row_text`, and not only in the reducer, because `Row` is
public and a frontend can build a task row itself. Text that does not fit is cut by
`fit_to_width`, which marks the cut with an ellipsis.

### What the row forbids

- The progress never reaches the duration slot.
- The progress never carries a control character or an escape sequence to the terminal.
- The progress never makes the row wider or taller. One task row is one line.
- An empty progress never draws a separator, and never a trailing gap.

## 3. Test cases

| Test | What it proves |
| --- | --- |
| `a_task_row_draws_the_progress_the_reducer_stored` | The percent, the counts, and the message all reach the screen. |
| `a_task_row_with_no_progress_draws_no_separator` | An empty progress leaves the row exactly as it was. |
| `a_task_row_keeps_the_progress_out_of_the_duration_slot` | A long progress cannot take the seven right columns, and a span still draws there. |
| `any_progress_string_stays_inside_the_task_row` | The invariant, over many lengths and hostile bytes. The row is exactly the width. It holds no control character and no escape. |
| `a_narrow_task_row_drops_the_progress_whole` | Below the minimum cell the row draws no separator and no bare ellipsis. |
| `a_stored_task_progress_summary_is_bounded` | The reducer keeps at most 64 columns, however much the task sends. |
| `the_task_row_pattern_names_every_field` | The source guard: the renderer's `Row::Task` pattern holds no `..`. |
| `a_failed_task_row_draws_in_the_error_role` | The `failed` flag drives the row colour, which its own doc comment promised and no code read. |
| `a_long_command_never_pushes_the_state_or_the_progress_off_the_row` | The rank and the half share. The state word and the whole progress survive a three hundred character command. |
| `a_task_row_keeps_its_whole_duration_beside_the_scroll_rail` | An overflowing transcript cannot take the last character of a task duration. |
| `a_tool_row_keeps_its_whole_duration_beside_the_scroll_rail` | The same, for the sibling row, which had the defect live. |
| `a_hostile_state_word_cannot_reach_the_duration_slot` | The state word is bounded too, so no field can take the slot. |
| `a_task_row_filters_a_command_a_frontend_built` | The row filters the command itself, and does not trust the reducer. |
| `a_task_row_drops_a_command_it_can_only_draw_as_an_ellipsis` | A one-column command goes whole, with its space. |
| `a_command_that_filters_to_nothing_leaves_no_gap` | A command of escape bytes alone leaves no double space. |
| `the_command_takes_at_most_half_the_row_beside_a_progress` | The share is exact, so the divisor cannot change in silence. |
| `a_random_task_row_never_leaves_its_bounds` | Two thousand generated rows, over all three fields and the width, keep the slot bound and the filter. |
| `a_row_drops_a_duration_it_cannot_draw_whole` | Every width from 10 to 30, on both rows: the duration is whole or absent, never a fragment. |
| `a_progress_and_a_duration_never_touch` | The gap column is blank while a full progress and a real duration both draw. |

## 3a. What the review changed

The contract went to four reviewers with one lens each, and to `codex review` as an
independent second opinion. Six findings changed the code, and every one is now a test:

| Finding | Who found it | The change |
| --- | --- | --- |
| The scroll rail took the last column of a duration, on this row and on the tool row | layout review | Both rows justify to the measure. Two design fixtures moved. |
| An unbounded state word could take the reserved slot | codex, security, layout | The state is bounded like every other field. |
| The row's filter on the command was never exercised by a test | codex, tests | A raw-command test with a filter oracle. |
| A one-column command drew as a lone ellipsis | layout | The command is dropped whole. |
| The half share was free: a change from two to three passed the suite | tests | The share is asserted exactly. |
| The stored bound was one-sided: 64 could shrink to 7 in silence | tests | The kept width is asserted exactly, and a character bound too. |
| The `..` guard claimed more than a compiler can hold | codex | The claim is narrowed, and the field list now comes from the enum. |
| The live check was a three-literal canary | security | It now finds any stripped CSI or OSC payload, and any C1 character. |
| A narrow row drew a **cut** duration, `1m 12`, which reads as a different span | codex, second pass | Both rows drop the slot instead, through `justify_slot`. |
| The gap column was pinned by three tests about other rules, never by its own purpose | tests | Every bound asserts a clear column before the slot, and one test states the rule. |

## 4. Out of scope

- **The missing bridge.** Nothing subscribes to `TaskRegistry::subscribe` in shipped rho,
  and `ToolContext::agent_events` lives for one tool call while a background task outlives
  the turn. So no task row can appear in a real session today, whatever it draws. That is a
  sixth dead switch of the same family. The fix needs a session-lifetime event path in
  `rho-core`, a call site in `rho-cli`, and a frontend stream that outlives one prompt. It is
  three crates and a contract, so it is not this lane. `bench/tui_task_row_drive.py` supplies
  the bridge inside the harness, which is how this row was driven at all. See
  `docs/verification/task-row-progress.md`.
- **A task span.** The row reserves the duration slot and draws whatever the state holds.
  Nothing settles a duration for a task row yet, so the slot is blank in production today.
  Writing the span belongs to `F-duration-ladder`.
- **`Row::Agent`.** It matches `{ name, outcome, .. }` and hides `depth`, `turns`, `cost`,
  `finished`, and `failed`. The `cost` summary is computed and never drawn, which is the
  same defect as this one. It needs its own layout decision, and this lane reports it. See
  `D-a-row-pattern-names-every-field`.
- **A progress bar.** `TaskProgress` has three optional fields, and a task that reports no
  percent could draw no bar.
- **A task list panel.** Rows only.
- **Colour for the progress text.** The row takes one style.
