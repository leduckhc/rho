# D-inline-viewport-not-alternate-screen — rho draws in an inline band, and the terminal keeps the transcript

**Question (owner):** "My intuition is that the alternate screen is not needed." Is it?

**Decision:** It is not needed, and rho drops it. `rho-tui` opens
`Viewport::Inline(band)` and never enters the alternate screen. A finished row leaves the
band through `Terminal::insert_before`, which puts it in the terminal's own scrollback.

The band holds three regions: the live rows of the current turn, the composer, and the
footer. The band height is fixed for the life of the `Terminal`, and the layout inside it
moves.

So the terminal owns the transcript, and rho owns only the live band.

**Reason:** The earlier decision claimed the fixed composer needs the alternate screen.
That claim was false, and I wrote it from memory instead of from the documents. The
`ratatui` guide calls the alternate screen a choice for a program that wants "the full
terminal window without disrupting the command line". `Viewport::Inline` exists for the
opposite case, and `Terminal::insert_before` exists to print above the band.

A spike proved every part on a real pseudo-terminal, and `tmux` confirmed the scrollback
holds all thirty frozen rows. See `docs/verification/inline-viewport-spike.md`.

The user's three complaints then answer themselves. The wheel scrolls, because the terminal
scrolls. A drag selects, because the text is ordinary output. A search finds, because the
scrollback holds the words. rho writes no scroll code for any of it.

**What it costs.** An inserted row is immutable, so a row must be final before it freezes.
See `D-a-frozen-row-never-repaints`. The live area is bounded by the band, so a long turn
shows its tail until the rows freeze.

**Rules out:** `EnterAlternateScreen` in `rho-tui`. A scroll offset in `TuiState`. A scroll
rail. A follow banner. A keyboard selection of transcript rows. An OSC 52 copy. A
`ctrl-p` dump to the scrollback, because the rows are already there. A second renderer.
