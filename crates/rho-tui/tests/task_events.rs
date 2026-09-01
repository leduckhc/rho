//! The interface must read background task events, for the whole session.
//!
//! The renderer could draw a task row before this lane, and no real session ever showed
//! one. Nothing subscribed to the registry, and `ToolContext::agent_events` lives for one
//! tool call while a task outlives the turn. See `SPEC-the-task-event-bridge`.
//!
//! Two of these tests are reducer rules that only a lag repair can trigger. A repair sends
//! events a live session never sends: a second `TaskStart` for a task already on screen,
//! and a progress report that is older than the row. See
//! `D-a-lagged-frontend-is-resynced-not-told`.

use std::sync::Arc;

use rho_core::{
    AgentEvent, BackgroundReason, TaskId, TaskLimits, TaskProgress, TaskRegistry, TaskState,
};
use rho_tui::{Row, TuiState};

/// The event channel holds 256 events. A flood past that forces a lag.
const CHANNEL_CAPACITY: usize = 256;

fn task_rows(state: &TuiState) -> Vec<&Row> {
    state
        .rows
        .iter()
        .filter(|row| matches!(row, Row::Task { .. }))
        .collect()
}

fn start(id: &str, command: &str) -> AgentEvent {
    AgentEvent::TaskStart {
        id: TaskId(id.to_string()),
        command: command.to_string(),
        reason: BackgroundReason::ModelRequested,
    }
}

fn progressed(id: &str, percent: u8) -> AgentEvent {
    AgentEvent::TaskProgressed {
        id: TaskId(id.to_string()),
        progress: TaskProgress {
            percent: Some(percent),
            ..TaskProgress::default()
        },
    }
}

fn ended(id: &str, code: i32) -> AgentEvent {
    AgentEvent::TaskEnd {
        id: TaskId(id.to_string()),
        state: TaskState::Exited { code },
        output_tail: String::new(),
    }
}

/// Fold every event the stream has ready, the way the event loop's arm does.
///
/// `unconstrained` is load-bearing. tokio gives a task a budget of 128 resource operations
/// per poll, so a plain `now_or_never` drain stops after 128 events even though more are
/// ready. This test floods 264, and the early stop hid the duplicate row it exists to
/// catch. The interactive loop has no such limit, because it awaits across many polls.
async fn fold_ready(state: &mut TuiState, events: &mut rho_core::SessionEvents, now_millis: i64) {
    use futures::FutureExt;
    while let Some(Some(event)) = tokio::task::unconstrained(events.next()).now_or_never() {
        state.apply(&event, now_millis);
    }
}

// ---- The reducer rules a repair needs. -----------------------------------------

#[test]
fn a_second_start_for_one_task_draws_one_row() {
    // A lag repair announces every task it can see, including ones already on screen.
    // `on_task_start` pushed a row unconditionally, so the repair would double the row.
    let mut state = TuiState::default();
    state.apply(&start("task-1", "cargo build"), 0);
    state.apply(&start("task-1", "cargo build"), 500);
    assert_eq!(
        task_rows(&state).len(),
        1,
        "one task is one row: {:?}",
        state.rows
    );
}

#[test]
fn a_repeated_start_keeps_the_row_it_already_drew() {
    // The repeated start is a no-op, and not an update. The row already holds the live
    // command and the live progress, and its start time is the clock the duration slot
    // measures. An update would reset that clock, so a four-minute build would look new.
    let mut state = TuiState::default();
    state.apply(&start("task-1", "cargo build"), 0);
    state.apply(&progressed("task-1", 42), 100);
    state.apply(&start("task-1", "cargo build"), 240_000);

    let rows = task_rows(&state);
    assert_eq!(rows.len(), 1, "still one row: {:?}", state.rows);
    match rows.first() {
        Some(Row::Task {
            progress, command, ..
        }) => {
            assert!(
                progress.contains("42"),
                "a repeated start must not wipe the live progress: {progress:?}"
            );
            assert_eq!(command, "cargo build");
        }
        other => panic!("expected one task row, got {other:?}"),
    }
}

#[test]
fn a_progress_report_for_a_finished_row_is_ignored() {
    // Defence in depth. The bridge already drops a report that follows an end it sent, so
    // this rule guards a different level: any producer, now or later, that reports progress
    // on a task the screen has finished. No later report would ever correct such a row.
    let mut state = TuiState::default();
    state.apply(&start("task-1", "cargo test"), 0);
    state.apply(&ended("task-1", 0), 100);
    state.apply(&progressed("task-1", 13), 200);

    match task_rows(&state).first() {
        Some(Row::Task {
            progress,
            finished,
            state: word,
            ..
        }) => {
            assert!(*finished, "the row stays finished");
            assert!(
                !progress.contains("13"),
                "a stale report must not move a finished row: {progress:?}"
            );
            assert!(
                !word.contains("running"),
                "and it must not read as running: {word:?}"
            );
        }
        other => panic!("expected one task row, got {other:?}"),
    }
}

// ---- The bridge and the reducer, together. --------------------------------------

#[tokio::test]
async fn a_task_event_from_the_bridge_becomes_a_row() {
    let registry = Arc::new(TaskRegistry::new(TaskLimits::default()));
    let mut events = registry.session_events();
    let mut state = TuiState::default();

    let handle = registry
        .start("cargo build", BackgroundReason::ModelRequested)
        .expect("the registry starts one task");
    handle.report_progress(TaskProgress {
        percent: Some(42),
        ..TaskProgress::default()
    });
    fold_ready(&mut state, &mut events, 0).await;

    match task_rows(&state).first() {
        Some(Row::Task {
            command, progress, ..
        }) => {
            assert_eq!(command, "cargo build");
            assert!(
                progress.contains("42"),
                "the progress reaches the row: {progress:?}"
            );
        }
        other => panic!("expected one task row, got {other:?}"),
    }
}

#[tokio::test]
async fn a_task_row_finishes_while_no_run_is_active() {
    // This is the whole point of a session-lifetime stream. A build usually ends between
    // two prompts. A run-scoped stream would drop this event and leave the row running.
    let registry = Arc::new(TaskRegistry::new(TaskLimits::default()));
    let mut events = registry.session_events();
    let mut state = TuiState::default();
    let handle = registry
        .start("cargo build", BackgroundReason::ModelRequested)
        .expect("the registry starts one task");

    fold_ready(&mut state, &mut events, 0).await;
    assert_ne!(
        state.activity,
        rho_tui::ActivityState::Running,
        "no run is active in this test, which is the case that was lost"
    );

    handle.finish(TaskState::Exited { code: 0 });
    fold_ready(&mut state, &mut events, 1_000).await;

    match task_rows(&state).first() {
        Some(Row::Task { finished, .. }) => assert!(
            *finished,
            "a task that ends while rho is idle must still finish its row"
        ),
        other => panic!("expected one task row, got {other:?}"),
    }
}

// ---- The invariant: after a lag, the screen matches the registry. ---------------

#[tokio::test]
async fn a_lagged_interface_ends_with_rows_that_match_the_registry() {
    // The test that catches the class. The narrower stream tests in `rho-core` cannot see
    // it, because they never run the reducer: a repair that sends only a state event passes
    // every one of them while the screen stays empty.
    //
    // It asserts the pairing, not one example. Every task the registry holds has exactly one
    // row, that row carries the real command, and its finished flag agrees with the
    // registry.
    let registry = Arc::new(TaskRegistry::new(TaskLimits::default()));
    let mut events = registry.session_events();
    let mut state = TuiState::default();

    // The flood evicts the first task's own start event. The second task then starts inside
    // the lost window, so the reader never sees its start either.
    let first = registry
        .start("cargo build --release", BackgroundReason::ModelRequested)
        .expect("the registry starts a task");
    for step in 0..(CHANNEL_CAPACITY + 8) {
        first.report_progress(TaskProgress {
            percent: Some((step % 100) as u8),
            ..TaskProgress::default()
        });
    }
    let second = registry
        .start("npm run watch", BackgroundReason::KnownLongRunning)
        .expect("the registry starts a second task");
    second.report_progress(TaskProgress {
        percent: Some(70),
        ..TaskProgress::default()
    });
    first.finish(TaskState::Exited { code: 0 });

    fold_ready(&mut state, &mut events, 5_000).await;

    let expected = registry.list().await;
    assert_eq!(expected.len(), 2, "the registry holds both tasks");
    for snapshot in &expected {
        let matching: Vec<&Row> = state
            .rows
            .iter()
            .filter(
                |row| matches!(row, Row::Task { id, .. } if id.as_str() == snapshot.id.0.as_str()),
            )
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "task {:?} must have exactly one row, and the rows are {:?}",
            snapshot.id,
            state.rows
        );
        match matching[0] {
            Row::Task {
                finished, command, ..
            } => {
                assert_eq!(
                    *finished,
                    snapshot.state.is_final(),
                    "task {:?} draws finished={finished} while the registry says {:?}",
                    snapshot.id,
                    snapshot.state
                );
                assert_eq!(
                    command, &snapshot.command,
                    "a repaired row must carry the real command"
                );
            }
            other => panic!("expected a task row, got {other:?}"),
        }
    }
}

// ---- The guard for the wiring itself. ------------------------------------------

#[test]
fn the_event_loop_reads_the_task_stream() {
    // Six switches shipped because a capability had no call site, and no test could see it.
    // The fold is proved above with the real bridge and the real reducer. What no unit test
    // can reach is the `select!` arm, because the loop owns a real terminal. So this test
    // reads the loop and states the rule where a contributor meets it. The live drive in
    // `bench/tui_task_row_drive.py` is the other half, and it runs the real binary.
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/app.rs"))
        .expect("read the app source");
    let code: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<&str>>()
        .join("\n");
    let loop_body = code
        .split("tokio::select! {")
        .nth(1)
        .expect("the event loop selects over its sources");
    assert!(
        loop_body.contains("task_events"),
        "the event loop must have an arm for the task event stream, or no row ever draws"
    );
}

// The `/model` slash command carries an argument. These tests pin the two branches: alone,
// it reports the current model; with an argument, it explains the deferred write path.
#[test]
fn model_is_marked_built_and_shown_in_the_list() {
    use rho_tui::slash_commands;
    let names: Vec<&str> = slash_commands().iter().map(|c| c.name).collect();
    assert!(names.contains(&"/model"), "\"/model\" is in the list");
    let model = slash_commands()
        .iter()
        .find(|c| c.name == "/model")
        .expect("in list");
    assert!(model.built, "/model is now built");
}

#[test]
fn a_command_matches_when_it_has_an_argument() {
    use rho_tui::filter_slash_commands;
    let hits = filter_slash_commands("/model claude-sonnet-4-6");
    let names: Vec<&str> = hits.iter().map(|c| c.name).collect();
    assert!(
        names.contains(&"/model"),
        "/model matches with an argument: {names:?}"
    );
}

#[test]
fn slash_argument_extracts_a_bare_argument() {
    use rho_tui::slash_argument;
    assert_eq!(
        slash_argument("/model claude-sonnet-4-6", "/model"),
        "claude-sonnet-4-6"
    );
    assert_eq!(slash_argument("/model", "/model"), "");
    assert_eq!(slash_argument("/model   spacey  ", "/model"), "spacey");
}
