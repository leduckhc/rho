# D-native-selection-is-the-default — The terminal keeps the mouse unless the user gives it away

**Question (owner, after asking for a Claude Code class interface):** rho enables mouse
capture for the whole session, so the slash list answers a click. Does capture stay on?

**Decision:** No. Mouse capture is off by default. `EnterAlternateScreen` no longer carries
`EnableMouseCapture`. A new flat config key, `tui-mouse`, turns capture on, and the
environment form is `RHO_TUI_MOUSE`. `App::with_mouse(bool)` carries the value into the
event loop. With capture off the terminal keeps drag-select and its own wheel. With capture
on rho gets the wheel and the clickable list, and the user loses drag-select.

**Reason:** The earlier change traded a habit every terminal user owns for one clickable
list. Drag-select works in every other program on the screen. A user who has never read our
help still expects it. Claude Code shipped capture first and then needed repairs, an
environment variable, and a second render mode.

The keyboard covers what capture would give us. `⇞ ⇟`, `ctrl-home`, and `ctrl-end` scroll.
`v`, `y`, and `o` select and copy. The arrows and `tab` already drive the slash list.

**Rules out:** Capture on by default. Capture as a build-time constant. A wheel handler that
only works when capture is on by default. Any claim that the list needs a mouse, because the
list already answers four keys.

---

**Superseded on 2026-08-19 by `D-the-wheel-needs-capture`.**

The reason above rests on the terminal keeping its own wheel. That is true for an inline
band, and false in the alternate screen, where there is no scrollback for a wheel to scroll.
So capture off now means no wheel at all. Capture is on by default, and `--no-mouse` turns
it off.
