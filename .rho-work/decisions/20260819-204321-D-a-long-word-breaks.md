# D-a-long-word-breaks — a word wider than the row is broken, never cut

Date: 20260819

## The question

`wrap` emitted a word wider than the row whole, and the renderer then clipped it to the frame. What
should happen to a word that cannot fit?

## The decision

**It breaks, by display column, and every character survives.** Both wrappers do it: `wrap` for a
line with no inline markup, and `wrap_runs` for a line with runs.

## The reason

A harsh correctness review found the defect and a live reproduction proved it. Measured before the
fix, driving the real renderer:

| Input | Width | Sent | Drawn |
| --- | --- | --- | --- |
| 30 `x` characters | 10 | 30 | **10** |
| the same 30 inside a code span | 10 | 30 | 30 |
| a 60 glyph CJK paragraph | 80 | 60 | **40** |
| a 72 character URL | 40 | 72 | **40** |

The tail was dropped with no marker, so the reader could not tell that anything had gone.

**A coding agent hits this constantly.** A URL, a path, a hash, a base64 blob, and a stack-trace
line are each one long word. So is a whole CJK or Thai paragraph, because those scripts put no space
between words, which is why a third of the CJK sample vanished.

**The two paths disagreeing is what exposed it.** `wrap_runs` already broke a long word and `wrap`
did not, so the same text survived inside backticks and was cut without them. A shortcut that
changes the answer is not a shortcut.

## What the review also caught in the other wrapper

`wrap_runs` appended the character that tipped a word past the width **before** breaking, so a row
could be one or two columns too wide. A two-column glyph at that edge was then dropped by `put`. The
break now happens before the tipping character, so no row is ever wider than the frame.

## The test that should have caught it

`a_long_line_inside_a_block_still_wraps` claimed to cover "one very long line" and used
`"word ".repeat(60)`: sixty spaced four-letter words, so it never had a word wider than the row. It
kept its name and its many-words case, and the long-word case now lives in `long_words.rs`.

## What this rules out

- **No silent clip, anywhere.** A row is cut at the frame only when a single glyph cannot fit.
- **No hyphenation.** A break is a break, with no inserted character, because an inserted hyphen in a
  URL or a hash would be worse than a visible wrap.
- **No two wrapping rules.** A line with markup and a line without must break a long word the same
  way, and `the_markup_free_path_agrees_with_the_run_path` holds that.
