# D-follow-is-derived-not-stored — One scroll number, and auto-follow is a question asked of it

**Question (controller, while specifying the transcript scroll):** the view needs a scroll
position and an auto-follow flag. Does the state hold two fields?

**Decision:** No. `TuiState` holds one number, `scroll_rows`. It counts rendered transcript
lines between the bottom of the view and the newest line. Auto-follow is the answer to
`scroll_rows == 0`, and `TuiState::follows()` returns it. No flag exists, so no flag can
disagree with the position.

**Reason:** Two fields for one fact fail open. A stale `follow = true` with a non-zero
offset would drag the view down on the next token, and the user would blame the terminal.
This project already paid for a pair that could disagree: `ToolKind::Other` counted as
non-mutating, so a read-only policy approved an undeclared tool. See
`D-plugin-does-not-classify-itself`.

A derived answer also makes the invariant testable as a pairing, which step 12 of
`AGENTS.md` asks for. A test asserts that new output moves the view only while
`scroll_rows == 0`.

**Rules out:** A `follow: bool` field. A `sticky_bottom` flag. Any code that sets follow
without moving the position. A reducer that resets the position when a row arrives.

---

**Superseded on 2026-08-18 by `D-inline-viewport-not-alternate-screen`.**

`TuiState` holds no scroll position at all now, so there is nothing to derive. The rule the
decision defended still stands as a habit: one fact lives in one field, and a second field
that can disagree is a defect.
