# D-progress-follows-the-state-and-never-moves-it

Date: 20260829

## The question

A background task reports progress. The reducer stores it in `Row::Task.progress`. The
renderer never reads it, so a long build shows no percentage.

`SPEC-wire-the-dead-switches` left this open on purpose. It needs a row layout decision.
Three questions had no answer: where the progress sits, what an empty progress draws, and
what stops a hostile progress string from taking the row.

## The decision

**The progress is the last text on the row, after the state word.**

```
task cargo build --release running · 42% 6/10 compiling                  1m 12s
```

The order is command, state, progress, then the duration slot. The progress is the part
that changes most, so it goes last. A value that changes cannot move a value that does
not. That is the same rule that `F-duration-slot` states for a duration, so the row keeps
one convention and does not invent a second.

**The duration slot stays reserved.** The row draws the seven-column right-aligned slot
that `F-duration-slot` defines, through the same `duration_slot` call the tool row makes.
No task span is written yet, so the slot draws seven blank columns today. It is reserved
now because progress must never be able to take it. A row that let progress reach the
right edge would reflow every text on the row on the day a span arrives.

**An empty progress draws nothing at all.** No separator, and no gap. The separator
belongs to the progress cell, so the two appear and vanish together.

**The rank under pressure is the state, then the command, then the progress.** The state
word is the status, so the row reserves it first and it always draws. The command names
which task the row is, so it comes next. The progress qualifies the two, so it goes last.

**The command is cut, and it takes at most half the text columns.** A live drive found both
rules. A model writes the command, and a real one was three hundred characters long: the row
drew the command alone, and the state word and the progress were both cut off the right edge.
With the command bounded but greedy, the progress then had four columns and read `100…`,
which says nothing. So the command is cut with a marked ellipsis, and while a progress draws
it keeps at most half the columns left of the duration slot.

**A narrow row drops the progress whole.** The progress cell needs four columns, and the
command needs eight, beside the state and the slot. Below that the row draws no progress and
no separator. A bare ellipsis teaches the reader nothing. Narrower still, the command goes
too, and the space that carried it goes with it, so the row never grows a double space.

**Every field of the row is bounded, and the state word too.** `Row` is public, so the
state is untrusted like the command and the progress. The state takes at most the text
columns less the label, so no field can reach the slot. A review found a five hundred
character state taking the slot, which is the one thing this row exists to protect.

**A row that carries a right-aligned slot justifies to the measure, not the frame width.**
The scroll rail draws over the last column of the transcript whenever it overflows, and
`RAIL_COLUMN` reserves that column always for exactly this reason. The row used the frame
width, so a settled duration read `1m 12│`. The tool row had the same defect, in the same
one word, and it was live rather than latent. Both rows are fixed, and the design fixtures
`100-idle.txt` and `100-tool-run.txt` moved one column with the fix.

**A command that can only draw as an ellipsis is dropped whole.** One column holds the cut
marker and nothing else, which is the bare marker this decision refuses for the progress.

**The progress is bounded twice, and both bounds are security.**

- The **row** cuts the progress to the columns that are left, with `fit_to_width`, so the
  cut is marked and the row cannot reach the duration slot.
- The **reducer** cuts the stored summary to 64 columns. The field says "a short progress
  summary", and `Row` is public, so another frontend reads it too. A bound in the data
  model keeps that promise for every reader.

The text is sanitised in the reducer and again in the row. `Row` is public, so a frontend
can build a task row directly, and the row filters at its own boundary. The tool row
already learnt this: a review found a second construction site that stored a raw name.

## What this rules out

- **No right-aligned progress slot.** One right-hand slot exists, and a duration owns it.
- **No wrapping.** A task row is one line. Progress that does not fit is cut, not moved to
  a second row.
- **No progress before the state.** That would let a percent change push the state word.
- **No unbounded progress in the state.** A reader of `Row::Task` may trust the bound.
- **No colour for the progress.** The row takes one style. `Role::Error` when the task
  failed, and the plain text style otherwise. The `failed` field says in its own doc comment
  that it drives the colour, and no code read it. That is the same defect as `progress`.
- **No unbounded command.** A command that can take the row can hide the status.

## Why not the alternatives

**Progress in the right slot, beside the duration.** It reads well, and it needs a second
fixed slot. Two fixed slots at the right edge leave a narrow terminal no text column, and
the progress is variable text, which is what a fixed slot is bad at.

**Progress on its own row.** A task row would become two rows, and a task lives for
minutes. Ten tasks would take twenty rows of a band that also holds the conversation.

**A percent bar.** A bar needs a percent, and `TaskProgress` has three optional fields. A
task that reports `6/10 compiling` and no percent could draw no bar.

## What the compiler can and cannot hold

The exhaustive destructure forces a **decision** about every new field of `Row::Task`. It
cannot force a **draw**: a contributor can name a new field `field: _` and the build passes.
So the claim stops there, and a reviewer is the only guard for the last step. Codex named this
overclaim in review, and the sentence above is the corrected version.

## What a live drive changed

The first version of this decision went to a real terminal through
`bench/tui_task_row_drive.py`, and the drive changed it twice. The command bound and the
half share are both in the list above because a real command showed the defect. Neither
came from a test. See `docs/verification/task-row-progress.md`.
