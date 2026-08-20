# D-a-panel-nobody-can-open — A rendered panel with no key that opens it is not a feature

**Question (controller, after a user drove the interface):** sprint 3 built the slash list,
the help panel, the binding table, the filter, and a test for each. A user pressed `/` and
got a literal slash in the draft. Was the feature built?

**Decision:** No. A key must reach every panel the renderer can draw, and the interface must
never promise a key it does not answer. So the key handler now routes by panel, and these
contracts hold.

- A chord is never text. `CONTROL` or `ALT` with a letter types nothing. Ctrl-D on an empty
  draft leaves rho, the way it ends a shell.
- `/` and `?` open their panel only on an empty draft. Inside a draft they stay text,
  because `why?` is a question.
- Esc closes a panel and keeps the draft. A path starts with a slash, so a user opens the
  list by accident, and a draft belongs to the user.
- Tab completes the draft to the selected command. Enter runs it. A mouse click runs the row
  under the pointer.
- A command with no implementation reports on screen. `/model`, `/sessions`, and `/guide`
  each push an error row that says it is not built yet.
- The transcript and the footer are the only channels the user reads. `state.status` is not
  a channel.

**Reason:** Every piece existed and passed its test. `slash_commands`,
`filter_slash_commands`, `help_rows`, `Panel::SlashList`, `Panel::Help`, `slash_panel`, and
`help_panel` were all green. `handle_key` answered four keys: Enter, Backspace, a printable
character, and Ctrl-C. So the suite proved that each part works, and nothing proved that a
user can reach any of it.

The worst case was the exit. `handle_ctrl_c` set
`self.status = "press Ctrl-C again to exit"`, and no code in `render.rs` reads
`state.status`. A grep for `.status` in the renderer returns nothing. Ctrl-C twice did quit,
in 11 milliseconds, and the user reported that Ctrl-C does not close the session, because
the interface never said a word. The same silence hid `"canceling…"` and every
`"run error: …"`.

A resize was one arm in the event loop. `Some(Ok(_)) => {}` swallowed `Event::Resize`, so the
frame kept the old width until the next key press. A probe measured it: after a resize from
100 to 60 columns, the widest drawn row was still 100.

**What this rules out:** A field that carries a message no code draws. A binding in
`bindings()` with no arm in `handle_key`. A panel variant with no key that opens it. A slash
command that resolves and then does nothing. A catch-all event arm that hides a class of
events. A green test suite as evidence that a feature is reachable.

**What it cost, and what changed in the spec.** `SPEC-tui-experience` listed mouse support
under `## Out of scope`. The user asked for the list to be selectable by mouse, so the
exclusion is gone and the spec now names the mouse tests. Mouse capture is on for the
lifetime of the app, and the click maps to a row through `slash_row_index`, which asks the
same `plan_layout` the renderer uses. Two copies of the geometry would drift.

**Still not wired, and stated rather than hidden:** `ctrl-o`, `ctrl-e`, and `alt+enter`.
`bindings()` promises all three on the help screen, and each needs a fold model or a
multi-row composer behind it. That is stage U6 work, not a key routing change. So the
binding table now carries a `built` flag, and the help screen prints `· not built yet` on
an unwired row. The help screen became reachable in this change, so an unwired promise
would otherwise be a new defect of the same family.

**Two findings from the review, both kept:**

A run can end with no `AgentEnd`. `rho_core::Driver::run` returns on `TurnOutcome::Failed`
and on `Closed` without a stop event, so the frontend stayed `Running` for the rest of the
session. The footer then read `canceling` forever, and the next Ctrl-C routed to a cancel on
a token that was gone, so Ctrl-C could no longer quit. `TuiState::end_run` ends the run from
the frontend side.

The event loop now admits `KeyEventKind::Press` only. A terminal that reports `Repeat` would
turn one held Ctrl-C into an arm and an exit, which defeats the two-press gate.

**Two hollow tests of my own, and how they were found:** a mutation pass, not the review.
One click test used `/model`, the first row, so it passed while the click always ran row 0.
One disarm test pressed `/` after arming the gate, and that key press does the disarming.
Both are rewritten. The rule stands: break the code and watch the test fail, or the test is
not a guard.
