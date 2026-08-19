# D-a-submitted-prompt-sits-on-a-band — one background, spent once

Date: 20260819

## The question

A long transcript gives no quick way to find where each turn began. The owner asked for a
full-width background behind an already-submitted user message, and sent screenshots of Claude Code
and pi doing it. How does that fit a theme that had no background at all?

## The decision

**A submitted prompt draws on a band: a full-width background, and bold text.** Nothing else in the
interface paints a background.

The band is a fourth theme mapping, `role_bg_256`, and one role uses it: `UserBand`.

## The reason

Both references mark the prompt the same way, and it is the cheapest possible scan: the bands are
the turn boundaries. Neither marks the answer, and neither should. A background on the answer as
well would make the whole screen a band and mark nothing.

**The band covers every column.** A band that stops at the last word is a ragged stripe, and reads
as a rendering fault rather than a boundary.

**One mechanism, not two.** The row is deliberately *not* padded to the width by its producer.
`put` fills the tail of a row with the row's own style, and that is what carries the band to the
frame edge. Padding at the producer as well would have worked and would have left the fill in `put`
untested, and untested code is where this project's defects have lived. Removing the fill now fails
two tests.

## Why a background is a fourth mapping, not a field on `RoleStyle`

`RoleStyle` describes the 16-colour and no-colour modes, and **neither can carry a quiet
background.** A 16-colour terminal has no subtle grey, and the no-colour mode has no colour at all.
So a background belongs only to the 256-colour mode, and it gets its own mapping there.

The other two modes mark a prompt differently, and the table says how: bold in 16 colours, reversed
with no colour.

## The weight is intended, in every mode

`style_for` reads modifiers from the 16-colour table whatever the colour depth. So the `bold` set
for `UserBand` applies **beside** the band in 256 colours, not only instead of it.

That was noticed as a side effect in a live run and then chosen on purpose: the words are the user's
own, and weight suits them. `a_submitted_prompt_is_bold_in_every_colour_mode` pins it, so it is a
decision rather than a leak.

## What this rules out

- **No second background, ever, without revisiting this file.** A background is loud and it is
  spent. `only_the_user_band_paints_a_background` fails if another role takes one.
- **No band on the answer, a tool row, or a notice.**
- **No band on the composer draft.** The draft is not submitted yet, and the composer has its own
  rules around it.
- **No blank banded rows above and below.** pi pads its band with a blank row on each side. rho
  bands the message rows only, as Claude Code does, so the separator rows stay clear and the band is
  exactly the message.
