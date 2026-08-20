# D-one-hue-means-code — inline code and a code block share one colour

Date: 20260819

## The question

A design review measured rho's new palette and called two pairs collisions. Inline code was index
115 and a heading is 78. Italic was 180, beside warn 179 and caution 173. Are they distinct?

## The decision

**No, they were not, and both moved.**

- Inline code and a code block now share **index 110**, the blue. `Role::MdCode` is deleted.
- Italic is **index 146**, a lavender, outside the amber status family.

## The reason, measured

The numbers are contrast ratios between the two foregrounds, computed from the xterm 256 palette:

| Pair | rgb | Ratio |
| --- | --- | --- |
| heading 78 against inline code 115 | (95,215,135) against (135,215,175) | **1.07x** |
| italic 180 against warn 179 | (215,175,135) against (215,175,95) | **1.02x** |
| italic 180 against caution 173 | (215,175,135) against (215,135,95) | 1.38x |

A ratio of 1.07 is the same colour to the eye. Both were greens of nearly equal luminance, so they
are also identical to a reader with deuteranopia. A ratio of 1.02 is the same again: emphasis inside
an answer read as a warning, on a screen that already carries amber notices.

**The replacements shift hue, not brightness.** Code leans blue and a heading leans green. Italic is
a lavender, so blue is at least as strong as green in it. Two tests hold exactly that, rather than
asserting a colour index that a theme may change.

## Why this deletes a role instead of adding one

`MdCode` and `MdCodeBlock` now mean the same thing and carry the same colour, so one of them is not a
role. Code is code, inline or in a block, and one hue says so. That also answers the review's wider
complaint that a transcript carried eleven colour values: this spends one fewer.

The alternative was jcode's inline chip, a background behind a code span. It reads well, and it
would break `D-a-submitted-prompt-sits-on-a-band`, which spends the only background on the user band.
One background, spent once, is the rule.

## What was left alone, and why

**Bold keeps index 231.** Against a default foreground of `#eeeeee` that is 1.16x, so on a terminal
whose text is already pure white the colour adds nothing and the weight carries it. Against
`#c0c0c0` it is 1.82x. The claim in the code now says that, rather than promising a colour difference
that some terminals cannot show.

**Muted 245 on the band background 236 is 3.82x, below AA.** It is unreachable today, because a user
row is a single run in the band style and carries no muted text. It is recorded rather than fixed,
and a test pins that only the band paints a background so the pairing cannot appear by accident.

## What this rules out

- **No second green.** A new role that leans green must be measured against 78 before it lands.
- **No status hue for a non-status meaning.** Amber is `Warn` and `Caution`. Red is `Error`.
- **No colour claim without a ratio.** A palette change states the measured number, in the table
  above or in `docs/tui-design.md`.
