# Inline viewport spike — the alternate screen is not needed

Date: 2026-08-18. Author: controller. Owner asked one question: is the alternate screen
needed at all? The answer is no, and this page holds the evidence.

The spike lives outside the repository, at `/tmp/inline-spike`. It links `ratatui 0.30.2`,
which is the version `rho-tui` already uses. It writes no rho code.

## What the spike does

It prints two lines of ordinary shell output. Then it opens a `Viewport::Inline(8)` band,
with no alternate screen. It draws a composer inside the band. It freezes three rows
upward with `Terminal::insert_before`. It grows the draft to four rows inside the same
band. It freezes thirty more rows. The driver resizes the pty from 80x24 to 60x18. The
spike then redraws and leaves.

## How it ran

The driver forks a real pseudo-terminal and plays the part of the terminal. It answers the
cursor-position query, which an inline viewport needs to anchor its origin. It reuses
`bench/ptyharness.py` for bounded teardown, per `D-pty-teardown-closes-before-it-waits`.

```sh
cd /tmp/inline-spike && cargo build --release
python3 drive.py            # the five step script, with a resize
python3 drive.py bulk       # thirty single-row inserts, for the cost number
```

The first attempt hung on a blank screen and reported this:

    Error: "The cursor position could not be read within a normal duration"

That failure is itself a finding. An inline viewport asks the terminal where the cursor is.
A harness that does not answer gets no interface at all.

## Result 1. No alternate screen, and the shell output survives

    alternate screen sequence (?1049h) present: False
    scrollback lines held by the terminal: 19
    first 4 scrollback lines: ['shell line A: cargo test', 'shell line B: 698 passed',
                               'frozen 1', 'frozen 2']

## Result 2. The composer grows inside a band of fixed height

`Viewport::Inline(height)` carries the height, and `Terminal::resize` recomputes only the
origin. The field is private, and no setter exists. So the band height is fixed for the
life of the `Terminal`.

That does not block the design. The draft grew from one row to four rows inside the same
eight-row band, because the layout inside the band moves. This is the answer to the one
question that blocked the decision.

## Result 3. A resize keeps the band whole

The final frame, after the pty went from 80x24 to 60x18:

    0 |live: after resize
    3 |┌──────────────────────────────────────────────────────────┐
    4 |│> draft row 1                                             │
    5 |│> draft row 2                                             │
    6 |└──────────────────────────────────────────────────────────┘
    7 |ready · enter send · ? help
    8 |SPIKE-DONE

The box redrew at the new width. The rows above stayed. The exit left the cursor below the
band, so the shell prompt returns in the right place.

## Result 4. The frozen rows reach a real scrollback

`pyte` models a screen, so it is not the last word on scrollback. `tmux` keeps a real
history, and it answers the question directly.

```sh
tmux new-session -d -s spike -x 80 -y 24 "/tmp/inline-spike/target/release/inline-spike; sleep 30"
sleep 3 && tmux capture-pane -p -t spike -S -200 | grep -c "bulk row"
```

    30

All thirty frozen rows are in the terminal's own history, with the three earlier rows and
the two shell lines above them. The user's wheel, their selection, and their search all
work on that text, because it is ordinary terminal output.

This test ran twice, once for each build:

| Build | Rows in the tmux history |
| --- | --- |
| default features | 30 |
| `scrolling-regions` feature | 30 |

So the feature is not needed for correctness. The next result says why we want it anyway.

## Result 5. The `scrolling-regions` feature cuts the write cost by seven times

Thirty single-row inserts, each followed by a redraw, which is what a streaming turn does.
The number is the byte count the program wrote to the pty.

| Build | Bytes written |
| --- | --- |
| default features | 24004 |
| `scrolling-regions` feature | 3425 |

Without the feature, `insert_before` clears the band and repaints it every time. With it,
`ratatui` sets a scroll region and scrolls. The escape trace shows the region:

    DECSTBM (scroll region) sequences written: ['3;13', '6;24', '1;16', '1;16']
    scroll-down/up (SU/SD) sequences: [('3', 'T'), ('11', 'T'), ('16', 'S'), ('3', 'S')]

A scroll region that does not start at line 1 can drop lines instead of keeping them. That
is why result 4 exists, and why it uses `tmux` rather than a screen model. The count is 30
either way, so no line is lost.

## What this changes

`D-keep-the-alternate-screen` gave a false reason: it said the fixed composer needs the
alternate screen. Result 2 disproves it. `D-inline-viewport-not-alternate-screen` replaces
it, and `SPEC-tui-inline-and-composer` replaces the spec built on it.

## What is still unverified

The spike ran under a forked pty and under `tmux`. It has not run in iTerm2, Terminal.app,
or Alacritty by hand. The band flicker is a byte count here, not a human judgement. Both
need the real binary, which is step 11 for the implementation.
