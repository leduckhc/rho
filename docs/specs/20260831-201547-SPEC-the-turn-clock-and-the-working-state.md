# SPEC-the-turn-clock-and-the-working-state — The turn clock and the working state

Status: draft for the working-state lane.

Owner crate: `rho-tui`. Related crate: `rho-cli`, which passes `--no-motion`.

This spec makes the working state move. It gives the TUI a tick source. The tick drives
two things. It drives the working-word sweep. It drives a live turn duration counter.

## The problem, in the user's words

The user asked for an animation that shows the model is working. The user asked for a
live duration counter beside it.

Neither works today. The renderer reads `state.tick`. No code ever changes `state.tick`.
The footer clock reads `state.turn_millis`. No code sets it until the turn ends. So the
working row sits still and the clock stays blank while a turn runs.

A live probe against Bedrock proved it. Over 25 seconds the status row held one value,
`◈ working ·`. The glyph never moved. No clock appeared. `--no-motion` rendered the same,
so the flag changed nothing.

## The sides

A side is any two places that must agree. This change has these sides.

- The **event loop** in `crates/rho-tui/src/app.rs`. It owns the terminal and the clock.
- The **reducer** `TuiState` in `crates/rho-tui/src/state.rs`. It stays pure.
- The **renderer** in `crates/rho-tui/src/render.rs`. It stays a pure function of state.
- The **CLI** in `rho-cli`. It passes the motion choice into the app.

The loop and the reducer must agree on one method, `on_tick`. The loop and the reducer
must agree on the end-of-run signature, `end_run`. The renderer and the reducer must
agree on the fields `tick` and `turn_millis`.

## The contract kinds this change touches

- The **public API**: two methods on `TuiState`, and one associated constant.
- The **data model**: the meaning of `tick` and `turn_millis` while a turn runs.
- The **behaviour rules**: when the loop ticks, and when it must not.
- The **configuration**: what `--no-motion` now does, observably.

## The contract, as compilable Rust

### 1. The tick period

The tick period is a constant in `crate::motion`, beside the sweep it drives. The type is
`u64`, so `std::time::Duration::from_millis` takes it with no cast.

```rust
// crates/rho-tui/src/motion.rs

/// The tick period, in milliseconds. One tick is 100 milliseconds.
///
/// The sweep period is `SWEEP_PERIOD_TICKS` ticks, which is 20. So one sweep lasts
/// `SWEEP_PERIOD_TICKS * TICK_PERIOD_MILLIS` milliseconds, which is 2000, or 2 seconds.
/// The loop fires one tick per this period while a turn runs. See
/// `D-the-loop-owns-the-tick-clock`.
pub const TICK_PERIOD_MILLIS: u64 = 100;
```

### 2. The tick reaches the state

The reducer gains one method. It advances the tick. It refreshes the live turn duration.
It reads no clock. The caller passes the time as data.

```rust
// crates/rho-tui/src/state.rs, inside `impl TuiState`.
// `ActivityState` is `crate::state::ActivityState`.

/// Advance one tick, at `now_millis` on the caller's clock.
///
/// Pure. No IO. No clock read. The loop calls this once per
/// `crate::motion::TICK_PERIOD_MILLIS` while a run is active. The loop never calls it
/// while idle, so the tick never advances on an idle screen. See
/// `D-the-loop-owns-the-tick-clock`.
///
/// It advances `tick`, which drives the motion sweep. While a turn runs it also
/// refreshes `turn_millis` from the turn start, so the footer clock grows live. While
/// no turn runs it leaves `turn_millis` alone, as a fail-safe against a stray call.
pub fn on_tick(&mut self, now_millis: i64) {
    self.tick = self.tick.wrapping_add(1);
    if self.activity == ActivityState::Running {
        if let Some(start) = self.turn_started {
            self.turn_millis = Some(now_millis - start);
        }
    }
}
```

The tick does not advance while idle. The loop arm carries the guard `events.is_some()`.
`on_tick` also checks `Running` before it touches the clock. So a stray idle call moves
the sweep counter but never the turn clock.

### 3. The live turn duration starts at zero

`TurnStart` now resets the live clock. So the first frame of a new turn shows `0s`, not
the last turn's final value. The reset is one line in the existing `apply` arm.

```rust
// crates/rho-tui/src/state.rs, inside `apply`, the `TurnStart` arm.
// `AgentEvent` is `rho_core::AgentEvent`.

AgentEvent::TurnStart => {
    self.activity = ActivityState::Running;
    self.canceling = false;
    self.turn_started = Some(now_millis);
    // The live clock starts at zero. Without this the footer shows the last turn's
    // duration until the first tick fires, which reads as a frozen, wrong clock.
    self.turn_millis = Some(0);
}
```

Before the first tick fires, the footer reads `format_duration(Some(0))`, which is `0s`.
So the clock shows `0s` at once, then grows. It never shows a blank slot during a run,
and it never shows the previous turn's value.

### 4. A run that ends with no `AgentEnd` freezes the clock

`end_run` gains a `now_millis` parameter. It settles the final duration. So a provider
error, or a closed stream, freezes the clock at the moment the run ended.

```rust
// crates/rho-tui/src/state.rs, inside `impl TuiState`.
// `AgentStopReason` is `rho_core::AgentStopReason`.

/// End the run when the event stream stops with no `AgentEnd`.
///
/// It settles `turn_millis` from `turn_started`, so a failed or closed run freezes its
/// final duration. Pass `failed` true for a stream error. Pass `now_millis` from the
/// same clock the reducer folds with. See `D-a-failed-turn-freezes-the-clock`.
pub fn end_run(&mut self, failed: bool, now_millis: i64) {
    let was_canceling = self.canceling;
    self.activity = ActivityState::Idle;
    self.canceling = false;
    if let Some(start) = self.turn_started {
        self.turn_millis = Some(now_millis - start);
    }
    if failed {
        self.last_error = true;
        self.status = "the run ended with an error".to_string();
    } else if was_canceling {
        self.last_stop = Some(AgentStopReason::Canceled);
        self.status = "canceled".to_string();
    }
}
```

Both callers in `app.rs` change. The failed arm calls `state.end_run(true, elapsed_millis(started))`.
The closed arm calls `state.end_run(false, elapsed_millis(started))`. After the run ends
the activity is `Idle`, so the loop stops ticking and the clock holds its frozen value.

### 5. The loop owns the timer

The loop builds one interval. The tick arm carries the `events.is_some()` guard. So the
loop repaints per tick while a run is active, and never while idle.

```rust
// crates/rho-tui/src/app.rs, inside `event_loop`, near `let mut input = EventStream::new();`.
// `Duration` is `std::time::Duration`. `MissedTickBehavior` is `tokio::time::MissedTickBehavior`.

let mut ticker = tokio::time::interval(Duration::from_millis(crate::motion::TICK_PERIOD_MILLIS));
// A skipped idle tick must not fire a burst when the next run starts.
ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
```

```rust
// crates/rho-tui/src/app.rs, a new arm in the `tokio::select!`.
// It draws per tick only while a run is active.

_ = ticker.tick(), if events.is_some() => {
    state.on_tick(elapsed_millis(started));
    draw(terminal, state)?;
}
```

The submit arm resets the interval, so the first live tick lands one full period after
the turn begins. Add `ticker.reset();` to the `KeyAction::Submit` branch, after the loop
sets `*events = Some(stream);`.

The idle cost is zero. The guard skips the arm while `events` is `None`. So an idle
screen never repaints on a timer. A busy loop that repaints an idle screen would be a
defect. The guard is what stops it.

## Behaviour rules

### `--no-motion`, observably

`--no-motion` stops the sweep band. It does not stop the live counter. See
`D-no-motion-stops-the-sweep-not-the-clock`.

With motion on, the working-word cells change style across ticks. The sweep band moves.
With motion off, the working-word cells hold one style across every tick. The word sits
still. In both cases the duration counter grows once per tick.

The renderer already gates the sweep on `motion_enabled`, inside `apply_sweep`. The
renderer must not gate the counter on motion. The test `no_motion_word_is_identical_across_ticks`
is the one that fails if the flag stops working. See the test cases.

### The amber cue

While a turn runs, the footer duration turns amber past one minute. The renderer reads
`live_duration_is_amber(millis)` on the live `turn_millis`. At or below 60000 milliseconds
the duration keeps the normal text role. Above it, the duration takes `Role::Warn`, which
is amber. This wires `live_duration_is_amber`, which no code calls today.

The amber applies only while the turn runs. After the run ends, the footer draws the
duration in its normal role, whatever the value. So a slow but finished turn reads calm,
and a slow running turn reads as a live warning.

### A cancelled turn and a failed turn

A cancelled turn freezes the clock. The final value is the elapsed time at the cancel.
The footer reads `done · canceled · <duration>`.

A failed turn freezes the clock. A provider error ends the stream with no `AgentEnd`. The
loop calls `end_run(true, now)`. The footer reads `done · error · <duration>`. The value
is the elapsed time at the failure.

## Test cases

Every test passes milliseconds as data. No test sleeps. No test reads a clock. No test
uses the network. This obeys AGENTS.md step 5.

- `on_tick_advances_the_tick`: one `on_tick` call raises `tick` from 0 to 1.
- `on_tick_grows_the_live_turn_duration`: a `TurnStart` at 0, then `on_tick` at 2500,
  sets `turn_millis` to `Some(2500)`, so the footer shows `2.5s`.
- `turn_start_resets_the_live_clock_to_zero`: after a turn ends at 5000, a new `TurnStart`
  sets `turn_millis` to `Some(0)`, so the footer shows `0s`, not `5s`.
- `on_tick_while_idle_leaves_the_turn_clock`: with activity `Idle`, `on_tick` raises `tick`
  but leaves `turn_millis` unchanged.
- `no_motion_word_is_identical_across_ticks`: with `animate` false, the rendered
  working-word cells at tick 0 and tick 7 hold identical styles. This proves `--no-motion`
  has an observable effect.
- `motion_word_changes_across_ticks`: with `animate` true, the rendered working-word cells
  at tick 0 and tick 7 differ. This proves the sweep animates.
- `live_duration_over_a_minute_renders_amber`: with a running turn and `turn_millis` of
  `Some(61000)`, the footer duration cells carry `Role::Warn`.
- `live_duration_under_a_minute_is_normal`: with a running turn and `turn_millis` of
  `Some(59000)`, the footer duration cells carry the normal text role.
- `a_failed_run_freezes_the_final_duration`: a `TurnStart` at 0, `on_tick` to 3000, then
  `end_run(true, 3200)`, sets `turn_millis` to `Some(3200)`, and the footer reads
  `done · error · 3.2s`.
- `a_canceled_run_freezes_the_final_duration`: a cancel, then `end_run(false, now)`, sets
  `turn_millis` from `turn_started` and the footer reads `done · canceled · <duration>`.
- `finished_duration_over_a_minute_is_not_amber`: after the run ends with `turn_millis`
  above 60000, the footer duration keeps the normal role.
- `duration_tick_growth_does_not_reflow`: this test already exists in
  `crates/rho-tui/tests/duration.rs`. It drove tick growth that production never produced.
  After this spec, production produces that growth, so the test guards a real path.

## Out of scope

- No change to the duration ladder. `format_duration` and `duration_slot` stay as they
  are. See `F-duration-ladder` and `F-duration-slot`.
- No change to the sweep shape. `sweep_weight`, `motion_cell`, and `sweep_frame` stay as
  they are. See `F-working-motion`.
- No second animation. One motion marks the working state, and this spec keeps it.
- No per-tool live clock. Only the turn clock grows live. A tool row still settles once,
  at its end.
- No configurable tick period. The period is a constant, tied to the sweep period.
- No spinner for a background task row. That row settles by its own task events.
