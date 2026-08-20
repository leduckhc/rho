# D-a-frozen-row-never-repaints — a row leaves the band only when nothing can change it

**Question (controller, after the inline spike):** `Terminal::insert_before` writes a row
into the terminal's scrollback, and rho cannot draw there again. When may a row leave?

**Decision:** Only when the row is final. A final row can never change again, for any
event the reducer may still apply. rho freezes the longest final prefix of the transcript,
oldest first, and it freezes nothing that sits behind a live row.

Order is part of the contract. A row may freeze only when every row before it has frozen.
So the scrollback reads in the same order as the session file.

**Reason:** The terminal owns the pixels once the row is above the band. A duration that
still ticks, a tool status that flips to failed, or a streaming answer that grows would
each need a repaint that cannot happen. Freezing early would leave a lie on screen.

Freezing a prefix, rather than any final row, keeps the order. A finished tool row that
jumped over a running one would report a false sequence of events.

**What it costs.** The fold keys cannot expand a frozen row. So `ctrl-o` and `ctrl-e` work
on the live band only, and the spec that adds them must say so.

**Rules out:** Freezing a row while its turn still runs, unless the row is final by the
rule in `D-row-finality-is-explicit`. Freezing out of order. Any later feature that repaints
a row after it froze, including a fold, a theme change, and a re-wrap on resize.
