# D-ledger-wins-the-band — The band stays a fixed fourteen rows, and an approval never yields

Date: 20260818

## The question

The mock and the renderer disagreed on the shape of the inline band. Four directions were
drawn and compared in a browser. The question was which one the renderer must follow.

The measured problem that started it: the first mock was 57% blank rows at idle. A fixed
band of fourteen rows pushes the shell prompt fourteen rows down to show two lines of text.

## The decision

**Direction A, "Ledger", wins.** The band stays a fixed `BAND_ROWS = 14`.

The owner chose it. The reason that decides it: a fixed band never marks the terminal.

Ledger resolves the row budget by rank:

1. The footer keeps its row.
2. A panel takes its rows next.
3. The composer scrolls inside what remains, and it never drops below three rows.

**One amendment to Ledger, made before it was adopted. An approval never yields a row.**
The first draft of the squeeze frame dropped the session root line to save one row. So the
panel named `rm -rf target` and hid the directory it ran in. A panel that states a
destructive command states all of it. The composer gives up the row instead.

The help screen becomes a scrolling window over the binding table, with a counted header
and a direction marker.

## What this rules out

- **No elastic band.** Direction B rested at four rows and gave ten rows back to the
  shell. It is rejected, because a band that grows scrolls the terminal's real scrollback
  upward, and a band that shrinks leaves blank rows that rho may not reclaim.
- **No tall panel that outgrows the band.** Direction D drew a thirty-row help screen and
  named the price: closing it leaves a seam of up to sixteen blank rows. rho refuses to
  rewrite rows the terminal owns, and `D-inline-viewport-not-alternate-screen` refuses the other escape. So a
  panel stays inside the band.
- **No aligned target column.** Direction C packed nine events where the baseline showed
  three, and its two-column help fitted the whole table into nine rows. It is rejected as
  a whole, because a fixed target column mutilates a long command, and a command is the
  row a user most needs verbatim.

## The evidence

The three rejected mocks stay in `docs/design/variants/`. Each one states its own frames
and its own weakness, and each passes `docs/design/check-mock.js`. Read them before any
later work reopens this question.

Two findings came out of the comparison and they outlive the losing directions:

- Direction D found the cost of elasticity that direction B did not state. Both grow and
  shrink, and only D drew the seam.
- Direction A's own squeeze frame found the approval defect above. A green mock drew it,
  and the prose beside it claimed a four-row approval while the band held three.

## Open, and not settled here

- `shift+enter` is the newline key, and no terminal reports it yet. rho never pushes the
  keyboard enhancement flags. See the separate question about the keyboard protocol.
- Whether `alt+enter` stays in the binding table. Every drawn mock assumes it is gone.
