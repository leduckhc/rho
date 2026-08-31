# D-the-loop-owns-the-tick-clock

Status: accepted

## Question

Who owns time in the TUI, so the working sweep and the live turn clock both move,
and the renderer stays a pure function of state?

## Decision

The event loop owns time. The loop runs one `tokio::time::interval`. The interval
period is 100 milliseconds. Each interval fire calls `TuiState::on_tick(now_millis)`.
The reducer reads no clock. Time arrives as data, as `now_millis`.

The tick arm carries the guard `if events.is_some()`. So the loop ticks only while a
run is active. The loop never ticks while idle. So an idle screen never repaints on a
timer.

The tick period is 100 milliseconds because the sweep already assumes it.
`crate::motion::SWEEP_PERIOD_TICKS` is 20. One period is 20 ticks. So one sweep lasts
2 seconds. A slower tick would make the sweep jerk. A faster tick would repaint more
often for no visible gain.

## What this rules out

- The renderer must not read a process clock. `render` stays pure.
- The reducer must not read a process clock. `on_tick` takes the time as a parameter.
- The loop must not repaint on a timer while idle. The `events.is_some()` guard stops it.
- crossterm has no tick event, so the loop must not wait for one.

## Reason

The renderer asserts exact frames in tests. A clock read inside the renderer would make
a frame depend on wall time. Then no test could assert a frame. The one clock read in
this crate stays in `elapsed_millis`, inside the loop. See
`D-the-reducer-owns-the-row-metadata`.
