# D-a-failed-turn-freezes-the-clock

Status: accepted

## Question

What does the footer clock show when a run ends with no `AgentEnd` event, which is
what a provider error produces?

## Decision

`TuiState::end_run` settles the final turn duration. It takes `now_millis` and writes
`turn_millis` from `turn_started`. So a run that ends with no `AgentEnd` freezes its
clock at the moment it ended.

The clock then holds that frozen value. The loop stops ticking, because the run is over
and the tick arm needs an active run. So the digits stop moving and stay put.

A cancelled turn freezes the same way. A first Ctrl-C cancels the turn. The turn may end
by `AgentEnd` with reason `Canceled`, or by a closed stream with no event. Both paths
settle the clock. `on_agent_end` settles it on the first path. `end_run` settles it on
the second path.

## What this rules out

- The clock must not keep growing after a provider error.
- The clock must not drop to an empty slot after a failed turn.
- The clock must not show a stale zero after a failed turn.
- `end_run` must not settle the clock without a real time source, so it takes `now_millis`.

## Reason

Today `end_run` sets no duration. So a failed run shows whatever the last tick wrote,
which is up to 100 milliseconds stale. Settling once at the end makes the final value
exact. A frozen, exact duration tells the user how long the failed turn ran. See the
failed-turn arm in `crates/rho-tui/src/app.rs`.
