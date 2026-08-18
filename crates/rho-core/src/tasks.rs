//! Background tasks and progress reporting.
//!
//! This module owns the task state model and the task registry. A long command
//! becomes a task the model can list, probe, wait on, or cancel. The registry
//! stores every task, bounds its resources, and wakes a waiter on a real event.
//!
//! The registry never polls. A waiter wakes on a [`tokio::sync::Notify`] signal,
//! which fires on a progress change or on a final state. See `SPEC-background-tasks` section 3.
//!
//! This module spawns no process. The caller owns the child. The caller feeds
//! output, progress, and the final state through a [`TaskHandle`]. So the module
//! stays free of shell and platform detail, and `rho-core` keeps no terminal or
//! HTTP dependency. `rho-tools` owns the child and the process-group kill.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::AgentEvent;

/// A handle to one background task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskId(pub String);

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a task is doing now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Running,
    /// Finished with this exit code. Zero means success.
    Exited {
        code: i32,
    },
    /// Killed by a signal, named where the platform reports one.
    Signaled {
        signal: Option<String>,
    },
    /// Cancelled by the caller.
    Canceled,
    /// Killed because it passed its timeout.
    TimedOut,
}

impl TaskState {
    /// True when the task will produce no further event.
    pub fn is_final(&self) -> bool {
        !matches!(self, TaskState::Running)
    }

    /// True when the task finished and reported success.
    pub fn is_success(&self) -> bool {
        matches!(self, TaskState::Exited { code: 0 })
    }
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

impl Default for TaskLimits {
    fn default() -> Self {
        Self {
            max_concurrent: 8,
            max_output_bytes_per_task: 100_000,
            max_line_bytes: 65_536,
            default_timeout_ms: 120_000,
            max_timeout_ms: 3_600_000,
        }
    }
}

/// An error from the task registry. Each message tells the user what to do.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TaskError {
    /// The session already runs the most tasks it may. The message names the
    /// limit and tells the user to cancel a task first.
    #[error(
        "the session already runs {max} background tasks, which is the limit. \
         Cancel a task, then start the new one."
    )]
    TooManyTasks { max: usize },
    /// The id names no task. The message tells the user to list the tasks.
    #[error("no task has the id {0}. List the tasks to see the valid ids.")]
    UnknownTask(TaskId),
}

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

/// The default foreground timeout limit, in milliseconds. A requested timeout
/// above this implies a background run.
pub const DEFAULT_FOREGROUND_LIMIT_MS: u64 = 30_000;

/// Decide how to run `command`. `requested` is the model's explicit choice, if any.
///
/// The rules run in priority order. See `SPEC-background-tasks` section 5.
///
/// 1. An explicit request from the model wins, either way.
/// 2. A requested timeout above `foreground_limit_ms` means background.
/// 3. A command that matches a known long-running shape means background.
/// 4. Everything else runs in the foreground.
///
/// The heuristic is conservative on purpose. A wrongly backgrounded fast command
/// costs one probe. A wrongly foregrounded slow command blocks the conversation.
/// So a doubtful case goes to the background.
pub fn decide_run_mode(
    command: &str,
    requested: Option<bool>,
    timeout_ms: u64,
    foreground_limit_ms: u64,
) -> RunMode {
    // Rule 1: an explicit choice from the model wins over every heuristic.
    match requested {
        Some(true) => return RunMode::Background(BackgroundReason::ModelRequested),
        Some(false) => return RunMode::Foreground,
        None => {}
    }

    // Rule 2: a command that admits it needs a long timeout must not block a turn.
    if timeout_ms > foreground_limit_ms {
        return RunMode::Background(BackgroundReason::LongTimeoutRequested);
    }

    // Rule 3: a command that matches a known long-running shape goes to the
    // background. The match is a denylist of shapes, not a guess at semantics.
    if matches_long_running(command) {
        return RunMode::Background(BackgroundReason::KnownLongRunning);
    }

    // Rule 4: everything else runs in the foreground.
    RunMode::Foreground
}

/// True when a command matches a known long-running shape.
fn matches_long_running(command: &str) -> bool {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    let has = |needle: &str| tokens.contains(&needle);
    let has_pair = |first: &str, second: &str| {
        tokens
            .windows(2)
            .any(|pair| pair[0] == first && pair[1] == second)
    };

    // A watch, serve, or dev mode runs until stopped.
    if has("--watch") || has("-w") || has("watch") || has("serve") || has("dev") {
        return true;
    }
    // A follow reads a stream forever. Match the follow flag with a follow tool,
    // so a bare `-f` on an unrelated command does not trip the rule.
    if has("-f") && (has("tail") || has("journalctl") || has("kubectl")) {
        return true;
    }
    // A sleep or a wait loop blocks by design.
    if has("sleep") || has("wait-for") || has("until") {
        return true;
    }
    // A heavy build or test run takes minutes.
    if has_pair("cargo", "test")
        || has_pair("cargo", "build")
        || has_pair("npm", "test")
        || has_pair("go", "test")
        || has_pair("docker", "build")
        || has_pair("terraform", "apply")
        || has("pytest")
        || has("make")
        || has("gradle")
    {
        return true;
    }
    false
}

/// The event channel buffer size. It bounds how many task events wait for a
/// subscriber. A subscriber that lags loses only display events; the task state
/// stays correct, so `get` and `wait` never miss a completion.
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// The shared state of one task. The registry and every handle share it.
struct TaskShared {
    id: TaskId,
    command: String,
    started_at_unix_ms: u64,
    /// Notified on every progress change and on the final state. A waiter wakes
    /// on this, so the registry never polls.
    notify: Notify,
    inner: Mutex<TaskInner>,
}

/// The mutable state of one task.
struct TaskInner {
    state: TaskState,
    progress: TaskProgress,
    /// The output ring buffer. It keeps the tail, so the useful end of a failing
    /// build survives.
    output: VecDeque<u8>,
    dropped_bytes: u64,
    ended_at_unix_ms: Option<u64>,
    /// A monotonic counter. It rises on every progress change and on finish, so a
    /// waiter can tell a new event from an old one with no lost wake.
    revision: u64,
    /// The caller set this to kill the process group. The registry calls it on
    /// cancel and on drop.
    killer: Option<Box<dyn Fn() + Send + Sync>>,
    /// True when the caller asked to cancel. The supervisor reads it to report
    /// `Canceled` instead of a raw signal.
    cancel_requested: bool,
}

impl TaskShared {
    fn snapshot(&self) -> TaskSnapshot {
        let inner = self.inner.lock().expect("the task lock is not poisoned");
        TaskSnapshot {
            id: self.id.clone(),
            command: self.command.clone(),
            state: inner.state.clone(),
            progress: inner.progress.clone(),
            output_tail: output_to_string(&inner.output),
            dropped_bytes: inner.dropped_bytes,
            started_at_unix_ms: self.started_at_unix_ms,
            ended_at_unix_ms: inner.ended_at_unix_ms,
        }
    }

    fn is_final(&self) -> bool {
        self.inner
            .lock()
            .expect("the task lock is not poisoned")
            .state
            .is_final()
    }
}

/// The registry every session owns.
///
/// Dropping the registry kills every running task, so a session never leaks a
/// process. See section 8.
pub struct TaskRegistry {
    limits: TaskLimits,
    tasks: Mutex<Vec<Arc<TaskShared>>>,
    events: tokio::sync::broadcast::Sender<AgentEvent>,
    next_id: AtomicU64,
}

impl TaskRegistry {
    pub fn new(limits: TaskLimits) -> Self {
        let (events, _) = tokio::sync::broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            limits,
            tasks: Mutex::new(Vec::new()),
            events,
            next_id: AtomicU64::new(1),
        }
    }

    /// The limits this registry enforces.
    pub fn limits(&self) -> &TaskLimits {
        &self.limits
    }

    /// Subscribe to the task event stream. A subscriber sees `TaskStart`,
    /// `TaskProgressed`, and `TaskEnd` events. Subscribe before you start a task,
    /// so no start event is missed.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<AgentEvent> {
        self.events.subscribe()
    }

    /// Start a new task and return a handle to feed it. The caller owns the
    /// child. It calls `push_output`, `report_progress`, and `finish` on the
    /// handle. It sets a killer with `set_killer`.
    ///
    /// This fails with `TooManyTasks` when the session already runs the most
    /// tasks it may.
    pub fn start(
        &self,
        command: impl Into<String>,
        reason: BackgroundReason,
    ) -> Result<TaskHandle, TaskError> {
        let command = command.into();
        let mut tasks = self
            .tasks
            .lock()
            .expect("the registry lock is not poisoned");
        let active = tasks.iter().filter(|task| !task.is_final()).count();
        if active >= self.limits.max_concurrent {
            return Err(TaskError::TooManyTasks {
                max: self.limits.max_concurrent,
            });
        }
        let id = TaskId(format!(
            "task-{}",
            self.next_id.fetch_add(1, Ordering::SeqCst)
        ));
        let shared = Arc::new(TaskShared {
            id: id.clone(),
            command: command.clone(),
            started_at_unix_ms: now_unix_ms(),
            notify: Notify::new(),
            inner: Mutex::new(TaskInner {
                state: TaskState::Running,
                progress: TaskProgress::default(),
                output: VecDeque::new(),
                dropped_bytes: 0,
                ended_at_unix_ms: None,
                revision: 0,
                killer: None,
                cancel_requested: false,
            }),
        });
        tasks.push(Arc::clone(&shared));
        // A lagged subscriber is not an error here. The task state is the source
        // of truth, so a missed start event does not lose the task.
        let _ = self.events.send(AgentEvent::TaskStart {
            id: id.clone(),
            command,
            reason,
        });
        Ok(TaskHandle {
            shared,
            max_output_bytes: self.limits.max_output_bytes_per_task,
            events: self.events.clone(),
        })
    }

    /// List every task, newest first.
    pub async fn list(&self) -> Vec<TaskSnapshot> {
        let tasks = self
            .tasks
            .lock()
            .expect("the registry lock is not poisoned");
        tasks.iter().rev().map(|task| task.snapshot()).collect()
    }

    /// Read one task.
    pub async fn get(&self, id: &TaskId) -> Option<TaskSnapshot> {
        self.find(id).map(|task| task.snapshot())
    }

    /// Ask a task to stop. Kills the process group. A cancel on an unknown task
    /// is an error. A cancel on a finished task is a no-op.
    pub async fn cancel(&self, id: &TaskId) -> Result<(), TaskError> {
        let shared = self
            .find(id)
            .ok_or_else(|| TaskError::UnknownTask(id.clone()))?;
        let mut inner = shared.inner.lock().expect("the task lock is not poisoned");
        if inner.state.is_final() {
            return Ok(());
        }
        inner.cancel_requested = true;
        if let Some(killer) = inner.killer.as_ref() {
            killer();
        }
        Ok(())
    }

    /// Wait until the task reaches a final state, or until the next progress
    /// checkpoint, or until `budget` expires. Return the snapshot at that moment.
    ///
    /// This is the call that replaces a sleep loop. It wakes on a real event, not
    /// on a timer. The budget deadline is the only timer, and it bounds the wait;
    /// it does not detect completion.
    pub async fn wait(
        &self,
        id: &TaskId,
        budget: Duration,
        until: WaitUntil,
    ) -> Result<TaskSnapshot, TaskError> {
        let shared = self
            .find(id)
            .ok_or_else(|| TaskError::UnknownTask(id.clone()))?;
        let start_revision = shared
            .inner
            .lock()
            .expect("the task lock is not poisoned")
            .revision;
        let deadline = tokio::time::Instant::now() + budget;

        loop {
            // Create the wake future before the state read. Creating it takes a
            // snapshot of the notify state. A notify after this line updates that
            // snapshot, so the later await returns at once. This removes the race
            // that would lose a wake between the read and the await.
            let notified = shared.notify.notified();
            {
                let inner = shared.inner.lock().expect("the task lock is not poisoned");
                if inner.state.is_final() {
                    drop(inner);
                    return Ok(shared.snapshot());
                }
                if until == WaitUntil::NextProgress && inner.revision != start_revision {
                    drop(inner);
                    return Ok(shared.snapshot());
                }
            }

            tokio::select! {
                biased;
                _ = notified => continue,
                _ = tokio::time::sleep_until(deadline) => {
                    return Ok(shared.snapshot());
                }
            }
        }
    }

    fn find(&self, id: &TaskId) -> Option<Arc<TaskShared>> {
        let tasks = self
            .tasks
            .lock()
            .expect("the registry lock is not poisoned");
        tasks.iter().find(|task| &task.id == id).cloned()
    }
}

impl Drop for TaskRegistry {
    /// Kill every running task. A session must not leak a running process.
    fn drop(&mut self) {
        let tasks = match self.tasks.lock() {
            Ok(tasks) => tasks,
            Err(poisoned) => poisoned.into_inner(),
        };
        for task in tasks.iter() {
            let inner = match task.inner.lock() {
                Ok(inner) => inner,
                Err(poisoned) => poisoned.into_inner(),
            };
            if !inner.state.is_final()
                && let Some(killer) = inner.killer.as_ref()
            {
                killer();
            }
        }
    }
}

/// A handle the caller uses to feed one task. The caller owns the child process.
pub struct TaskHandle {
    shared: Arc<TaskShared>,
    max_output_bytes: usize,
    events: tokio::sync::broadcast::Sender<AgentEvent>,
}

impl std::fmt::Debug for TaskHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskHandle")
            .field("id", &self.shared.id)
            .finish_non_exhaustive()
    }
}

impl TaskHandle {
    /// The id of the task this handle feeds.
    pub fn id(&self) -> TaskId {
        self.shared.id.clone()
    }

    /// True when the caller asked to cancel this task. The supervisor reads it,
    /// so it reports `Canceled` rather than a raw signal.
    pub fn cancel_requested(&self) -> bool {
        self.shared
            .inner
            .lock()
            .expect("the task lock is not poisoned")
            .cancel_requested
    }

    /// Set the killer for this task. The registry calls it on cancel and on drop.
    pub fn set_killer(&self, killer: impl Fn() + Send + Sync + 'static) {
        self.shared
            .inner
            .lock()
            .expect("the task lock is not poisoned")
            .killer = Some(Box::new(killer));
    }

    /// Append one output line to the task. The line has no trailing newline. The
    /// buffer keeps the tail, up to the byte cap, and counts the dropped head.
    pub fn push_output(&self, line: &str) {
        let mut inner = self
            .shared
            .inner
            .lock()
            .expect("the task lock is not poisoned");
        for byte in line.bytes().chain(std::iter::once(b'\n')) {
            inner.output.push_back(byte);
        }
        while inner.output.len() > self.max_output_bytes {
            inner.output.pop_front();
            inner.dropped_bytes += 1;
        }
    }

    /// Report a progress change. Bump the revision, wake every waiter, and emit a
    /// `TaskProgressed` event.
    pub fn report_progress(&self, progress: TaskProgress) {
        {
            let mut inner = self
                .shared
                .inner
                .lock()
                .expect("the task lock is not poisoned");
            inner.progress = progress.clone();
            inner.revision += 1;
        }
        self.shared.notify.notify_waiters();
        let _ = self.events.send(AgentEvent::TaskProgressed {
            id: self.shared.id.clone(),
            progress,
        });
    }

    /// Record the final state. Emit a `TaskEnd` event, always, whatever the task
    /// wrote. This call is idempotent: a second call after a final state does
    /// nothing, so the completion event fires once only.
    pub fn finish(&self, state: TaskState) {
        let output_tail;
        {
            let mut inner = self
                .shared
                .inner
                .lock()
                .expect("the task lock is not poisoned");
            if inner.state.is_final() {
                // The task already finished. Do not fire a second end event.
                return;
            }
            inner.state = state.clone();
            inner.ended_at_unix_ms = Some(now_unix_ms());
            inner.revision += 1;
            output_tail = output_to_string(&inner.output);
        }
        self.shared.notify.notify_waiters();
        let _ = self.events.send(AgentEvent::TaskEnd {
            id: self.shared.id.clone(),
            state,
            output_tail,
        });
    }
}

/// Render the output ring buffer as a lossy UTF-8 string. The buffer may be in
/// two segments, so join both.
fn output_to_string(output: &VecDeque<u8>) -> String {
    let (front, back) = output.as_slices();
    let mut bytes = Vec::with_capacity(front.len() + back.len());
    bytes.extend_from_slice(front);
    bytes.extend_from_slice(back);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The current unix time in milliseconds. A clock error yields zero, which is
/// harmless for a display timestamp.
fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|delta| delta.as_millis() as u64)
        .unwrap_or(0)
}
