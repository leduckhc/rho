# SPEC-background-tasks — Background tasks and progress reporting

Status: draft for sprint 2.
Owning crates: `rho-core` (the task registry and events), `rho-tools` (the `bash`
tool and the task tools).
Features: F-background-tasks (background tasks), and new features for auto-backgrounding, progress
reporting, and completion signalling.

## 1. The problem, in the user's words

Four complaints, from real use of another harness.

1. **The user must ask for a background run.** A long test suite, a build, or a wait
   loop should not block the conversation. Today the user types "run this in the
   background" every time. The agent should decide.
2. **A script fails in silence.** The parent learns nothing until the end, and
   sometimes not even then.
3. **The parent cannot ask how a job is going.** There is no way to probe progress.
4. **The child cannot tell the parent it finished.** The agent has to guess, so it
   writes `sleep` and poll loops, which waste turns and tokens.

## 2. Design rules

- **The child never blocks the conversation.** A background task runs while the model
  keeps working.
- **No polling loop in the model's head.** The agent must be able to *wait for an
  event*, not sleep and re-check. A `sleep` loop in a transcript is a design failure
  of the harness, not of the model.
- **A finished task always reports.** Success and failure both produce an event. A
  task that ends must never be silent, whatever it wrote to its streams.
- **Progress is optional but cheap.** A script that says nothing still reports start,
  end, exit code, and output. A script that opts in gets structured progress.
- **Backgrounding is reversible.** A foreground command that outruns its timeout is
  adopted into the background rather than killed. Its work is not thrown away.
- **The decision is explainable.** When rho backgrounds a command by itself, it says
  why. A silent policy is impossible to trust or debug.

## 3. Why the child does not need an OS signal

The user asked whether a subprocess can signal the parent when it finishes. It can,
and on Unix it already does: the kernel sends `SIGCHLD`. But rho must not handle that
signal itself.

`tokio::process::Child` owns the handle. Awaiting `child.wait()` in a task *is* the
completion notification, and tokio does the `SIGCHLD` bookkeeping. So the design is:

- one supervisor task per background job awaits `child.wait()`;
- when it resolves, the supervisor pushes a `TaskEnd` event with the exit status;
- the event reaches the agent loop and the frontend through the normal event stream.

This is better than a raw signal for three reasons. It carries the exit code and the
output, not just "something changed". It works the same on Windows. And it cannot be
lost, because the supervisor is already awaiting before the child can exit.

So: **no custom signal handling.** The completion path is an awaited handle.

## 4. Progress protocol

A child reports progress by writing a line to its **stderr** or **stdout** in this
form:

```
RHO_PROGRESS {"percent": 42, "message": "compiling", "done": 6, "total": 10}
```

Every field is optional. `percent` is 0 to 100. `done` and `total` are counts.

Rules:

- A line that starts with `RHO_PROGRESS ` and parses as JSON becomes a
  `TaskProgress` event and is **removed** from the captured output. So progress does
  not pollute the log the model reads.
- A malformed `RHO_PROGRESS` line stays in the output verbatim and raises no error.
  A broken progress line must never fail a job.
- Progress is also **inferred** when a child opts out. rho parses common shapes from
  ordinary output: `6/10`, `42%`, and `[3 of 7]`. Inference never overrides an
  explicit `RHO_PROGRESS` line.
- rho sets `RHO_PROGRESS_FD=1` in the child environment, so a script can detect that
  a progress consumer exists.

Why a sentinel line and not a dedicated file descriptor: a shell script can write a
line with `echo`. Anything needing fd 3 needs a wrapper, and the point is that a
plain script can opt in with one `echo`.

## 5. The decision: foreground or background

`rho-core` decides, and the model may override. The decision is a pure function, so
it is testable.

```rust
/// Why a command runs in the background. Reported to the user, so the choice is
/// never silent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundReason {
    /// The model asked for it.
    ModelRequested,
    /// The command matches a known long-running shape.
    KnownLongRunning,
    /// The command asked for a timeout above the foreground limit.
    LongTimeoutRequested,
    /// The command outran its foreground timeout and was adopted.
    AdoptedOnTimeout,
}

/// The decision for one command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunMode {
    Foreground,
    Background(BackgroundReason),
}

/// Decide how to run `command`. `requested` is the model's explicit choice, if any.
pub fn decide_run_mode(
    command: &str,
    requested: Option<bool>,
    timeout_ms: u64,
    foreground_limit_ms: u64,
) -> RunMode;
```

Rules, in priority order:

1. An explicit `run_in_background` from the model wins, either way. The model knows
   its intent better than a heuristic does.
2. A requested timeout above `foreground_limit_ms`, which defaults to 30000, means
   background. A command that admits it needs a minute should not block a turn.
3. A command matching a known long-running shape means background. The starting list:
   a watch or serve mode (`--watch`, `-w`, `watch`, `serve`, `dev`), a follow
   (`tail -f`, `journalctl -f`, `kubectl logs -f`), a sleep or wait loop
   (`sleep`, `wait-for`, `until`), and a heavy build or test
   (`cargo test`, `cargo build`, `npm test`, `pytest`, `go test`, `make`,
   `gradle`, `docker build`, `terraform apply`).
4. Everything else runs in the foreground.

**The heuristic must be conservative, and the asymmetry is the reason.** Wrongly
backgrounding a fast command costs one extra probe. Wrongly foregrounding a slow one
blocks the conversation for minutes. So a doubtful case goes to the background.

**Rule 3 is a heuristic and will be wrong sometimes.** So it is a denylist of shapes,
not a guess at semantics, and every match is reported with its reason. A user who
disagrees can pass `run_in_background: false`.

## 6. Public API

```rust
/// A handle to one background task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskId(pub String);

/// What a task is doing now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Running,
    /// Finished with this exit code. Zero means success.
    Exited { code: i32 },
    /// Killed by a signal, named where the platform reports one.
    Signaled { signal: Option<String> },
    /// Cancelled by the caller.
    Canceled,
    /// Killed because it passed its timeout.
    TimedOut,
}

impl TaskState {
    /// True when the task will produce no further event.
    pub fn is_final(&self) -> bool;
    /// True when the task finished and reported success.
    pub fn is_success(&self) -> bool;
}

/// One progress report from a task.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskProgress {
    pub percent: Option<u8>,
    pub message: Option<String>,
    pub done: Option<u64>,
    pub total: Option<u64>,
}

/// A snapshot of a task, for a probe.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub id: TaskId,
    pub command: String,
    pub state: TaskState,
    pub progress: TaskProgress,
    /// Output captured so far, bounded. See section 8.
    pub output_tail: String,
    /// Bytes dropped from the head of the output, if the cap was hit.
    pub dropped_bytes: u64,
    pub started_at_unix_ms: u64,
    pub ended_at_unix_ms: Option<u64>,
}

/// The registry every session owns.
pub struct TaskRegistry { /* private */ }

impl TaskRegistry {
    pub fn new(limits: TaskLimits) -> Self;
    /// List every task, newest first.
    pub async fn list(&self) -> Vec<TaskSnapshot>;
    /// Read one task.
    pub async fn get(&self, id: &TaskId) -> Option<TaskSnapshot>;
    /// Ask a task to stop. Kills the process group.
    pub async fn cancel(&self, id: &TaskId) -> Result<(), TaskError>;
    /// Wait until the task reaches a final state, or until the next progress
    /// checkpoint, or until `budget` expires. Returns the snapshot at that moment.
    ///
    /// This is the call that replaces a sleep loop.
    pub async fn wait(&self, id: &TaskId, budget: Duration, until: WaitUntil)
        -> Result<TaskSnapshot, TaskError>;
}

/// What `wait` should wake on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitUntil {
    /// Wake when the task finishes.
    Finished,
    /// Wake on the next progress change, or on finish, whichever comes first.
    NextProgress,
}

/// Caps that keep one session honest. See section 8.
#[derive(Clone, Copy, Debug)]
pub struct TaskLimits {
    pub max_concurrent: usize,
    pub max_output_bytes_per_task: usize,
    pub max_line_bytes: usize,
    pub default_timeout_ms: u64,
    pub max_timeout_ms: u64,
}
```

New agent events, added to `AgentEvent`:

```rust
    /// A background task started.
    TaskStart { id: TaskId, command: String, reason: BackgroundReason },
    /// A task reported progress.
    TaskProgressed { id: TaskId, progress: TaskProgress },
    /// A task reached a final state. Always emitted, success or failure.
    TaskEnd { id: TaskId, state: TaskState, output_tail: String },
```

## 7. The tools the model sees

`bash` gains two arguments:

- `run_in_background: bool` — optional. Overrides the heuristic either way.
- `timeout_ms: u64` — already present. A value above the foreground limit implies
  background.

A background `bash` call returns at once with the `TaskId` and the reason, so the
model can carry on.

One new tool, `task`, with an `action` argument:

| Action | Effect |
| --- | --- |
| `list` | Every task with its state and progress. |
| `get` | One task, with its output tail. |
| `wait` | Block until finish or next progress, within a budget. |
| `cancel` | Kill the process group. |

`task` declares `ToolKind::Read` for `list`, `get`, and `wait`, and
`ToolKind::Execute` for `cancel`. **A single tool cannot vary its kind per call, so
`task` must be two tools**: `task` for the read actions and `task_cancel` for the
mutating one. Otherwise a read-only policy either blocks harmless probes or allows a
kill. See decision D-todo-in-a-green-stage for why the boundary fails closed.

## 8. Limits, because a background task is unsupervised

A foreground command is bounded by a turn. A background task is not, so every
resource needs a cap.

- **Output.** Keep the last `max_output_bytes_per_task`, default 100000, and count the
  dropped bytes. A ring buffer, not truncation at the head, because the tail of a
  failing build is the useful part.
- **One line.** Cap at `max_line_bytes`, default 65536, and split a longer line.
  This is decision D-bash-line-cap. A newline-free stream once drove 805 MB of resident memory.
- **Concurrency.** `max_concurrent`, default 8. Starting a ninth returns
  `TaskError::TooManyTasks`, which names the limit and suggests cancelling one.
- **Time.** `max_timeout_ms`, default 3600000. A task that passes it is killed and
  reports `TimedOut`.
- **Process group.** Every task runs in its own group, and a kill targets the group,
  so no grandchild survives. This already holds for foreground `bash`.
- **Session end.** Dropping the registry kills every task. A session must not leak a
  running process.

## 9. Security

`bash` in the background has the same threat model as `bash` in the foreground, and
one addition: it outlives the turn that started it.

- The approval policy runs **before** the task starts, never after. A background
  `bash` is still `ToolKind::Execute`, so a read-only policy denies it.
- The credential scrub of decision D-bash-scrubs-credentials applies unchanged.
- `task` read actions are safe under a read-only policy. `task_cancel` is not, and it
  is a separate tool for exactly that reason.
- A background task must not be able to outlive the session. Section 8 pins that.
- A progress line is **untrusted input**, because a file the child prints can contain
  anything. So a progress `message` is sanitised before it reaches the TUI, exactly
  like tool output. A control sequence in a progress message must not corrupt the
  display.

## 10. Test cases

Decision, then the test that pins it.

The run-mode decision, all pure and offline:
- `decide_run_mode_respects_an_explicit_background_request`
- `decide_run_mode_respects_an_explicit_foreground_request` — an explicit `false`
  beats every heuristic, even for `cargo test`.
- `decide_run_mode_backgrounds_a_long_timeout`
- `decide_run_mode_backgrounds_a_watch_command`
- `decide_run_mode_backgrounds_a_test_run`
- `decide_run_mode_backgrounds_a_follow_command`
- `decide_run_mode_keeps_a_quick_command_in_the_foreground` — `ls`, `git status`,
  and `echo` must not be backgrounded, or every trivial call costs a probe.
- `decide_run_mode_reports_a_reason_for_every_background_choice`

Progress:
- `progress_line_becomes_an_event_and_leaves_the_output`
- `malformed_progress_line_stays_in_the_output_and_does_not_fail_the_task`
- `progress_is_inferred_from_a_count_shape`
- `explicit_progress_overrides_inference`
- `progress_message_with_an_escape_sequence_is_sanitised`

Completion signalling:
- `task_end_is_emitted_on_success`
- `task_end_is_emitted_on_failure_with_the_exit_code`
- `task_end_is_emitted_when_a_task_writes_nothing` — the silent-failure complaint.
  A task that prints nothing and exits 1 must still report.
- `task_end_is_emitted_on_a_signal`
- `task_end_is_emitted_once_only`

Waiting, with no sleep in any test:
- `wait_returns_when_the_task_finishes`
- `wait_returns_on_the_next_progress_checkpoint`
- `wait_returns_the_snapshot_when_the_budget_expires`
- `wait_on_a_finished_task_returns_at_once`
- `wait_on_an_unknown_task_is_an_error`

Limits:
- `output_keeps_the_tail_and_counts_dropped_bytes`
- `a_line_longer_than_the_cap_is_split`
- `starting_more_than_max_concurrent_tasks_is_an_error`
- `a_task_past_max_timeout_is_killed_and_reports_timed_out`
- `dropping_the_registry_kills_every_task`
- `killing_a_task_kills_its_grandchildren`

Adoption:
- `a_foreground_command_past_its_timeout_is_adopted_not_killed`
- `an_adopted_task_keeps_its_output_from_before_adoption`

Security:
- `a_read_only_policy_denies_a_background_bash`
- `a_read_only_policy_allows_a_task_probe`
- `a_read_only_policy_denies_task_cancel`
- `approval_runs_before_the_task_starts`

## 11. Out of scope for sprint 2

- Persisting a task across a restart of rho. A task dies with its session.
- Reattaching to a task from a different session.
- Streaming a task's output into the transcript live. The model probes instead.
- A dependency graph between tasks.
- Resource quotas beyond the caps in section 8, for example CPU or memory limits per
  task. Those need a cgroup or a job object, and that is a sandbox, which sprint 2
  does not add.
- Inferring progress from a machine-readable test format such as `cargo test --json`.
  Worth doing, and it needs its own spec.
