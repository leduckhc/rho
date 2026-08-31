//! Tests for background tasks in `rho-core`.
//!
//! No test sleeps to synchronise. A wait test uses `tokio::time` pause and a
//! `Notify` signal, so it is deterministic and fast. See `SPEC-background-tasks` section 10.

use std::sync::Arc;
use std::time::Duration;

use rho_core::{
    AgentEvent, BackgroundReason, RunMode, TaskError, TaskId, TaskLimits, TaskProgress,
    TaskRegistry, TaskState, WaitUntil, decide_run_mode,
};

const FOREGROUND_LIMIT_MS: u64 = 30_000;

// --- The run-mode decision. All pure and offline. ---

#[test]
fn decide_run_mode_respects_an_explicit_background_request() {
    let mode = decide_run_mode("echo hi", Some(true), 1_000, FOREGROUND_LIMIT_MS);
    assert_eq!(
        mode,
        RunMode::Background(BackgroundReason::ModelRequested),
        "an explicit true must background even a trivial command"
    );
}

#[test]
fn decide_run_mode_respects_an_explicit_foreground_request() {
    // An explicit false beats every heuristic, even for `cargo test`.
    let mode = decide_run_mode("cargo test --all", Some(false), 1_000, FOREGROUND_LIMIT_MS);
    assert_eq!(mode, RunMode::Foreground);
}

#[test]
fn decide_run_mode_backgrounds_a_long_timeout() {
    let mode = decide_run_mode(
        "echo hi",
        None,
        FOREGROUND_LIMIT_MS + 1,
        FOREGROUND_LIMIT_MS,
    );
    assert_eq!(
        mode,
        RunMode::Background(BackgroundReason::LongTimeoutRequested)
    );
}

#[test]
fn decide_run_mode_backgrounds_a_watch_command() {
    for command in [
        "npm run dev",
        "cargo watch -x run",
        "vite --watch",
        "python -m http.server serve",
    ] {
        let mode = decide_run_mode(command, None, 1_000, FOREGROUND_LIMIT_MS);
        assert_eq!(
            mode,
            RunMode::Background(BackgroundReason::KnownLongRunning),
            "{command} must background"
        );
    }
}

#[test]
fn decide_run_mode_backgrounds_a_test_run() {
    for command in [
        "cargo test",
        "npm test",
        "pytest -q",
        "go test ./...",
        "make",
        "gradle build",
    ] {
        let mode = decide_run_mode(command, None, 1_000, FOREGROUND_LIMIT_MS);
        assert_eq!(
            mode,
            RunMode::Background(BackgroundReason::KnownLongRunning),
            "{command} must background"
        );
    }
}

#[test]
fn decide_run_mode_backgrounds_a_follow_command() {
    for command in ["tail -f app.log", "journalctl -f", "kubectl logs -f pod"] {
        let mode = decide_run_mode(command, None, 1_000, FOREGROUND_LIMIT_MS);
        assert_eq!(
            mode,
            RunMode::Background(BackgroundReason::KnownLongRunning),
            "{command} must background"
        );
    }
}

#[test]
fn decide_run_mode_keeps_a_quick_command_in_the_foreground() {
    // A trivial command must not cost a probe.
    for command in [
        "ls",
        "git status",
        "echo hello",
        "cat file.txt",
        "grep foo bar.txt",
    ] {
        let mode = decide_run_mode(command, None, 1_000, FOREGROUND_LIMIT_MS);
        assert_eq!(mode, RunMode::Foreground, "{command} must stay foreground");
    }
}

#[test]
fn decide_run_mode_reports_a_reason_for_every_background_choice() {
    let cases = [
        (
            "echo hi",
            Some(true),
            1_000,
            BackgroundReason::ModelRequested,
        ),
        (
            "echo hi",
            None,
            FOREGROUND_LIMIT_MS + 1,
            BackgroundReason::LongTimeoutRequested,
        ),
        (
            "cargo test",
            None,
            1_000,
            BackgroundReason::KnownLongRunning,
        ),
    ];
    for (command, requested, timeout, expected) in cases {
        match decide_run_mode(command, requested, timeout, FOREGROUND_LIMIT_MS) {
            RunMode::Background(reason) => {
                assert_eq!(reason, expected, "{command} must report its reason")
            }
            RunMode::Foreground => panic!("{command} must background"),
        }
    }
}

// --- Waiting. No test sleeps. ---

fn registry() -> Arc<TaskRegistry> {
    Arc::new(TaskRegistry::new(TaskLimits::default()))
}

#[tokio::test]
async fn wait_returns_when_the_task_finishes() {
    let registry = registry();
    let handle = registry
        .start("sleep", BackgroundReason::ModelRequested)
        .unwrap();
    let id = handle.id();

    let waiter = {
        let registry = Arc::clone(&registry);
        let id = id.clone();
        tokio::spawn(async move {
            registry
                .wait(&id, Duration::from_secs(10), WaitUntil::Finished)
                .await
        })
    };

    // The finish wakes the waiter. No sleep: the waiter is already parked on the
    // notify, so this event cannot be lost.
    handle.finish(TaskState::Exited { code: 0 });

    let snapshot = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("wait must return after finish")
        .unwrap()
        .unwrap();
    assert!(snapshot.state.is_final());
    assert!(snapshot.state.is_success());
}

#[tokio::test]
async fn wait_returns_on_the_next_progress_checkpoint() {
    let registry = registry();
    let handle = registry
        .start("build", BackgroundReason::KnownLongRunning)
        .unwrap();
    let id = handle.id();

    // The main task calls `wait` first, so it reads its baseline revision and
    // parks on the notify before any progress fires. The spawned task then
    // reports progress, which wakes the parked waiter. No sleep, and the event
    // cannot be lost.
    let reporter = tokio::spawn(async move {
        handle.report_progress(TaskProgress {
            percent: Some(42),
            ..Default::default()
        });
    });

    let snapshot = tokio::time::timeout(
        Duration::from_secs(5),
        registry.wait(&id, Duration::from_secs(10), WaitUntil::NextProgress),
    )
    .await
    .expect("wait must return on the next progress")
    .unwrap();
    reporter.await.unwrap();
    assert_eq!(snapshot.progress.percent, Some(42));
    assert_eq!(snapshot.state, TaskState::Running);
}

#[tokio::test(start_paused = true)]
async fn wait_returns_the_snapshot_when_the_budget_expires() {
    // The budget is the only timer. With no event, the wait wakes on the budget
    // and returns the current snapshot. `start_paused` advances virtual time, so
    // the test is instant and deterministic.
    let registry = registry();
    let handle = registry
        .start("watch", BackgroundReason::KnownLongRunning)
        .unwrap();
    let id = handle.id();

    let snapshot = registry
        .wait(&id, Duration::from_millis(50), WaitUntil::Finished)
        .await
        .unwrap();
    assert_eq!(
        snapshot.state,
        TaskState::Running,
        "budget expiry returns the live snapshot"
    );
}

#[tokio::test]
async fn wait_on_a_finished_task_returns_at_once() {
    let registry = registry();
    let handle = registry
        .start("echo", BackgroundReason::ModelRequested)
        .unwrap();
    let id = handle.id();
    handle.finish(TaskState::Exited { code: 0 });

    // A long budget, but the task is already final, so this returns without a
    // wake and without hitting the budget.
    let snapshot = tokio::time::timeout(
        Duration::from_secs(5),
        registry.wait(&id, Duration::from_secs(3600), WaitUntil::Finished),
    )
    .await
    .expect("wait on a finished task must return at once")
    .unwrap();
    assert!(snapshot.state.is_final());
}

#[tokio::test]
async fn wait_on_an_unknown_task_is_an_error() {
    let registry = registry();
    let error = registry
        .wait(
            &TaskId("no-such".into()),
            Duration::from_secs(1),
            WaitUntil::Finished,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, TaskError::UnknownTask(_)));
}

// --- Completion signalling that needs no process. ---

#[tokio::test]
async fn task_end_is_emitted_once_only() {
    let registry = registry();
    let mut events = registry.subscribe();
    let handle = registry
        .start("echo", BackgroundReason::ModelRequested)
        .unwrap();

    handle.finish(TaskState::Exited { code: 0 });
    // A second finish must not fire another end event.
    handle.finish(TaskState::Exited { code: 1 });

    let mut ends = 0;
    // Drain every buffered event without blocking.
    while let Ok(event) = events.try_recv() {
        if matches!(event, AgentEvent::TaskEnd { .. }) {
            ends += 1;
        }
    }
    assert_eq!(ends, 1, "exactly one end event");
    // The first final state wins.
    let snapshot = registry.get(&handle.id()).await.unwrap();
    assert_eq!(snapshot.state, TaskState::Exited { code: 0 });
}

// --- Limits. ---

#[tokio::test]
async fn output_keeps_the_tail_and_counts_dropped_bytes() {
    let limits = TaskLimits {
        max_output_bytes_per_task: 16,
        ..TaskLimits::default()
    };
    let registry = Arc::new(TaskRegistry::new(limits));
    let handle = registry
        .start("noisy", BackgroundReason::ModelRequested)
        .unwrap();

    // Each push adds the line plus a newline.
    handle.push_output("aaaaaaaa"); // 9 bytes with newline
    handle.push_output("bbbbbbbb"); // 9 more, total 18, over the 16 cap
    let snapshot = registry.get(&handle.id()).await.unwrap();

    assert!(
        snapshot.output_tail.len() <= 16,
        "the tail must stay within the cap, got {}",
        snapshot.output_tail.len()
    );
    assert!(
        snapshot.dropped_bytes > 0,
        "the dropped head must be counted"
    );
    assert!(
        snapshot.output_tail.ends_with("bbbbbbbb\n"),
        "the tail keeps the newest output: {:?}",
        snapshot.output_tail
    );
}

#[tokio::test]
async fn starting_more_than_max_concurrent_tasks_is_an_error() {
    let limits = TaskLimits {
        max_concurrent: 2,
        ..TaskLimits::default()
    };
    let registry = Arc::new(TaskRegistry::new(limits));
    let _a = registry
        .start("a", BackgroundReason::ModelRequested)
        .unwrap();
    let _b = registry
        .start("b", BackgroundReason::ModelRequested)
        .unwrap();
    let error = registry
        .start("c", BackgroundReason::ModelRequested)
        .unwrap_err();
    match error {
        TaskError::TooManyTasks { max } => assert_eq!(max, 2),
        other => panic!("expected TooManyTasks, got {other:?}"),
    }
    // The message names the limit and suggests a cancel.
    let text = error.to_string();
    assert!(text.contains('2'), "the message names the limit: {text}");
    assert!(
        text.to_lowercase().contains("cancel"),
        "the message suggests a cancel: {text}"
    );
}

#[tokio::test]
async fn a_finished_task_frees_a_concurrency_slot() {
    let limits = TaskLimits {
        max_concurrent: 1,
        ..TaskLimits::default()
    };
    let registry = Arc::new(TaskRegistry::new(limits));
    let a = registry
        .start("a", BackgroundReason::ModelRequested)
        .unwrap();
    assert!(
        registry
            .start("b", BackgroundReason::ModelRequested)
            .is_err()
    );
    a.finish(TaskState::Exited { code: 0 });
    // The finished task no longer counts against the limit.
    assert!(
        registry
            .start("b", BackgroundReason::ModelRequested)
            .is_ok()
    );
}

// --- Task state helpers and serde. ---

#[test]
fn task_state_is_final_and_is_success() {
    assert!(!TaskState::Running.is_final());
    assert!(TaskState::Exited { code: 0 }.is_final());
    assert!(TaskState::Exited { code: 0 }.is_success());
    assert!(!TaskState::Exited { code: 1 }.is_success());
    assert!(TaskState::Canceled.is_final());
    assert!(!TaskState::Canceled.is_success());
    assert!(TaskState::TimedOut.is_final());
    assert!(TaskState::Signaled { signal: None }.is_final());
}

#[test]
fn task_state_serialises_in_snake_case() {
    let json = serde_json::to_value(TaskState::Exited { code: 2 }).unwrap();
    assert_eq!(json, serde_json::json!({ "exited": { "code": 2 } }));
    let json = serde_json::to_value(TaskState::TimedOut).unwrap();
    assert_eq!(json, serde_json::json!("timed_out"));
}

// --- The snapshot's wire form. The model reads this. ---

#[tokio::test]
async fn a_snapshot_names_its_enums_in_one_casing() {
    // `TaskSnapshot` is serialised for the model by the `task` tool. `TaskState` was
    // snake_case from the start, and `reason` arrived later with no casing attribute, so one
    // object carried `"state": "running"` beside `"reason": "ModelRequested"`.
    //
    // The match is exhaustive on purpose. A new reason fails the build here, so the next
    // variant cannot arrive with the wrong casing in silence.
    for reason in [
        BackgroundReason::ModelRequested,
        BackgroundReason::KnownLongRunning,
        BackgroundReason::LongTimeoutRequested,
        BackgroundReason::AdoptedOnTimeout,
    ] {
        let expected = match reason {
            BackgroundReason::ModelRequested => "model_requested",
            BackgroundReason::KnownLongRunning => "known_long_running",
            BackgroundReason::LongTimeoutRequested => "long_timeout_requested",
            BackgroundReason::AdoptedOnTimeout => "adopted_on_timeout",
        };
        let registry = Arc::new(TaskRegistry::new(TaskLimits::default()));
        let _handle = registry
            .start("cargo build", reason)
            .expect("the registry starts one task");
        let snapshot = registry.list().await.remove(0);
        let json = serde_json::to_value(&snapshot).expect("a snapshot serialises");
        assert_eq!(
            json["reason"], expected,
            "the reason must be snake_case on the wire: {json}"
        );
        assert_eq!(
            json["state"], "running",
            "and the sibling enum keeps the same convention: {json}"
        );
    }
}
