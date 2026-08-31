//! Drive the task row on a real terminal, with a real child process.
//!
//! This is the step 11 harness for `SPEC-the-task-row-draws-its-progress`. No test backend
//! can prove what this proves: it starts a real command through the real `bash` tool, the
//! real `TaskRegistry` broadcasts real events, the real reducer folds them, and the real
//! renderer writes bytes to a real terminal.
//!
//! **It now drives the shipped bridge.** It used to subscribe to the registry itself,
//! because nothing in a binary did, and a task row could not appear in a real session at
//! all. `SPEC-the-task-event-bridge` closed that, so this harness reads
//! `TaskRegistry::session_events`, which is the exact stream `rho-cli` gives the interface.
//! A harness that supplies the missing half proves nothing about the product. See
//! `docs/verification/task-row-progress.md` and `docs/verification/task-event-bridge.md`.
//!
//! Run one scenario:
//!
//! ```text
//! cargo build --release --example task_row_drive -p rho-tui
//! ./target/release/examples/task_row_drive progress
//! ```
//!
//! Every scenario runs its task **twice**, because "the same thing twice" has caught two
//! defects in this project.
//!
//! It prints one `ROW|` line per drawn task row at the end, so a harness can assert on the
//! row text. The row text comes from a second render of the same state, through a test
//! backend, because reading a row back out of a live terminal needs a screen scraper. The
//! byte-level claims in the verification document come from the live stream, not from here.

use std::io::Write;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::Terminal;
use ratatui::backend::{CrosstermBackend, TestBackend};
use rho_core::{
    AgentEvent, CancelToken, TaskId, TaskLimits, TaskProgress, TaskRegistry, Tool, ToolContext,
};
use rho_tools::BashTool;
use rho_tui::{Row, TuiState, render};

/// The frame the row snapshot draws into. It follows the real terminal, so a narrow
/// pseudo-terminal shows the narrow layout.
fn snapshot_size() -> (u16, u16) {
    crossterm::terminal::size().unwrap_or((100, 24))
}

/// A hostile progress message, as a real child would print it. The bytes are JSON escapes
/// on the wire, so the line parses, and they decode to a real escape sequence: clear the
/// screen, home the cursor, then retitle the window through OSC.
const HOSTILE_JSON: &str =
    r#"{"percent": 50, "message": "\u001b[2J\u001b[H\u001b]0;pwned\u0007wiped"}"#;

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_millis() as i64
}

/// The command for a scenario, or `None` when the scenario needs no child.
fn command_for(scenario: &str) -> Option<String> {
    match scenario {
        // A real progress reporter: five explicit `RHO_PROGRESS` lines.
        "progress" => Some(
            "for i in 1 2 3 4 5; do \
             echo \"RHO_PROGRESS {\\\"percent\\\": $((i * 20)), \\\"message\\\": \\\"compiling\\\", \
             \\\"done\\\": $i, \\\"total\\\": 5}\"; sleep 0.1; done"
                .to_string(),
        ),
        // A task that reports no progress at all.
        "quiet" => Some("sleep 0.2; echo working".to_string()),
        // A task that reports hostile bytes.
        "hostile" => Some(format!("printf 'RHO_PROGRESS {HOSTILE_JSON}\\n'; sleep 0.2")),
        // The failure paths: an absent file, and a file with no execute permission.
        "missing" => Some("cat /nope/nope/nope".to_string()),
        "denied" => Some("printf 'echo hi\\n' > blocked.sh; chmod 000 blocked.sh; ./blocked.sh".to_string()),
        // No child. The row is built directly, the way another frontend would build it.
        "raw" => None,
        _ => None,
    }
}

/// Render the state through a test backend and return the drawn task rows.
fn task_rows(state: &TuiState) -> Vec<String> {
    let (width, height) = snapshot_size();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal
        .draw(|frame| render(state, frame))
        .expect("draw the frame");
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .filter(|row| row.contains("task "))
        .collect()
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let scenario = std::env::args().nth(1).unwrap_or_else(|| "progress".into());
    let root = tempfile::tempdir().expect("a temporary session root");
    let mut state = TuiState::default();
    let mut terminal =
        Terminal::new(CrosstermBackend::new(std::io::stdout())).expect("a real terminal");
    // One registry for the whole run, because a session has one. Two registries would mint
    // the same task id twice, and the reducer matches a row by id.
    let registry = Arc::new(TaskRegistry::new(TaskLimits::default()));

    // Two passes. A defect that needs the second run has bitten this project twice.
    for pass in 1..=2 {
        match command_for(&scenario) {
            Some(command) => {
                run_one_task(&mut state, &mut terminal, root.path(), &command, &registry).await;
            }
            None => {
                raw_rows(&mut state, pass);
                draw(&mut state, &mut terminal);
            }
        }
    }

    // The row text, for the harness. Written after the last draw.
    let mut out = std::io::stdout();
    let _ = write!(out, "\r\n");
    for row in task_rows(&state) {
        let _ = write!(out, "ROW|{row}\r\n");
    }
    let _ = write!(out, "DONE|{scenario}\r\n");
    let _ = out.flush();
}

/// Start one background command and fold every event it broadcasts into the state.
async fn run_one_task(
    state: &mut TuiState,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    root: &std::path::Path,
    command: &str,
    registry: &Arc<TaskRegistry>,
) {
    // The real bridge, the one `rho-cli` wires into the interface. It subscribes inside
    // the call, so a task started right afterwards still reports its start.
    let mut events = registry.session_events();
    let tool = BashTool::with_tasks(Arc::clone(registry));
    let (updates, mut updates_rx) = tokio::sync::mpsc::channel::<String>(64);
    let (agent_events, _agent_rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
    let ctx = ToolContext {
        session_root: root.to_path_buf(),
        cancel: CancelToken::new(),
        updates,
        agent_events,
    };
    tokio::spawn(async move { while updates_rx.recv().await.is_some() {} });
    let started = tool
        .execute(
            serde_json::json!({ "command": command, "run_in_background": true }),
            ctx,
        )
        .await;
    if let Err(error) = &started {
        eprintln!("the tool refused the command: {error}");
    }
    // Fold every event until the task reaches its final state.
    while let Some(event) = events.next().await {
        let final_event = matches!(event, AgentEvent::TaskEnd { .. });
        state.apply(&event, now_millis());
        draw(state, terminal);
        if final_event {
            break;
        }
    }
}

/// Two task rows built with no reducer between, the way another frontend builds one.
///
/// `Row` is public, so this is a real path and not a test trick. The first row carries a
/// live escape sequence. The second carries ten thousand characters.
fn raw_rows(state: &mut TuiState, pass: usize) {
    let hostile = format!("\u{1b}[2J\u{1b}[H\u{1b}]0;pwned\u{7}wiped pass {pass}",);
    state.rows.push(Row::Task {
        id: format!("raw-{pass}"),
        command: "raw".to_string(),
        state: "running".to_string(),
        finished: false,
        failed: false,
        progress: hostile,
    });
    state.rows.push(Row::Task {
        id: format!("long-{pass}"),
        command: "long".to_string(),
        state: "running".to_string(),
        finished: false,
        failed: false,
        progress: "z".repeat(10_000),
    });
    // A progress report through the reducer as well, so both bounds run on this pass. It
    // targets the `long` row, not the hostile one: an event on the hostile row would replace
    // its raw progress with a sanitised summary, and the row's own filter would go untested.
    state.apply(
        &AgentEvent::TaskProgressed {
            id: TaskId(format!("long-{pass}")),
            progress: TaskProgress {
                percent: Some(50),
                message: Some("y".repeat(10_000)),
                done: None,
                total: None,
            },
        },
        now_millis(),
    );
}

fn draw(state: &mut TuiState, terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) {
    terminal
        .draw(|frame| render(state, frame))
        .expect("draw the frame");
}
