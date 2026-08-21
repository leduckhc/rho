# D-the-wheel-needs-capture — rho captures the mouse, because the wheel is the only way to scroll

Date: 20260819

## The question

`D-native-selection-is-the-default` turned mouse capture off. Does that hold now that rho
owns the alternate screen?

## The decision

**No. Mouse capture is on by default.** `--no-mouse` turns it off, and it wins over the
config and the environment.

## The reason

The older decision gave this reason: with capture off, the terminal keeps drag-select **and
its own wheel**. That reason was true for an inline band. The terminal owned the scrollback,
so its wheel scrolled the transcript for free.

**The alternate screen has no scrollback.** So with capture off the wheel does nothing at
all, and the user has no way to scroll the transcript with the mouse. The habit the older
decision protected does not exist here.

The prior art agrees, and it was measured. `opencode` and `cline` both take the alternate
screen, and both take the mouse. No measured app takes the screen and leaves the mouse,
because that combination leaves the user with no wheel.

## The cost, and what pays it back

Capture takes drag-select from the terminal. Two things reduce the cost:

- Every terminal rho targets keeps a modifier bypass. Option and drag selects text in
  Ghostty and in iTerm2, so a user still copies with the mouse.
- `--no-mouse` gives the terminal the mouse back for a user whose terminal has no bypass.

The footer must state the bypass, because a user who cannot select text and reads no hint
reports a defect.

## What this rules out

- **No automatic capture switch.** rho does not turn capture off while it thinks the user
  wants to select. A mode that guesses is worse than a flag that states.
- **A config with no `tui.mouse` key reads as the new default, which is on.** An old config
  gains the wheel and needs no edit.
