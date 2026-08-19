# D-alternate-screen-after-all — rho takes the whole screen, and it gives the transcript back with one key

Date: 20260819

## The question

Does rho draw in an inline band, or does it take the alternate screen?

This question was settled twice before, and both records stay. `D-keep-the-alternate-screen`
said keep it, for a reason that was false. `D-inline-viewport-not-alternate-screen` then
dropped it, for a reason that was true. This decision reverses the second one.

## The decision

**rho enters the alternate screen when it starts, and it owns the whole terminal.** The
owner decided it, and the evidence below says the cost is payable.

`ctrl-p` leaves the alternate screen, writes the whole transcript into the terminal's own
scrollback, and returns. So the terminal's search and its copy still reach every row.

rho turns mouse capture **on** by default, because in the alternate screen the wheel is the
only way to scroll, and a terminal offers no scrollback to fall back on.

## The reason

The prior art is split, and the split is informative. Each app was measured, not recalled:

| App | Alternate screen | Mouse capture | Note |
| --- | --- | --- | --- |
| opencode | yes | yes | it takes the screen at startup |
| cline | yes | yes | it takes the screen at startup |
| claude | no | no | it grows the inline region for a picker, then shrinks it |
| codex | no | no | it left the alternate screen in an earlier version |
| pi | no | no | |

The two apps that take the screen also take the mouse. No app takes the screen and leaves
the mouse, because that combination leaves the user with no way to scroll.

**Claude Code shows that an elastic inline region is possible.** Its `/resume` picker grows
the inline region to thirteen rows and then returns to seven, and it leaves no blank rows
behind. So a growing region does not have to leave a seam. A row that already went to the
terminal's scrollback cannot be reclaimed, and a row the application drew in its own live
viewport can be. `D-ledger-wins-the-band` said a seam was unavoidable, and that claim was
wrong.

That option is rejected here for a different reason, stated below.

A spike settled the mechanics on a real terminal. See `docs/verification/alt-screen-spike.md`.
It proved the wheel arrives, a resize holds, the dump recovers every row, and the exit is
clean.

The gain is the whole terminal height. The 22-row key table stops needing a window. An
approval never has to drop a row. A tall draft and a panel stop competing for fourteen rows.

## Why not the elastic inline region

It keeps the terminal's own search and scrollback, which is a real gain, and Claude Code
proves it works. It is rejected because it keeps **two owners of scrolling**.

Inline, the terminal owns the history and rho owns the live region, so every row has a state:
frozen, or still rho's. That boundary is the source of the freeze machinery, and it already
produced one defect that this spec has to repair, which is the reducer dropping a late event
that names a frozen row. Every future panel, every resize, and every repaint has to ask the
same question about every row.

The alternate screen deletes the question. rho owns every row, so any row can be repainted,
and `ctrl-p` hands the whole transcript to the terminal when the user wants it. One owner,
one invariant, and a spike proved the cost is payable.

## What this rules out

- **No two renderers.** rho has one screen model. There is no `--inline` flag and no mode
  switch, because a second layout doubles every frame, every fixture, and every future
  panel. `D-keep-the-alternate-screen` ruled this out first, and it stays ruled out.
- **No setting to invert the wheel.** The operating system applies its natural-scrolling
  preference before the terminal sees the event, so rho cannot detect the setting and must
  not try. An invert setting would double-invert for every natural-scrolling user.
- **No silent restore.** Terminal restoration stops being a statement at the end of `run`.
  It becomes a `Drop` guard, a panic hook, and a signal handler. See the reason below.

## What this supersedes

- `D-inline-viewport-not-alternate-screen` is superseded. Its claim was correct and its
  premise changed: rho now wants the screen for the height, not for the fixed composer.
- The row-budget half of `D-ledger-wins-the-band` is superseded. `BAND_ROWS` stops being a
  layout budget, so the ranked yield, the squeeze frame, and the eight-row help window lose
  their reason to exist.
- The rest of `D-ledger-wins-the-band` stands, and it stands unchanged: the composer is two
  rules with open sides and a ten-row draft cap, a tool row leads with its status glyph and
  indents two columns, and **an approval states its session root and never yields a row**.

## The reason restoration becomes structural

A killed rho leaves the terminal unusable, and this was measured on the product, not
guessed. With `--mouse`, a `SIGTERM` left mouse reporting on, so every mouse move printed
`35;111;18M` into the shell. `SIGHUP` is what closing a window sends, and it behaves the
same. A panic unwinds past `run`, and `restore_terminal` is a statement, so it never runs.

Inline, that fault costs a garbled prompt. In the alternate screen it costs the whole
terminal, because the user is left in a buffer with no prompt and no echo. So the guard is
part of this decision and not a later cleanup.
