# D-text-fills-the-width — the transcript uses the frame, less one rail column

Date: 20260819

## The question

The transcript wrapped at `min(80, width - 10)`. On a 156 column terminal that used half the
screen, and on a 40 column terminal it used 30. The owner asked for the full width, with no margin
either side. How much of the frame can text actually take?

## The decision

**All of it, less one column.** The measure is `width - 1`.

The one column belongs to the scroll rail. There is no margin on the left, and none on the right
beyond that column.

An earlier draft of this reserved one blank column on each side. The owner retracted that, so
there is none.

## The reason for the one column

The scroll rail draws at `width - 1`, as a single-column overlay on the transcript. Text that used
the whole width lost its last character to the rail every time the transcript overflowed. A
transcript that overflows is the normal state of a session, so that is not an edge case.

**Reserving the column only while the rail shows does not work, and the reason is worth writing
down.** The measure decides how many rows the text wraps to. The row count decides whether the
transcript overflows. The overflow would decide the measure. That is a cycle, and a layout that
depends on its own output is a layout that flickers between two states at the boundary. So the
column is reserved always.

## What the old numbers were for

The 80 column cap came from typographic advice about a comfortable reading measure, and the 10
column margin came with it. The advice is sound for prose on a page. It is wrong here for two
reasons the owner named: a coding agent's answer is full of code, paths, and tables that read worse
when folded early, and a terminal that shows 156 columns was asked to show 156.

## What this rules out

- **No reading measure.** rho does not cap a line at a comfortable length. A user who wants a
  narrower measure makes the window narrower.
- **No margin.** Text starts at column 0.
- **No dynamic reservation.** See the cycle above.
- **A divider still spans the frame.** A horizontal rule and the composer rules fill every column,
  including the rail column, because a divider is chrome and the rail sits over the transcript
  only. The markdown rule was capped at the old measure and is now full width.
- **A table row is unaffected.** Its width comes from its columns, and `put` cuts it at the frame
  edge.

## What was regenerated, and checked

Four design fixtures changed: `40-streaming`, `80-streaming`, `100-idle`, and `100-streaming`. The
diff was read before it was staged. Every change is a line holding more words. Every frame keeps
24 rows, and the chrome is untouched. At 40 columns the measure went from 30 to 39, which is the
clearest gain.
