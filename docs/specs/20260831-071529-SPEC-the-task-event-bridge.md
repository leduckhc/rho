# SPEC-the-task-event-bridge — the sixth dead switch, and the row a user finally sees

Status: delivered. Driven for real against live Bedrock; see
`docs/verification/task-event-bridge.md`.
Owning crates: `rho-core`, `rho-tui`, `rho-cli`
Features: F-background-tasks, F-task-progress-row
Decisions this spec implements: D-a-task-event-outlives-its-tool-call,
D-a-lagged-frontend-is-resynced-not-told

## 1. The problem

`SPEC-the-task-row-draws-its-progress` drew the task row. It also named what it could not fix,
in its own out-of-scope list:

> **The missing bridge.** Nothing subscribes to `TaskRegistry::subscribe` in shipped rho, and
> `ToolContext::agent_events` lives for one tool call while a background task outlives the
> turn. So no task row can appear in a real session today, whatever it draws.

That is the whole defect. Three parts prove it:

1. `TaskRegistry::subscribe` has one caller in `crates/`, and it is `bench`'s own harness. No
   binary calls it.
2. `Driver::dispatch_one` builds `ToolContext::agent_events` per tool call, and drops it when
   the call returns. A background task returns its id in milliseconds and runs for minutes.
3. `TuiState::on_task_start`, `on_task_progress`, and `on_task_end` are reached by tests only.

So `rho` runs `cargo build` in the background, and the interface shows nothing at all. This is
the sixth dead switch of the family `D-dead-surface-is-a-defect-class` names.

## 2. The sides, and who owns each

| Side | Crate | What it owns |
| --- | --- | --- |
| The producer | `rho-core` | `TaskRegistry` broadcasts three task events. |
| The bridge | `rho-core` | A session-lifetime stream, and the lag repair. |
| The call site | `rho-cli` | It holds the registry, and it builds the interface. |
| The reader | `rho-tui` | The event loop folds an event and redraws. |

The contract kinds this change touches: the **public API** of `rho-core` and `rho-tui`, and the
**behaviour rules** for ordering, for lag, and for lifetime. It adds no wire format, no
persisted format, no config key, and no error variant.

## 3. The contract

Written first, as compilable Rust, before either side starts.

### `rho-core`

```rust
/// Events that reach a frontend outside any one agent run.
///
/// A background task outlives the tool call that started it, and it outlives the run
/// as well. So its events cannot travel on `AgentEvents`. A frontend holds this stream
/// for as long as it owns the screen. See `D-a-task-event-outlives-its-tool-call`.
pub struct SessionEvents {
    /* private */
}

impl SessionEvents {
    /// The next event to fold. `None` when the bridge has ended.
    ///
    /// The bridge ends when the task registry is dropped. A caller that gets `None`
    /// stops reading, because no later event can arrive.
    ///
    /// This is cancel-safe. A `select!` arm that loses a race loses no event, because
    /// every repair event waits in `self` and the only await is a channel receive.
    pub async fn next(&mut self) -> Option<AgentEvent>;
}

impl TaskRegistry {
    /// Bridge every task event onto a session-lifetime stream.
    ///
    /// This subscribes before it returns, so a task started right after this call
    /// still delivers its `TaskStart`.
    ///
    /// The bridge holds a weak reference to the registry, so it never keeps a killed
    /// session's child processes alive.
    pub fn session_events(self: &std::sync::Arc<Self>) -> SessionEvents;
}
```

### `rho-tui`

```rust
impl App {
    /// Read background task events for the whole life of the interface.
    ///
    /// Without this call the interface draws no task row, whatever the renderer can
    /// draw. The stream outlives one run on purpose: a task that ends while the user
    /// types must still mark its row finished.
    pub fn with_task_events(self, events: rho_core::SessionEvents) -> Self;
}
```

The reducer changes twice, and both changes are part of this contract. A repair after a lag
sends events a live session never sends, so the reducer must hold two more rules. See
`D-a-lagged-frontend-is-resynced-not-told`.

- **A `TaskStart` for an id that already has a row changes nothing.** Today it pushes a second
  row for one task. The row already holds the command, and its start time is the clock the
  duration slot measures, so an update would reset that clock.
- **A `TaskProgressed` for a finished row is ignored.** A stale report can arrive after a
  repair has finished the row, and no later report would correct it.

### `rho-cli`

One call at the interface builder, beside `with_motion` and `with_notices`. The registry is
already in scope there as `_tasks`, held for the life of the run.

## 4. The behaviour rules

**Order.** The bridge forwards each event unchanged, in the order the registry sent it.

**No lost start.** `session_events` subscribes inside the call. A task that starts after the
call returns delivers its `TaskStart`.

**A lag is repaired, not reported.** On `RecvError::Lagged`, the bridge reads the registry and
sends, for every task it holds, a `TaskStart` first and then that task's state now. A final
task sends `TaskEnd`. A running task with progress sends `TaskProgressed`. The `TaskStart` is
what makes the repair work at all: only `on_task_start` creates a row, and the lost window can
hold the start event itself. So no row stays `running` after its task ended, and no live task
is left with no row. See `D-a-lagged-frontend-is-resynced-not-told`.

**A repair never reorders against itself.** The bridge sends the whole repair before it reads
the channel again. A stale event that arrives after a repair is accepted, and the two reducer
rules above bound the damage to one percentage on a running row.

**Cancel-safety.** The bridge holds its repair backlog and its lag marker in `SessionEvents`,
not in the future that `next` returns. So a `select!` arm that loses a race loses nothing. A
lag marker that lived in the future would take the whole repair with it when the arm was
dropped, and the row would stay stale for good.

**Lifetime.** The bridge holds `Weak<TaskRegistry>`. It reads inside `next`, so it spawns no
task and a reader that leaves takes its subscription with it. It never extends the life of a
task process. Events already in the channel arrive before `next` returns `None`, because a
`broadcast` receiver drains before it closes.

**A killed task sends no `TaskEnd`.** `Drop for TaskRegistry` kills every running task and
broadcasts nothing. No reader is left with an unfinished row, because the bridge ends at the
same moment. This is stated so the next reader does not look for the missing event.

**Idle delivery.** The interface folds a task event while no run is active. This is the rule
that makes the feature real, because a build usually ends between two prompts.

**No duplicate.** The `bash` tool sends no task event on `ToolContext::agent_events`. It
reports through the registry handle only. So one event reaches the screen once.

## 5. What the contract forbids

- A frontend must not subscribe to the raw broadcast channel. It would have to handle lag.
- The bridge must not hold an `Arc<TaskRegistry>`. It would keep child processes alive.
- The bridge must not drop a `TaskEnd` for any reason. A stale row is the defect.
- The bridge must not send a state event for a task without a `TaskStart` before it. The
  reducer drops a state event that finds no row, in silence.
- The bridge must not hold a lag marker or a repair event inside the future `next` returns.
  A lost race would then lose the repair.
- `SessionConfig` gains no task field. See `D-no-four-argument-session-new`.

## 6. The extension point

A new frontend calls `TaskRegistry::session_events` and folds `AgentEvent` with its own
reducer. It edits no shared code. `rho-jsonl` is the next caller, and section 8 says why it is
not this lane.

## 7. Test cases

### `rho-core`, in `crates/rho-core/tests/session_events.rs`

- `a_task_event_reaches_a_session_stream` — a task that starts, reports, and finishes delivers
  `TaskStart`, `TaskProgressed`, and `TaskEnd` on the stream, in that order.
- `a_task_started_after_the_bridge_still_reports_its_start` — the subscription happens inside
  `session_events`, so no start event is lost to a race.
- `a_lag_repair_sends_a_start_before_any_state_event` — after a lag, the first event the reader
  sees for any task id is a `TaskStart`. This is the rule a state event alone cannot keep.
- `a_lagged_reader_learns_a_task_it_never_saw_start` — the flood evicts the `TaskStart` itself.
  The reader still ends up told that the task exists, with its command.
- `a_lagged_reader_still_learns_a_task_ended` — the last event for a finished task id is
  `TaskEnd`, however many reports were lost.
- `a_lagged_reader_learns_the_state_of_a_running_task` — a repair reports a running task's
  current progress, so a row is corrected rather than abandoned.
- `a_task_never_goes_back_to_running_after_it_ended` — no progress report reaches a frontend
  after the end this stream already sent. The stream is monotonic per task.
- `a_repair_reports_why_a_task_went_to_the_background` — a repaired `TaskStart` carries the
  task's real `BackgroundReason`. This is the only test that reads the new `TaskSnapshot`
  field, so without it a hardcoded reason would pass.
- `the_bridge_does_not_keep_a_dropped_registry_alive` — drop the last `Arc`, and `next` returns
  `None`. A strong reference inside the bridge would fail this.
- `an_event_sent_before_the_last_arc_drops_still_arrives` — a `broadcast` receiver drains
  before it closes, and the last event of a session is the one that matters most.
- `a_dropped_stream_leaves_the_registry_working` — the bridge spawns no task, so a reader that
  leaves takes its subscription with it. What needs proving is that the next reader still
  works.

### `rho-tui`, in `crates/rho-tui/tests/task_events.rs`

- `a_task_event_from_the_bridge_becomes_a_row` — an event on the real stream becomes a
  `Row::Task`, folded by the real reducer.
- `a_task_row_finishes_while_no_run_is_active` — a `TaskEnd` folded while the interface is
  idle marks the row finished. This is the defect that a run-scoped stream would keep.
- `a_second_start_for_one_task_draws_one_row` — `TaskStart` is idempotent by id, so a repair
  cannot double a row.
- `a_repeated_start_keeps_the_row_it_already_drew` — the no-op keeps the live command and the
  live progress. The row's start time is unobservable from outside the crate, so the clock
  rule rests on the no-op being a plain early return.
- `a_progress_report_for_a_finished_row_is_ignored` — a stale report after a repair cannot
  move a finished row backwards.
- `the_event_loop_reads_the_task_stream` — a source guard. The `select!` arm cannot be
  reached by a unit test, because the loop owns a real terminal, so this reads the loop and
  fails when the arm is gone. The live drive is the other half.
- `a_lagged_interface_ends_with_rows_that_match_the_registry` — the invariant test. It drives
  the real bridge and the real reducer, floods the channel hard enough to evict a `TaskStart`,
  and asserts every task in the registry has exactly one row whose finished flag and state
  word match the registry. It asserts the pairing, not one example. The narrower rho-core
  stream tests above cannot see this class, because they never run the reducer.

### `rho-cli`, in `crates/rho-cli/src/cli.rs` tests

- `the_interface_reads_the_task_event_stream` — the interactive path wires the bridge. This
  guards the call site, which is the exact thing that was missing for six switches.

## 8. Out of scope

- **The JSONL frontend.** `rho-jsonl` maps three task events to `None` on purpose, and
  `SPEC-jsonl-frontend` section 7 says a task event family is an extension with its own wire
  format. A wire format binds every client, so it gets its own spec.

  **This defers a trap, so the trap is written where a contributor meets it.** A review
  found it: wiring `session_events` into the jsonl path would drop every task event at
  runtime, in silence. The match has no wildcard, which catches a new variant, but these
  three arms already return `None`. So `map_event` in `crates/rho-jsonl/src/pump.rs` now
  carries that warning beside the arms.
- **The headless `rho run` path.** It prints a run and exits at `AgentEnd`. A background task
  dies with the session, so a progress line there has no reader to help.
- **A task span in the duration slot.** `F-duration-ladder` owns it. The slot stays blank.
- **Reattaching to a task from a later session.** `SPEC-background-tasks` section 11 rules it
  out already.
- **A task list panel.** Rows only.

## 9. What the contract review changed

The contract went through review before any side started, which is step 3. The review broke
it, and the record is worth keeping.

**The repair sent no `TaskStart`.** Only `on_task_start` creates a row in the reducer, and
`on_task_progress` drops an event that finds no row. So a task whose start was lost to the lag
got a repair that landed on nothing, and it never drew a row. The cure held the disease. The
repair now sends `TaskStart` first, always, and `TaskStart` became idempotent by id.

**`TaskSnapshot` gained a `reason` field.** The repair rebuilds a `TaskStart`, and that event
carries a `BackgroundReason`. A snapshot could not say why a task ran in the background, so the
repair had no honest value to send. The field is additive on the wire the `task` tool writes for
the model, so a reader that ignores it reads what it read before.

**Nothing else in the contract moved.** The review tried to break the `Weak` lifetime and could
not, and it agreed that hiding a lag from a frontend is right as long as the repair is total.

## 10. What the implementation added

**The stream is monotonic per task.** A lag resumes a `broadcast` reader at the oldest retained
event, which is older than the snapshot the repair just read. So progress reports from before
the lag arrive after the repair's `TaskEnd`. The first draft left that to the reducer. It is now
fixed in the bridge as well, because a stale row is a defect for every frontend and not only
for this one. The reducer keeps its own rule as defence in depth, and the two guard different
levels: the bridge bounds what any reader can see, and the reducer bounds what any producer can
do to a finished row.

**A test harness needs `tokio::task::unconstrained`.** tokio gives a task a budget of 128
resource operations per poll, and a channel that spends the budget returns `Pending` although an
event is ready. A `now_or_never` drain therefore stopped after 128 events. A lag test that
floods 264 of them passed while the events that mattered were never read. The interactive loop
has no such limit, because it awaits across many polls, so this is a harness rule and not a
product rule.

## 11. Two guards that passed against the bug they were written for

Both were caught by step 7, and both are the same family: an assertion that matched the wrong
thing.

**The `rho-cli` call-site guard read its own source.** It searched `cli.rs` for
`.with_task_events(...)`, and that literal was written in its own assertion. So it passed with
the call site deleted. It now reads the production half of the file only, and it builds the
needle from parts.

**The live drive counted the model's prose as a row.** `task_rows` matched any line holding the
word `task`, so "Your task id is task-1" counted. Three checks passed against a build with no
bridge at all. It now matches the row grammar, which starts with the label `task `. A second
check in the same script matched `exit 3` inside a command string instead of the state word, and
a third could never match `failed (1)` because a trailing `\b` cannot follow a bracket.

The lesson is not "write better regexes". It is that a guard is code with no test of its own, so
the only proof it works is to break the thing it guards and watch it fail.
