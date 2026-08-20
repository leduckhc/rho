# D-markdown-line-level-first — colour a whole row now, colour a word later

Date: 20260819

## The question

The owner asked rho to colour markdown instead of drawing its punctuation. jcode and pi both
do it. How much of it does rho do, and in what order?

## The decision

**Line level first, and inline second.** rho now colours a heading, a fence, a code line, a
quote, a list marker, and a rule. Each of those styles a whole row.

**Inline emphasis waits.** `**bold**` and `` `code` `` are not done, and the punctuation still
shows for them. Phase 2 of `SPEC-tui-markdown` carries them, behind a run-level row contract.

## The reason

The split is not arbitrary. It falls exactly where the current renderer stops being able to
express the answer.

A row is `(String, Style)`, so one style covers a whole row, and `put` writes one style per
row. **An inline style is not expressible at all today.** Making it expressible means changing
the row type to a list of styled runs, which touches 30 producer sites and the frame writer.
That is a contract change, and it deserves its own review.

**The ordering decided the rest.** `wrap_block` measures the text it is handed. Removing `**`
after wrapping would leave every affected row four columns narrower than the wrap assumed, and
a row that wrapped early would keep a hole. Every line-level element avoids that trap, because
its markup sits at the start of a line and can be removed **before** the text is wrapped, with
one uniform style for every row the line produces.

So line level is not a smaller version of inline. It is the part that is correct without a new
contract.

## What the review changed

The contract went to review before either side was written, as step 3 requires. It came back
`REVISE`, and three findings landed in this phase:

1. **Fence state must span the whole message, not the visible window.** A code line whose
   opening fence has scrolled off screen would otherwise be read as prose, and a `#` or a `-`
   inside code would be miscoloured. The scanner reads a whole message.
   `fence_state_spans_the_whole_message_not_the_visible_window` pins it.
2. **A table must degrade to verbatim text.** A model emits tables constantly, and half a
   table styled is worse than none. `a_table_degrades_to_verbatim_text` and
   `an_alignment_row_is_not_mistaken_for_a_rule` pin it.
3. **Six new theme roles was vocabulary pollution.** Trimmed to two, `MdHeading` and
   `MdCodeBlock`. A fence and a quote reuse `Muted`, and a bullet reuses `Accent`. A role now
   exists only when no current role carries its meaning.

## Every rule needs a separator

A coding agent's prose is full of text that looks like markup. Each rule requires a space, so
none of these change:

| Text | Why it stays text |
| --- | --- |
| `#[derive(Debug)]` | a heading needs a space after its hashes |
| `--no-mouse` | a bullet needs a space after its marker |
| `1.2.3` | a numbered item needs a space after the dot |
| `>out.txt` | a quote needs a space after the arrow |
| `|---|` | a rule is only rule characters, and a pipe is not one |

## What this rules out

- **No parser, and no highlighter.** No `pulldown-cmark`, no `syntect`. rho's size and start
  time are features, and jcode's markdown crate is 7140 lines. rho's scanner is under 200.
- **No guessed language.** pi's own comment says auto-detection "can misidentify prose as
  AppleScript ... coloring random English words as keywords". rho colours a code block as one
  colour and guesses nothing.
- **No OSC 8 hyperlink.** A link that hides its target is a phishing surface in a terminal.
- **The markdown subset is closed, and it is not an extension point.** A new element means
  editing the scanner. The review agreed that is acceptable **only** because the subset is a
  frozen internal vocabulary. The open contract is the frame contract, and phase 2 defines it.
- **A user row is never markdown.** rho draws what the user typed.
