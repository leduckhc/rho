# The alternate screen, proved on a real terminal

Date: 20260819. Spike: a throwaway binary outside the repo, 192 lines, at `/tmp/altspike`.
Terminal: ghostty, inside tmux, on macOS with natural scrolling.

The owner asked for the whole application in the alternate screen. That reverses
`D-inline-viewport-not-alternate-screen`. The last time this question was settled from
memory, the answer was wrong and needed a supersede note. So it was settled by measurement
again. Nothing in `crates/` changed for this record.

## 1. The mechanics work

| Question | Result |
| --- | --- |
| Does the application own the whole terminal? | Yes. 20 body rows at 24 rows high. |
| Does the wheel reach the application? | Yes, as SGR button 64 and 65. |
| Can a dump leave the alternate screen and return? | Yes. The display came back clean. |
| Does the dump survive in the real scrollback? | Yes. 40 of 40 prose rows and 40 of 40 tool rows. |
| Does a resize hold? | Yes, at 24, then 40, then 12 rows. |
| Does it exit cleanly? | Yes. |

So the one real cost of the alternate screen, which is the loss of the terminal's own
history, is recoverable with one key.

## 2. The scroll direction cannot be detected, and must not be

The wheel carries two logical directions, button 64 and button 65. The operating system
applies its natural-scrolling preference **before** the terminal sees the event. So the
application receives a normalised direction, and a natural setting and a normal setting
look the same to rho.

Therefore rho maps `ScrollUp` to older rows on every platform, and it never offers a
setting to invert the wheel. An invert setting would double-invert for every user who has
natural scrolling on.

The owner confirmed the mapping from the other side. A physical swipe down on a natural
setting reaches rho as button 64, which is `ScrollUp`, which shows older rows. That is what
Safari does with the same gesture.

## 3. The transcript is oldest first

The newest row is at the bottom, directly above the composer. `g` showed turn 1 at the top,
and `G` showed turn 40 at the bottom. rho's own frames agree: a tool row pair reads `read`,
then `edit`, then `bash`, in the order they ran, and the composer sits below all of them.

So both of these hold, and they agree with each other:

| Input | Offset | Reaches |
| --- | --- | --- |
| `j`, `Down` | increases | the newest row |
| wheel down, button 65 | increases | the newest row |
| `k`, `Up` | decreases | the oldest row |
| wheel up, button 64 | decreases | the oldest row |

## 4. The defect the owner found by hand

The owner reported that the wheel jumped, and that the same gesture landed in random
places. A scroll to the bottom showed turn 8 one time and turn 1 another time.

The spike logged every event. Three faults compounded:

1. **The offset was clamped where it drew, and never where it moved.** A momentum flick
   sent hundreds of events, and each one raised the offset past the last row. The screen
   stood still, so the reading looked correct. A scroll back then paid one event per event
   overshot, which reads as a dead wheel and then a jump.
2. **Each event moved three rows.** Trackpad momentum multiplied that.
3. **The trackpad also sends horizontal events.** They must be ignored by name.

The log, over one session of hand scrolling:

```
real vertical scroll events            2789
arrived while already at the bottom     562
arrived while already at the top        398
horizontal noise, ScrollLeft or Right   537
offset above its maximum                  0
```

The 960 edge events are the defect. Each one used to inflate the offset.

**The same fault was live in rho.** `help_offset` grew without a bound while `help_panel`
clamped at draw time, so the help window banked key presses. A `#[derive(Default)]` also
gave `help_visible_rows` the value zero, and a zero there clamped the window to one row.
Both are fixed, and `the_help_window_answers_the_first_press_back` pins the behaviour. The
test that missed it asserted the drawn rows and never pressed a key afterwards.

## 5. What the scroll contract must state

1. Clamp the offset where it changes, never only where it draws.
2. One row for each wheel event.
3. Ignore `ScrollLeft` and `ScrollRight` by name.
4. The newest row is at the bottom. `ScrollUp` shows older rows.
5. Offer no setting to invert the wheel.
6. Hold the view at the bottom while output streams, and release that hold when the user
   scrolls up. Restore it when the user returns to the bottom. The spike has no live output,
   so this rule is unproved and it needs its own test.
7. Draw a one-column rail only when the transcript overflows.
8. State what mouse capture costs. Capture must be on for the wheel to arrive, and capture
   can take native selection away. Claude Code shipped the opposite and needed four repairs.

## 6. What the spike does not prove

- **Live output.** The transcript is 160 fixed rows, so rule 6 above is untested.
- **Copy and paste.** No selection or OSC 52 path was exercised.
- **`shift+enter`.** The alternate screen does not fix it. rho still pushes no keyboard
  enhancement flags, so the terminal still reports a bare carriage return.
- **Windows and Linux.** Every measurement here is macOS, ghostty, and tmux.
