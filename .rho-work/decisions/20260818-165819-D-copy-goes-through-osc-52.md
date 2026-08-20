# D-copy-goes-through-osc-52 — A copy writes an OSC 52 sequence, and never links a clipboard crate

**Question (controller):** `y` copies a selection. Which clipboard does it write?

**Decision:** The terminal's. rho writes the OSC 52 escape sequence to standard output, and
the terminal puts the text on the system clipboard. rho links no clipboard crate. The key
handler stays pure: it returns `KeyAction::Copy(String)`, and the event loop writes the
sequence.

Two escape hatches cover a terminal that refuses OSC 52. `o` opens the selection in
`$EDITOR`. `ctrl-p` writes the transcript to the scrollback, where the mouse works.

> **Amended on 2026-08-19.** The second hatch is gone, because rho builds no dump. See
> `D-alternate-screen-after-all`. `$EDITOR` remains, and the terminal's own selection still
> works. A user who needs the mouse for a selection passes `--no-mouse`.

**Reason:** OSC 52 works over ssh and inside tmux, because the sequence travels with the
terminal stream. A clipboard crate talks to the local display server, so it fails in the
exact place a terminal agent runs most. It would also add a platform dependency tree to a
crate that draws text.

The sequence carries base64 text, so the payload needs no shell quoting, and no command
runs. A copy therefore starts no process.

**Rules out:** `arboard`, `copypasta`, or any clipboard crate in `rho-tui`. A shell out to
`pbcopy` or `xclip`. A copy performed inside the key handler, because the handler does no IO.

---

**Superseded on 2026-08-18 by `D-inline-viewport-not-alternate-screen`.**

The transcript is ordinary terminal output now, so a drag selects it and the terminal
copies it. rho needs no clipboard path, and it writes no OSC 52 sequence. The ban on a
clipboard crate in `rho-tui` stands, and it is now free.
