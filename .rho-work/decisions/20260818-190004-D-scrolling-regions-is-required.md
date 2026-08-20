# D-scrolling-regions-is-required — rho enables the ratatui scrolling-regions feature

**Question (controller):** `Terminal::insert_before` has two implementations. Which one
does rho use?

**Decision:** The scroll-region one. `rho-tui` enables the `scrolling-regions` feature of
`ratatui`, with `cargo add`. Without the feature, `insert_before` clears the band and
repaints it on every insert.

**Reason:** Measured, on a real pseudo-terminal, over thirty single-row inserts with a
redraw after each. The number is the bytes the program wrote:

| Build | Bytes written |
| --- | --- |
| default features | 24004 |
| `scrolling-regions` | 3425 |

Seven times less output for the same result. A streaming turn inserts a row at a time, so
this is the common path, not an edge case. See
`docs/verification/inline-viewport-spike.md` for the commands.

Correctness is equal. Both builds put all thirty rows in the `tmux` history. The feature
sets a scroll region that does not start at line 1, which can drop lines in theory, so the
spike checked a real terminal history rather than a screen model.

**Rules out:** The default `insert_before` path. Any claim about flicker that has no byte
count behind it. A future dependency bump that drops the feature, which the gate must
catch.
