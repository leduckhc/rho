# D-a-late-event-for-a-frozen-row-is-dropped — the reducer never writes a row the terminal owns

**Question (contract review):** `tool_row_mut`, `agent_row_mut`, and `task_row_mut` search
every row by id. A repeated `ToolEnd`, or a `ToolStart` for an id that already finished,
would write a row that is already in the terminal's scrollback. What happens?

**Decision:** The event is dropped. A lookup for an update returns a live row only, so a
frozen row is unreachable from the reducer. `on_tool_start` pushes a new row only when the
id matches no row at all, live or frozen, so a late start pushes no duplicate.

A drop is counted in `TuiState::late_events`. A test asserts the count, and a live run
prints it at `debug` level. So a provider that repeats itself is visible, not silent.

**Reason:** rho cannot repaint a frozen row, so a write to one puts the state and the screen
in permanent disagreement. The state would say `failed` while the user reads `running`.

The reducer cannot assume a well-behaved event order. Sprint 1 found three providers that
sent surprising requests and responses, and none of them was caught by a fixture. See
`docs/verification/sprint-1.md`. A contract that trusts the order is the same class of
mistake as `ToolKind::Other`, which trusted a tool to declare its own kind.

Dropping is safe here, because the row already reached its final state before it froze. The
session file keeps every event either way, so nothing is lost from the record.

**Rules out:** A reducer lookup that reaches a frozen row. A second row for an id that
already exists. A silent drop with no counter. Unfreezing a row to repair it.
