# D-seven-column-duration-slot — every duration sits in a seven-column slot

**Question.** How wide is the duration slot, and does it change per duration?

**Decision.** Every duration sits in a fixed slot of seven columns. The value is right
aligned. The slot is seven columns everywhere, at every width.

**Reason.** Seven columns is the width of the widest rung, `18m 04s`. A fixed, right
aligned slot means a live tick never reflows the text beside it. A value that grows from
`9.1s` to `2m 41s` moves no character after the slot. Makit found this constraint, and it
is cheap to miss.

**It rules out.** It rules out a slot that sizes to its current value. It rules out a
left-aligned duration. It rules out a per-context slot width, so the header, the footer,
and a tool row all use the same seven columns.
