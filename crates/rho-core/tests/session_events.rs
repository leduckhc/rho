//! The task event bridge: a background task's events must reach a frontend.
//!
//! A task outlives the tool call that started it, and it outlives the run. So its events
//! cannot travel on `AgentEvents`. `TaskRegistry::session_events` is the session-lifetime
//! stream a frontend reads instead. See `SPEC-the-task-event-bridge`.
//!
//! No test here sleeps, and none touches the network. Every test drives the real registry.

use std::sync::Arc;

use rho_core::{
    AgentEvent, BackgroundReason, TaskId, TaskLimits, TaskProgress, TaskRegistry, TaskState,
};

/// The event channel holds 256 events. A flood past that forces a lag.
const CHANNEL_CAPACITY: usize = 256;

fn registry() -> Arc<TaskRegistry> {
    Arc::new(TaskRegistry::new(TaskLimits::default()))
}

/// The id of whatever task an event is about.
fn event_id(event: &AgentEvent) -> Option<&TaskId> {
    match event {
        AgentEvent::TaskStart { id, .. }
        | AgentEvent::TaskProgressed { id, .. }
        | AgentEvent::TaskEnd { id, .. } => Some(id),
        _ => None,
    }
}

/// A short name for an event kind, for a readable assertion message.
fn kind(event: &AgentEvent) -> &'static str {
    match event {
        AgentEvent::TaskStart { .. } => "TaskStart",
        AgentEvent::TaskProgressed { .. } => "TaskProgressed",
        AgentEvent::TaskEnd { .. } => "TaskEnd",
        _ => "other",
    }
}

/// Read every event the stream can produce without waiting for a new one.
///
/// It stops at the first event that is not ready, so it never hangs.
///
/// `unconstrained` is load-bearing. tokio gives a task a budget of 128 resource
/// operations per poll, and a channel that has spent the budget returns `Pending` even
/// though an event is ready. A plain `now_or_never` therefore stopped this drain after
/// 128 events, and a lag test that floods 264 of them then passed while the events that
/// mattered were never read. That is a test passing against the bug it was written for.
async fn drain(events: &mut rho_core::SessionEvents) -> Vec<AgentEvent> {
    use futures::FutureExt;
    let mut seen = Vec::new();
    while let Some(Some(event)) = tokio::task::unconstrained(events.next()).now_or_never() {
        seen.push(event);
    }
    seen
}

// ---- The plain path. ------------------------------------------------------------

#[tokio::test]
async fn a_task_event_reaches_a_session_stream() {
    let registry = registry();
    let mut events = registry.session_events();

    let handle = registry
        .start("cargo build", BackgroundReason::ModelRequested)
        .expect("the registry starts one task");
    handle.report_progress(TaskProgress {
        percent: Some(42),
        ..TaskProgress::default()
    });
    handle.finish(TaskState::Exited { code: 0 });

    let seen = drain(&mut events).await;
    let kinds: Vec<&str> = seen.iter().map(kind).collect();
    assert_eq!(
        kinds,
        ["TaskStart", "TaskProgressed", "TaskEnd"],
        "the three task events reach the frontend, in order: {kinds:?}"
    );
}

#[tokio::test]
async fn a_task_started_after_the_bridge_still_reports_its_start() {
    // `session_events` must subscribe inside the call. A subscription made later, for
    // example in the first `next`, would lose the start event of a task that began
    // in between. That race is invisible in a test that starts the task first.
    let registry = registry();
    let mut events = registry.session_events();

    let handle = registry
        .start("sleep 5", BackgroundReason::KnownLongRunning)
        .expect("the registry starts one task");
    drop(handle);

    let seen = drain(&mut events).await;
    assert!(
        seen.iter().any(|event| matches!(
            event,
            AgentEvent::TaskStart { command, .. } if command == "sleep 5"
        )),
        "the start event of a task begun after the call must arrive: {:?}",
        seen.iter().map(kind).collect::<Vec<_>>()
    );
}

// ---- The lag repair. -----------------------------------------------------------

/// Start a task, then push more progress reports than the channel can hold.
///
/// The reader has not read anything yet, so the flood evicts the oldest events. The
/// task's own `TaskStart` is the very oldest, so it goes first.
fn flood_past_the_channel(registry: &Arc<TaskRegistry>, command: &str) -> rho_core::TaskHandle {
    flood_past_the_channel_with(registry, command, BackgroundReason::ModelRequested)
}

/// The same flood, with a stated reason, so a test can prove the reason survives a repair.
fn flood_past_the_channel_with(
    registry: &Arc<TaskRegistry>,
    command: &str,
    reason: BackgroundReason,
) -> rho_core::TaskHandle {
    let handle = registry
        .start(command, reason)
        .expect("the registry starts one task");
    for step in 0..(CHANNEL_CAPACITY + 8) {
        handle.report_progress(TaskProgress {
            percent: Some((step % 100) as u8),
            ..TaskProgress::default()
        });
    }
    handle
}

#[tokio::test]
async fn a_lag_repair_sends_a_start_before_any_state_event() {
    // Only `on_task_start` creates a row in a reducer. So a repair that sends a state
    // event first lands on nothing, and the task never draws again. This is the rule a
    // first draft of the contract got wrong.
    let registry = registry();
    let mut events = registry.session_events();
    let handle = flood_past_the_channel(&registry, "cargo build");
    handle.finish(TaskState::Exited { code: 0 });

    let seen = drain(&mut events).await;
    let mut started: Vec<&TaskId> = Vec::new();
    for event in &seen {
        let Some(id) = event_id(event) else { continue };
        match event {
            AgentEvent::TaskStart { .. } => started.push(id),
            _ => assert!(
                started.contains(&id),
                "a {} for {id:?} arrived with no TaskStart before it: {:?}",
                kind(event),
                seen.iter().map(kind).collect::<Vec<_>>()
            ),
        }
    }
    assert!(!seen.is_empty(), "the repair must send something");
}

#[tokio::test]
async fn a_lagged_reader_learns_a_task_it_never_saw_start() {
    let registry = registry();
    let mut events = registry.session_events();
    let _handle = flood_past_the_channel(&registry, "cargo build --release");

    let seen = drain(&mut events).await;
    assert!(
        seen.iter().any(|event| matches!(
            event,
            AgentEvent::TaskStart { command, .. } if command == "cargo build --release"
        )),
        "a task whose start was evicted must still be announced, with its command: {:?}",
        seen.iter().map(kind).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn a_repair_reports_why_a_task_went_to_the_background() {
    // The repair rebuilds a `TaskStart`, and that event carries a `BackgroundReason`. The
    // reason is why `TaskSnapshot` gained a field: without it the repair would have to
    // invent one, and a frontend would be told the model asked for a task it never
    // mentioned. Nothing else asserts the value, so a hardcoded reason would pass.
    let registry = registry();
    let mut events = registry.session_events();
    let _handle = flood_past_the_channel_with(
        &registry,
        "npm run watch",
        BackgroundReason::KnownLongRunning,
    );

    let seen = drain(&mut events).await;
    let reason = seen.iter().find_map(|event| match event {
        AgentEvent::TaskStart {
            command, reason, ..
        } if command == "npm run watch" => Some(*reason),
        _ => None,
    });
    assert_eq!(
        reason,
        Some(BackgroundReason::KnownLongRunning),
        "a repaired start must carry the real reason: {:?}",
        seen.iter().map(kind).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn a_lagged_reader_still_learns_a_task_ended() {
    // The invariant, not an example: whatever was lost, the reader must not be left
    // believing a finished task still runs.
    let registry = registry();
    let mut events = registry.session_events();
    let handle = flood_past_the_channel(&registry, "cargo test");
    handle.finish(TaskState::Exited { code: 1 });

    let seen = drain(&mut events).await;
    let last_for_task = seen
        .iter()
        .rfind(|event| event_id(event).is_some())
        .expect("the reader sees at least one task event");
    assert!(
        matches!(last_for_task, AgentEvent::TaskEnd { state, .. } if state.is_final()),
        "the last word on a finished task must be its end: {:?}",
        seen.iter().map(kind).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn a_lagged_reader_learns_the_state_of_a_running_task() {
    let registry = registry();
    let mut events = registry.session_events();
    let handle = flood_past_the_channel(&registry, "npm run watch");
    handle.report_progress(TaskProgress {
        percent: Some(77),
        message: Some("linking".to_string()),
        ..TaskProgress::default()
    });

    let seen = drain(&mut events).await;
    assert!(
        seen.iter().any(|event| matches!(
            event,
            AgentEvent::TaskProgressed { progress, .. } if progress.percent == Some(77)
        )),
        "a running task must be corrected to its current progress, not abandoned: {:?}",
        seen.iter().map(kind).collect::<Vec<_>>()
    );
    assert!(
        !seen
            .iter()
            .any(|event| matches!(event, AgentEvent::TaskEnd { .. })),
        "a running task must not be reported as finished"
    );
}

#[tokio::test]
async fn a_task_never_goes_back_to_running_after_it_ended() {
    // The repair reads the registry, so it can send a `TaskEnd` while the channel still
    // holds progress reports from before the lag. Those arrive afterwards. Without a rule
    // in the bridge, the last word on a finished task would be a stale percentage, and
    // every frontend would need its own guard.
    let registry = registry();
    let mut events = registry.session_events();
    let handle = flood_past_the_channel(&registry, "cargo test");
    handle.finish(TaskState::Exited { code: 0 });

    let seen = drain(&mut events).await;
    let mut ended = false;
    for event in &seen {
        match event {
            AgentEvent::TaskEnd { .. } => ended = true,
            AgentEvent::TaskProgressed { .. } => assert!(
                !ended,
                "a progress report reached the frontend after the task ended: {:?}",
                seen.iter().map(kind).collect::<Vec<_>>()
            ),
            _ => {}
        }
    }
    assert!(ended, "the task must be reported as ended at all");
}

// ---- Lifetime. -----------------------------------------------------------------

#[tokio::test]
async fn the_bridge_does_not_keep_a_dropped_registry_alive() {
    // Dropping the registry kills every running task. A strong reference inside the
    // bridge would tie a child process to a stream nobody reads, so this asserts the
    // weak reference by its only visible effect.
    let registry = registry();
    let mut events = registry.session_events();
    drop(registry);
    assert!(
        events.next().await.is_none(),
        "the stream ends when the registry is gone"
    );
}

#[tokio::test]
async fn an_event_sent_before_the_last_arc_drops_still_arrives() {
    // The last event of a session is often the one that matters most. A stream that
    // closed on the drop and threw the buffer away would lose it.
    let registry = registry();
    let mut events = registry.session_events();
    let handle = registry
        .start("cargo build", BackgroundReason::ModelRequested)
        .expect("the registry starts one task");
    handle.finish(TaskState::Exited { code: 0 });
    drop(handle);
    drop(registry);

    let seen = drain(&mut events).await;
    assert!(
        seen.iter()
            .any(|event| matches!(event, AgentEvent::TaskEnd { .. })),
        "an event sent before the drop must still arrive: {:?}",
        seen.iter().map(kind).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn a_dropped_stream_leaves_the_registry_working() {
    // The bridge spawns no task. It reads inside `next`, so a reader that leaves takes
    // its subscription with it and nothing is left running. What is worth proving is
    // that the registry is unharmed: a second frontend, or a later one, still works.
    let registry = registry();
    drop(registry.session_events());

    let mut events = registry.session_events();
    let handle = registry
        .start("cargo build", BackgroundReason::ModelRequested)
        .expect("the registry starts one task");
    handle.finish(TaskState::Exited { code: 0 });

    let seen = drain(&mut events).await;
    let kinds: Vec<&str> = seen.iter().map(kind).collect();
    assert_eq!(
        kinds,
        ["TaskStart", "TaskEnd"],
        "a reader that left must not break the next one: {kinds:?}"
    );
}
