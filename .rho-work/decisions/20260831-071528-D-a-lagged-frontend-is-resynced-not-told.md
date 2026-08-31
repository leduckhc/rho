# D-a-lagged-frontend-is-resynced-not-told — the bridge repairs a lag itself

**Question:** the task event channel is a `broadcast` with room for 256 events. A reader that
falls behind gets `RecvError::Lagged` and loses the events in between. One of the lost events
can be `TaskEnd`. What does a frontend do about it?

## Why a lost event is not a display detail

`TaskRegistry` says a lagged subscriber "loses only display events", and that is true of the
registry. It is false of the screen. The reducer folds `TaskEnd` to mark a row finished, so a
lost `TaskEnd` leaves the row reading `running` for the rest of the session. The task is over.
The screen says it is not. A user reads the screen.

So a lag is not a missed frame. It is a row that stays wrong until rho exits.

**Decision: the bridge repairs the lag, and the frontend never learns one happened.** On a
`Lagged`, the bridge reads the registry and sends, for every task it holds:

1. `TaskStart`, always, and first,
2. then `TaskEnd` for a task in a final state,
3. or `TaskProgressed` for a running task that has progress to report.

The frontend folds those with the same reducer it already has. After the repair the screen
matches the registry, which is the only source of truth.

## The repair must send `TaskStart`, and the first draft did not

A contract review broke the first draft of this decision. The hole is worth recording.

The draft sent the state event alone, because the reader "already has the row". It does not
always have the row. Read the reducer:

```rust
fn on_task_progress(&mut self, id: &TaskId, progress: &TaskProgress) {
    if let Some(Row::Task { progress: row_progress, .. }) = self.task_row_mut(&id.0) { ... }
    // No row for this id? The event is dropped, in silence.
}
```

Only `on_task_start` creates a row. So a task whose `TaskStart` was itself lost to the lag got
a `TaskProgressed` that landed on nothing. It never drew a row again. The repair was a no-op
for the one task that needed it most. That is the defect this decision exists to prevent,
rebuilt inside the cure.

A state event cannot create the row on its own, and that is not an oversight. `TaskProgressed`
carries an id and a progress. `TaskEnd` carries an id and a state. Neither carries the command,
and a row with no command is not a row a user can read. `TaskStart` carries the command. So the
repair sends `TaskStart`.

## Which makes `TaskStart` idempotent, by id

`on_task_start` pushes a row unconditionally today. A repair for a task the reader already
draws would push a second row for one id. So the reducer changes: **a `TaskStart` for an id
that already has a row changes nothing.**

It is a no-op, and not an update, on purpose. The row already holds the live command. The row's
start time is what the duration slot measures. An update would reset that clock, and a
four-minute build would look new.

## Why the repair lives here

**One place, one test.** rho has two frontends today and expects more. Lag handling in the
bridge is written once and proved once. Lag handling in each frontend is a rule that a new
frontend author must read and obey, and this project has five shipped defects that came from
exactly that shape.

**The contract stays small.** `next` returns `Option<AgentEvent>`. There is no lag variant, so
there is no branch a caller can leave empty. A `SessionEvent::Lagged` variant would be a
fail-open default: the compiler accepts an empty arm, and the row stays stale.

## What this rules out

**Reporting the lag to the frontend.** See above. It moves a correctness duty onto every
reader.

**Growing the channel until a lag cannot happen.** A bound that is never reached is a bound
that was never needed. 256 events of buffer plus a repair is honest. Ten thousand events of
buffer is a hope.

**Silently dropping the lag.** That is today's behaviour by default, and it is the defect.

**Replaying the whole event history.** The bridge sends current state, not the events that
were lost. State is what the row draws. The lost intermediate percentages have no reader.

## Two orderings this accepts

A `Lagged` resumes the reader at the oldest event the channel still holds. That event can be
older than the snapshot the repair just read. So a stale event can arrive after a repair.

**A stale `TaskProgressed` for a running task.** It rewrites the percentage with an older one.
The next real report corrects it, because progress is last-writer-wins by design. A row that
is one report behind for a moment is not a defect.

**A stale `TaskProgressed` for a task the repair already finished.** This one is not harmless.
No later report will ever correct it, so a finished row would draw a mid-run percentage for the
rest of the session. The reducer gains one rule: **a progress report for a finished row is
ignored.** The screen then only moves forward.

A duplicate `TaskEnd` needs no rule. `on_task_end` writes the same three fields from the same
final state, so folding it twice gives the same row.

## What a killed task does not send

Dropping the registry kills every running task, and it broadcasts nothing. `Drop for
TaskRegistry` calls each killer and returns. So no `TaskEnd` arrives for a task that a teardown
killed.

That is safe only because the bridge dies at the same moment. No reader is left holding a row
it can never finish. It is written here because it looks like a missing event, and the next
reader of this code will look for it.
