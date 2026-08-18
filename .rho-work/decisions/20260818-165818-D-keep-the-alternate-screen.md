# D-keep-the-alternate-screen — rho keeps the alternate screen, and adds an escape hatch

**Question (owner):** Codex left the alternate screen, so the transcript lands in the
terminal scrollback. Does rho follow?

**Decision:** No. rho keeps the alternate screen. `ctrl-p` writes the whole transcript to
the terminal scrollback instead. The event loop leaves the alternate screen, prints every
row once, and enters it again.

**Reason:** The fixed composer, the header, and the footer need a screen rho owns. Both
references use that shape. Dropping the alternate screen would mean redrawing the chrome
into the scroll flow, which is a second renderer, not a setting.

The cost of keeping it is real: the terminal's own search and its scrollback hold nothing.
`ctrl-p` pays that cost back in one key, and it needs no clipboard and no dependency.

**Rules out:** A second scrolling-log renderer in this spec. A `/tui` style mode switch.
A partial dump that prints only the visible window. A dump that leaves the terminal in the
alternate screen after an error.
