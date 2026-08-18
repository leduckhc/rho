# D-a-rail-only-when-it-overflows — The scroll rail appears only when rows are hidden

**Question (owner):** where does the scroll position show on screen?

**Decision:** One column, at the right edge of the transcript, drawn only when the
transcript is taller than its window. The track is `┃` in the `muted` role. The thumb is `█`
in the `text` role. The rail carries no arrow heads and no border. A transcript that fits
draws no rail at all, so the resting frame keeps every column it has today.

A second signal states what the rail cannot. While `scroll_rows` is above zero, a follow
banner replaces the header rule, and it counts the hidden lines.

**Reason:** A permanent rail spends a column on a fact that is usually false. The resting
frame is the frame a user sees most. The rail is also the only place the position shows, and
a position with no count answers the wrong question. So the banner carries the count and the
key that returns to the latest row.

**Rules out:** A rail that always draws. Arrow heads. A rail wider than one column. A rail
inside the composer box or the footer. A banner that stays while the view follows.
