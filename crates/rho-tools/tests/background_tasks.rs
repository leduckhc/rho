//! Integration tests for background tasks, driven with real child processes.
//!
//! No test sleeps to synchronise. A timeout test uses the feature's own timer,
//! exactly like the existing `bash` timeout test. A test that observes an
//! external process polls with `yield_now` against a deadline, never a sleep,
//! matching the existing cancel test.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::Harness;
use rho_core::{
    AgentEvent, TaskId, TaskLimits, TaskProgress, TaskRegistry, TaskState, Tool, WaitUntil,
};
use rho_tools::BashTool;
use tokio::sync::broadcast::Receiver;
use tokio::time::{Instant, timeout};

/// A generous bound. A test that hangs fails here instead of blocking the suite.
const BOUND: Duration = Duration::from_secs(10);

fn registry_with(limits: TaskLimits) -> Arc<TaskRegistry> {
    Arc::new(TaskRegistry::new(limits))
}

/// Run one command in the background and return its task id at once.
async fn start_background(registry: &Arc<TaskRegistry>, harness: &mut Harness, command: &str) {
    let tool = BashTool::with_tasks(Arc::clone(registry));
    tool.execute(
        serde_json::json!({ "command": command, "run_in_background": true }),
        harness.ctx(),
    )
    .await
    .expect("a background start returns at once");
}

/// Wait for the next `TaskEnd` event, within the bound.
async fn next_end(events: &mut Receiver<AgentEvent>) -> (TaskId, TaskState, String) {
    loop {
        let event = timeout(BOUND, events.recv())
            .await
            .expect("a task end must arrive")
            .expect("the event channel stays open");
        if let AgentEvent::TaskEnd {
            id,
            state,
            output_tail,
        } = event
        {
            return (id, state, output_tail);
        }
    }
}

/// Collect the progress events until the task ends. Return them with the final
/// state and the final output tail.
async fn progress_until_end(
    events: &mut Receiver<AgentEvent>,
) -> (Vec<TaskProgress>, TaskState, String) {
    let mut progress = Vec::new();
    loop {
        let event = timeout(BOUND, events.recv())
            .await
            .expect("a task event must arrive")
            .expect("the event channel stays open");
        match event {
            AgentEvent::TaskProgressed { progress: p, .. } => progress.push(p),
            AgentEvent::TaskEnd {
                state, output_tail, ..
            } => return (progress, state, output_tail),
            _ => {}
        }
    }
}

// --- Completion signalling. ---

#[tokio::test]
async fn task_end_is_emitted_on_success() {
    let registry = registry_with(TaskLimits::default());
    let mut events = registry.subscribe();
    let mut h = Harness::new();
    start_background(&registry, &mut h, "echo done; exit 0").await;

    let (_, state, output) = next_end(&mut events).await;
    assert_eq!(state, TaskState::Exited { code: 0 });
    assert!(output.contains("done"), "output kept: {output:?}");
}

#[tokio::test]
async fn task_end_is_emitted_on_failure_with_the_exit_code() {
    let registry = registry_with(TaskLimits::default());
    let mut events = registry.subscribe();
    let mut h = Harness::new();
    start_background(&registry, &mut h, "echo oops; exit 7").await;

    let (_, state, _) = next_end(&mut events).await;
    assert_eq!(state, TaskState::Exited { code: 7 });
}

#[tokio::test]
async fn task_end_is_emitted_when_a_task_writes_nothing() {
    // The silent-failure complaint. A task that prints nothing and exits 1 must
    // still report. This is the test that matters most to the owner.
    let registry = registry_with(TaskLimits::default());
    let mut events = registry.subscribe();
    let mut h = Harness::new();
    start_background(&registry, &mut h, "exit 1").await;

    let (_, state, output) = next_end(&mut events).await;
    assert_eq!(state, TaskState::Exited { code: 1 });
    assert!(output.is_empty(), "a silent task still reports: {output:?}");
}

#[tokio::test]
async fn task_end_is_emitted_on_a_signal() {
    let registry = registry_with(TaskLimits::default());
    let mut events = registry.subscribe();
    let mut h = Harness::new();
    // The shell kills itself with SIGKILL, so the child ends on a signal.
    start_background(&registry, &mut h, "kill -9 $$").await;

    let (_, state, _) = next_end(&mut events).await;
    assert!(
        matches!(state, TaskState::Signaled { .. }),
        "a signal must report Signaled, got {state:?}"
    );
}

// --- Progress protocol. ---

#[tokio::test]
async fn progress_line_becomes_an_event_and_leaves_the_output() {
    let registry = registry_with(TaskLimits::default());
    let mut events = registry.subscribe();
    let mut h = Harness::new();
    start_background(
        &registry,
        &mut h,
        r#"echo 'RHO_PROGRESS {"percent": 42, "message": "compiling"}'; echo ordinary"#,
    )
    .await;

    let (progress, state, output) = progress_until_end(&mut events).await;
    assert!(state.is_success());
    assert!(
        progress.iter().any(|p| p.percent == Some(42)),
        "a progress event fired: {progress:?}"
    );
    assert!(
        !output.contains("RHO_PROGRESS"),
        "the progress line left the output: {output:?}"
    );
    assert!(
        output.contains("ordinary"),
        "ordinary output stays: {output:?}"
    );
}

#[tokio::test]
async fn malformed_progress_line_stays_in_the_output_and_does_not_fail_the_task() {
    let registry = registry_with(TaskLimits::default());
    let mut events = registry.subscribe();
    let mut h = Harness::new();
    start_background(&registry, &mut h, "echo 'RHO_PROGRESS not json'; exit 0").await;

    let (_, state, output) = next_end(&mut events).await;
    assert_eq!(
        state,
        TaskState::Exited { code: 0 },
        "a broken line fails nothing"
    );
    assert!(
        output.contains("RHO_PROGRESS not json"),
        "the malformed line stays verbatim: {output:?}"
    );
}

#[tokio::test]
async fn progress_is_inferred_from_a_count_shape() {
    let registry = registry_with(TaskLimits::default());
    let mut events = registry.subscribe();
    let mut h = Harness::new();
    start_background(&registry, &mut h, "echo 'built 6/10 crates'").await;

    let (progress, _, _) = progress_until_end(&mut events).await;
    assert!(
        progress
            .iter()
            .any(|p| p.done == Some(6) && p.total == Some(10)),
        "a count shape is inferred: {progress:?}"
    );
}

#[tokio::test]
async fn explicit_progress_overrides_inference() {
    let registry = registry_with(TaskLimits::default());
    let mut events = registry.subscribe();
    let mut h = Harness::new();
    // The explicit line sets percent 50. The later count line must not override
    // it once an explicit line was seen.
    start_background(
        &registry,
        &mut h,
        r#"echo 'RHO_PROGRESS {"percent": 50}'; echo '1/2 done'; exit 0"#,
    )
    .await;

    let (progress, _, _) = progress_until_end(&mut events).await;
    assert!(
        progress.iter().any(|p| p.percent == Some(50)),
        "the explicit progress fired: {progress:?}"
    );
    assert!(
        progress
            .iter()
            .all(|p| p.done.is_none() && p.total.is_none()),
        "inference must not override the explicit progress: {progress:?}"
    );
}

// --- Limits. ---

#[tokio::test]
async fn a_task_past_max_timeout_is_killed_and_reports_timed_out() {
    // The registry caps the timeout. The command sleeps far past the cap, so the
    // supervisor kills it and reports TimedOut. The 100 ms cap is the feature's
    // own timer, not a synchronisation sleep.
    let limits = TaskLimits {
        default_timeout_ms: 100,
        max_timeout_ms: 100,
        ..TaskLimits::default()
    };
    let registry = registry_with(limits);
    let mut events = registry.subscribe();
    let mut h = Harness::new();
    start_background(&registry, &mut h, "sleep 30").await;

    let (_, state, _) = next_end(&mut events).await;
    assert_eq!(state, TaskState::TimedOut);
}

#[tokio::test]
async fn dropping_the_registry_kills_every_task() {
    let registry = registry_with(TaskLimits::default());
    let mut events = registry.subscribe();
    let mut h = Harness::new();
    start_background(&registry, &mut h, "sleep 30").await;

    // Wait for the task to start, then drop the last registry handle. Drop must
    // kill the running task, so a session leaks no process.
    let _ = timeout(BOUND, events.recv()).await; // the TaskStart event
    drop(registry);

    let (_, state, _) = next_end(&mut events).await;
    assert!(
        matches!(state, TaskState::Signaled { .. }),
        "the dropped registry killed the task, got {state:?}"
    );
}

#[tokio::test]
async fn killing_a_task_kills_its_grandchildren() {
    let registry = registry_with(TaskLimits::default());
    let mut h = Harness::new();
    // The shell starts a grandchild and prints its pid, then waits.
    start_background(&registry, &mut h, "sleep 30 & echo GC=$!; wait").await;

    // Find the task id and the grandchild pid from the output.
    let deadline = Instant::now() + BOUND;
    let (id, gc_pid) = loop {
        if let Some(snapshot) = registry.list().await.first()
            && let Some(pid) = grandchild_pid(&snapshot.output_tail)
        {
            break (snapshot.id.clone(), pid);
        }
        assert!(
            Instant::now() < deadline,
            "the grandchild never reported its pid"
        );
        tokio::task::yield_now().await;
    };

    registry.cancel(&id).await.unwrap();

    // The grandchild must die, because the kill targets the whole process group.
    let deadline = Instant::now() + BOUND;
    loop {
        if !process_alive(gc_pid) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the grandchild survived the kill"
        );
        tokio::task::yield_now().await;
    }
}

// --- Adoption. ---

#[tokio::test]
async fn a_foreground_command_past_its_timeout_is_adopted_not_killed() {
    let registry = registry_with(TaskLimits::default());
    let mut events = registry.subscribe();
    let mut h = Harness::new();
    let tool = BashTool::with_tasks(Arc::clone(&registry));
    // A short foreground timeout against a long command. It is adopted, not
    // killed, so the call returns a task id rather than a timeout error.
    let out = tool
        .execute(
            serde_json::json!({ "command": "sleep 30", "timeout_ms": 50, "run_in_background": false }),
            h.ctx(),
        )
        .await
        .expect("an adopted command returns a task, not an error");
    let text: String = out
        .content
        .iter()
        .filter_map(|b| match b {
            rho_core::ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(
        text.contains("background task"),
        "adoption returns a task: {text}"
    );

    // The adopted task exists and runs.
    let start = timeout(BOUND, events.recv()).await;
    assert!(
        matches!(start, Ok(Ok(AgentEvent::TaskStart { .. }))),
        "a task started"
    );
    let tasks = registry.list().await;
    assert_eq!(tasks.len(), 1);
    registry.cancel(&tasks[0].id).await.unwrap();
}

#[tokio::test]
async fn an_adopted_task_keeps_its_output_from_before_adoption() {
    let registry = registry_with(TaskLimits::default());
    let mut h = Harness::new();
    let tool = BashTool::with_tasks(Arc::clone(&registry));
    // The command prints a marker, then blocks past the foreground timeout.
    let out = tool
        .execute(
            serde_json::json!({ "command": "echo MARKER_BEFORE_ADOPT; sleep 30", "timeout_ms": 300 }),
            h.ctx(),
        )
        .await
        .unwrap();
    assert!(
        out.content.iter().any(|b| matches!(b, rho_core::ContentBlock::Text { text } if text.contains("background task"))),
        "the command was adopted"
    );

    // The output from before adoption is kept in the task.
    let deadline = Instant::now() + BOUND;
    let id = registry.list().await[0].id.clone();
    loop {
        let snapshot = registry.get(&id).await.unwrap();
        if snapshot.output_tail.contains("MARKER_BEFORE_ADOPT") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the pre-adoption output was lost"
        );
        tokio::task::yield_now().await;
    }
    registry.cancel(&id).await.unwrap();
}

// --- The wait tool path, no sleep. ---

#[tokio::test]
async fn task_tool_wait_returns_when_the_task_finishes() {
    let registry = registry_with(TaskLimits::default());
    let mut h = Harness::new();
    start_background(&registry, &mut h, "echo hi; exit 0").await;
    let id = registry.list().await[0].id.clone();

    let snapshot = registry
        .wait(&id, BOUND, WaitUntil::Finished)
        .await
        .unwrap();
    assert!(snapshot.state.is_final());
}

/// Parse a `GC=<pid>` marker from output.
fn grandchild_pid(output: &str) -> Option<i32> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("GC="))
        .and_then(|pid| pid.trim().parse().ok())
}

/// True when the process is still alive. `kill(pid, 0)` sends no signal; it only
/// checks that the process exists.
fn process_alive(pid: i32) -> bool {
    // Safety: `kill` with signal 0 is a plain existence check. It changes no
    // process state.
    unsafe { libc::kill(pid, 0) == 0 }
}
