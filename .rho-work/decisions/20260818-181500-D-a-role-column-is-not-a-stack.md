# D-a-role-column-is-not-a-stack — A theme role resolves in one mode, and the columns never stack

**Question (controller, after a user reported the contrast):** the role table in
`docs/tui-design.md` section 3 gives every role three values: a 256-colour index, a
16-colour value, and a no-colour modifier set. The renderer applied the 256-colour index
and the no-colour modifiers together. Is that one style or two?

**Decision:** Two, and it is wrong. A role resolves in exactly one mode. In the 256-colour
mode a role is its colour. The modifier column exists for a terminal that has no colour to
carry the meaning with. `style_for` now returns the colour alone. `caution` keeps a bold
weight, because the design gives the approval panel a stronger weight in every mode.

The footer holds two roles on one row. The activity word takes `text`. The key hints take
`muted`.

The composer placeholder takes `muted`, which section 8 of the design already required.

**Reason:** A user reported two faults in one sentence each. The placeholder read as bright
as an answer. The footer read as almost invisible. Both came from the same line of code.

`muted` is 245, measured at 5.19 to 1 against the dark background, which passes WCAG AA.
The renderer painted 245 **and** `DIM`, so the real ratio was lower than the number the
design records, and nobody had measured the result. Two dimmings is not a theme, it is an
accident.

The placeholder was the other half. It drew with the default foreground, so the hint text
competed with the assistant's answer for attention.

A probe against the release binary reads the escape codes rho really writes. An idle frame
now holds no `ESC[2m` at all, and four runs of `38;5;245`:

    idle frame: DIM sequences (ESC[2m) = 0
    idle frame: grey 245 sequences     = 4

    placeholder: ESC[38;5;245;49m  before "Type a prompt"
    activity:    ESC[39;49m        before "ready"
    hints:       ESC[38;5;245;49m  before "enter send"

**How it survived.** No test asserted a style. The frame fixtures compare symbols, not
colours, because `render_rows` reads `cell.symbol()`. So every colour and modifier in the
interface was unpinned. Three tests now pin them, and the first one is the general guard: no
cell may carry grey 245 and `DIM` together.

**Rules out:** Stacking the modifier column on top of the colour column. A contrast ratio
in the design that no test checks. A placeholder that draws in the same role as body text. A
footer that paints one role across two meanings. A style change that ships with no test,
because the frame fixtures cannot see colour.
