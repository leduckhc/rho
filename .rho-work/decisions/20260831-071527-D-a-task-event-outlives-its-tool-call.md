# D-a-task-event-outlives-its-tool-call — the bridge is session-lifetime

**Question:** a background task reports progress, and no frontend ever sees it. The registry
broadcasts `TaskStart`, `TaskProgressed`, and `TaskEnd`. Where does the subscription live?

## The two channels that exist today, and why neither works

**`ToolContext::agent_events` lives for one tool call.** `Driver::dispatch_one` builds the
channel, forwards from it while `tool.execute` is pending, and drops it when the call returns.
A background task returns its id at once and runs for minutes. So every event it sends after
that arrives at a closed channel.

**`AgentEvents` lives for one run.** `Session::prompt` builds the stream, and the frontend
drops it on `AgentEnd`. A task that a turn started keeps running while the user reads the
answer and types the next prompt. A task that ends in that gap would send `TaskEnd` to nobody.

**Decision: the task registry owns the bridge, and the frontend reads it for as long as it
owns the screen.** `TaskRegistry::session_events` returns a `SessionEvents` stream. The
interface holds that stream for the life of the interface, not for the life of a run.

```rust
impl TaskRegistry {
    pub fn session_events(self: &Arc<Self>) -> SessionEvents;
}

impl SessionEvents {
    pub async fn next(&mut self) -> Option<AgentEvent>;
}
```

## Why the registry and not the session

`Session` knows nothing about tasks, and it must stay that way. `rho-core` builds the session
from a provider, a tool registry, and a config. The task registry reaches the tools, not the
session. Adding a task registry to `SessionConfig` would put a field in a shared contract for
one caller, which is the mistake `D-no-four-argument-session-new` records.

The registry already publishes the events. So the bridge is a second reader of a channel that
exists, not a new path through three crates.

## What this rules out

**Forwarding task events onto the run stream.** It reads well, and it loses every event that
arrives while rho is idle. A row would then say `running` after the build finished, which is
worse than no row. The user would trust it.

**A frontend that subscribes to the broadcast channel itself.** The raw
`tokio::sync::broadcast::Receiver` makes every frontend handle lag, and a frontend that
forgets leaves a stale row. See `D-a-lagged-frontend-is-resynced-not-told`.

**A polling loop over `TaskRegistry::list`.** It would draw a task late by up to one tick, and
it would wake a quiet terminal on a timer. The registry already pushes.

## The bridge holds a weak reference

Dropping the registry kills every running task. That is section 8 of
`SPEC-background-tasks`, and it is how a session never leaks a process. So the bridge task
holds a `Weak<TaskRegistry>` and never an `Arc`. A strong reference would tie the life of
every child process to a frontend stream, and a forgotten stream would keep a killed
session's processes alive.

The bridge ends when the reader drops the stream, or when the registry is dropped. `next`
then returns `None`.
