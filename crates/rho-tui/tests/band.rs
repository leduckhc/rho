//! Band tests: the inline viewport, the freeze, the banner, and the mouse.
//!
//! rho draws an inline band and gives every final row to the terminal's scrollback. A
//! frozen row can never repaint, so these tests guard the shape the band freezes. See
//! `SPEC-tui-inline-and-composer` sections 1, 2, 3.2, 3.3, 3.6, 4, and 5.

use rho_core::{AgentEvent, AgentStopReason, StreamEvent, ToolKind, ToolOutput};
use rho_tui::{
    App, BAND_ROWS, Row, TuiState, band_rows, banner_freeze, banner_line, freeze_all, live_window,
    next_freeze, plan_band, restore_sequences, setup_sequences,
};

/// A running state, with the header fields the banner needs.
fn running() -> TuiState {
    let mut state = TuiState::default();
    state.cwd = "~/Work/Vibe/rho".to_string();
    state.branch = "main".to_string();
    state.model = "sonnet-4.5".to_string();
    state.provider = "openrouter".to_string();
    state.apply(&AgentEvent::TurnStart, 1_000);
    state
}

fn tool_start(id: &str) -> AgentEvent {
    AgentEvent::ToolStart {
        id: id.to_string(),
        name: "bash".to_string(),
        kind: ToolKind::Execute,
    }
}

fn tool_end(id: &str, failed: bool) -> AgentEvent {
    AgentEvent::ToolEnd {
        id: id.to_string(),
        output: ToolOutput {
            content: vec![rho_core::ContentBlock::Text {
                text: "done".to_string(),
            }],
            is_error: failed,
        },
    }
}

fn text_start() -> AgentEvent {
    AgentEvent::Stream(StreamEvent::TextStart { index: 0 })
}

fn agent_end() -> AgentEvent {
    AgentEvent::AgentEnd {
        stop_reason: AgentStopReason::EndTurn,
    }
}

// ---- Section 5: the setup sequences. -------------------------------------------

#[test]
fn the_setup_never_enters_the_alternate_screen() {
    assert!(
        !setup_sequences(false).contains("?1049h"),
        "the whole architecture rests on the main screen"
    );
    assert!(!setup_sequences(true).contains("?1049h"));
}

#[test]
fn the_restore_matches_the_setup() {
    // The restore disables no mode the setup skipped. With mouse off, both are empty.
    assert_eq!(setup_sequences(false), "");
    assert_eq!(restore_sequences(false), "");
    // With mouse on, the restore turns off exactly the modes the setup turned on.
    for mode in ["1000", "1002", "1003", "1006"] {
        let enabled = setup_sequences(true).contains(&format!("{mode}h"));
        let disabled = restore_sequences(true).contains(&format!("{mode}l"));
        assert_eq!(
            enabled, disabled,
            "mode {mode} must pair a setup with a restore"
        );
    }
}

// ---- Section 2: the band height. -----------------------------------------------

#[test]
fn the_band_leaves_a_row_for_the_shell() {
    assert_eq!(
        band_rows(10),
        9,
        "a short terminal keeps a row for the shell"
    );
    assert_eq!(band_rows(40), BAND_ROWS, "a tall terminal caps at the band");
}

#[test]
fn a_one_row_terminal_gets_one_row() {
    assert_eq!(band_rows(1), 1);
}

#[test]
fn the_band_value_is_fourteen_rows() {
    assert_eq!(BAND_ROWS, 14, "the layout table needs the value");
}

#[test]
fn the_band_degrades_in_the_stated_order() {
    // A five-row band keeps the composer input row, which is the last survivor.
    let band = plan_band(5, 1, 0);
    assert!(band.composer_input, "the input row must be the last to go");
}

// ---- Section 3.2 and 3.3: the freeze. ------------------------------------------

#[test]
fn a_final_row_behind_a_live_row_waits() {
    // A running tool row is live. The user row behind it is final, but it must wait.
    let mut state = running();
    state.apply(&tool_start("call-1"), 1_100);
    state.rows.push(Row::User {
        text: "later".to_string(),
    });
    state.row_durations.push(None);
    assert!(
        next_freeze(&state, 80).is_none(),
        "a live row blocks the rows behind it"
    );
}

#[test]
fn the_freeze_takes_the_longest_final_prefix() {
    // Three final rows, then a live one. The batch covers exactly the three.
    let mut state = running();
    state.apply(&tool_start("c1"), 1_100);
    state.apply(&tool_end("c1", false), 1_200);
    state.apply(&tool_start("c2"), 1_300);
    state.apply(&tool_end("c2", false), 1_400);
    state.apply(&text_start(), 1_500); // the newest assistant row is live
    let batch = next_freeze(&state, 80).expect("a final prefix exists");
    assert_eq!(
        batch.rows, 2,
        "the batch covers the two final tool rows only"
    );
}

#[test]
fn nothing_freezes_twice() {
    let mut state = running();
    state.apply(&tool_start("c1"), 1_100);
    state.apply(&tool_end("c1", false), 1_200);
    state.apply(&agent_end(), 1_300);
    let batch = next_freeze(&state, 80).expect("the first call freezes the row");
    state.mark_frozen(batch.rows);
    assert!(
        next_freeze(&state, 80).is_none(),
        "a second call with no new row freezes nothing"
    );
}

#[test]
fn the_freeze_sanitises_every_line() {
    // A tool preview holding a clear-screen escape must freeze with no escape byte.
    let mut state = running();
    state.apply(&tool_start("c1"), 1_100);
    state.apply(
        &AgentEvent::ToolUpdate {
            id: "c1".to_string(),
            output: "danger\u{1b}[2Jgone".to_string(),
        },
        1_150,
    );
    state.apply(&tool_end("c1", false), 1_200);
    state.apply(&agent_end(), 1_300);
    let batch = next_freeze(&state, 80).expect("the row is final");
    for line in &batch.lines {
        assert!(
            !line.contains('\u{1b}'),
            "an escape byte reached a frozen line: {line:?}"
        );
    }
}

#[test]
fn a_batch_never_ends_with_a_blank_line() {
    // The separator leads the row below it, so a batch ends on content.
    let mut state = running();
    state.rows.push(Row::User {
        text: "hello".to_string(),
    });
    state.row_durations.push(None);
    state.apply(&agent_end(), 1_300);
    let batch = next_freeze(&state, 80).expect("the user row is final");
    let last = batch.lines.last().expect("the batch has lines");
    assert!(
        !last.trim().is_empty(),
        "the last line of a batch must hold text, got {last:?}"
    );
}

#[test]
fn the_freeze_width_is_the_draw_width() {
    // A long line freezes wrapped at the width the caller passes.
    let mut state = running();
    state.rows.push(Row::Assistant {
        text: "word ".repeat(40).trim_end().to_string(),
    });
    state.row_durations.push(None);
    state.apply(&agent_end(), 1_300);
    let narrow = next_freeze(&state, 30).expect("final row");
    for line in &narrow.lines {
        assert!(
            unicode_width::UnicodeWidthStr::width(line.as_str()) <= 30,
            "a line wrapped past the freeze width: {line:?}"
        );
    }
}

// ---- Section 4: the live window. -----------------------------------------------

#[test]
fn the_live_window_keeps_the_newest_lines() {
    let mut state = running();
    for index in 0..8 {
        state.rows.push(Row::User {
            text: format!("row {index}"),
        });
        state.row_durations.push(None);
    }
    let lines = live_window(&state, 80, 3);
    let joined = lines.join("\n");
    assert!(joined.contains("row 7"), "the newest row must be present");
    assert!(!joined.contains("row 0"), "the oldest row must scroll out");
}

// ---- Section 2: the banner. ----------------------------------------------------

#[test]
fn the_banner_holds_the_cwd_and_the_model() {
    let state = running();
    let banner = banner_line(&state, 100);
    assert!(
        banner.contains("~/Work/Vibe/rho"),
        "the banner holds the cwd"
    );
    assert!(banner.contains("sonnet-4.5"), "the banner holds the model");
}

#[test]
fn the_banner_freezes_once() {
    let state = running();
    assert!(
        banner_freeze(&state, 100, false).is_some(),
        "the first startup step freezes the banner"
    );
    assert!(
        banner_freeze(&state, 100, true).is_none(),
        "a second startup step inserts no second banner"
    );
}

// ---- Section 3.6: exit. --------------------------------------------------------

/// Drain every batch the finality rule allows now. Return the frozen lines.
fn drain_freezes(state: &mut TuiState, width: u16) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(batch) = next_freeze(state, width) {
        out.extend(batch.lines);
        state.mark_frozen(batch.rows);
    }
    out
}

#[test]
fn exit_mid_run_freezes_every_row_once() {
    // A streaming turn, then ctrl-d. `end_run` makes every row final, and the freeze
    // covers each row exactly once.
    let mut state = running();
    state.rows.push(Row::User {
        text: "the prompt".to_string(),
    });
    state.row_durations.push(None);
    state.apply(&text_start(), 1_100);
    if let Some(Row::Assistant { text }) = state.rows.last_mut() {
        *text = "a streaming answer".to_string();
    }
    state.apply(&tool_start("c1"), 1_200); // this row never finished

    state.end_run(false); // the exit sequence makes every row final
    let row_count = state.rows.len();
    let lines = drain_freezes(&mut state, 80);

    assert!(state.live_rows().is_empty(), "every row left the band");
    assert_eq!(state.frozen_rows, row_count, "every row is frozen");
    assert!(lines.iter().any(|l| l.contains("the prompt")));
    assert!(lines.iter().any(|l| l.contains("a streaming answer")));
    let prompt_count = lines.iter().filter(|l| l.contains("the prompt")).count();
    assert_eq!(prompt_count, 1, "no row freezes twice");
}

#[test]
fn exit_freezes_a_running_tool_row_as_running() {
    // A tool that never finished freezes as it stands, and it reads as running.
    let mut state = running();
    state.apply(&tool_start("c1"), 1_100);
    state.end_run(false);
    let lines = drain_freezes(&mut state, 80);
    assert!(
        lines.iter().any(|l| l.contains('\u{25cf}')),
        "the running glyph must survive the freeze: {lines:?}"
    );
}

// ---- Section 5: the mouse. -----------------------------------------------------

#[test]
fn capture_is_off_by_default() {
    assert_eq!(
        setup_sequences(false),
        "",
        "capture is off by default, so drag-select works"
    );
}

#[test]
fn with_mouse_enables_capture() {
    assert!(
        setup_sequences(true).contains("1006h"),
        "the mouse flag adds SGR mouse capture"
    );
}

#[test]
fn with_mouse_reaches_the_setup() {
    let app = App::new(test_session(), "sonnet-4.5").with_mouse(true);
    assert!(
        app.setup_sequence().contains("1006h"),
        "the app built with the flag writes the capture sequence"
    );
}

// ---- Section 4.1: a resize redraws the band whole. -----------------------------

#[test]
fn a_resize_redraws_the_band_whole() {
    // The composer border spans the whole width after a resize to a new width.
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let state = running();
    for width in [50u16, 72] {
        let backend = TestBackend::new(width, BAND_ROWS);
        let mut terminal = Terminal::new(backend).expect("a test terminal");
        terminal
            .draw(|frame| rho_tui::render(&state, frame))
            .expect("draw");
        let buffer = terminal.backend().buffer().clone();
        // The composer's top rule locates it. The landmark was `╭` until
        // `D-ledger-wins-the-band` opened the composer's sides, and the assertion below is
        // unchanged: the rule must span the new width.
        let border = (0..BAND_ROWS)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .find(|line| line.starts_with('\u{2500}'))
            .expect("the composer top rule");
        assert_eq!(
            unicode_width::UnicodeWidthStr::width(border.as_str()),
            width as usize,
            "the composer border must span the new width"
        );
    }
}

/// A provider that yields no event. It lets a test build an `App` with no network.
struct SilentProvider;

#[async_trait::async_trait]
impl rho_core::Provider for SilentProvider {
    fn id(&self) -> &str {
        "silent"
    }

    async fn stream(
        &self,
        _request: rho_core::CompletionRequest,
        _cancel: rho_core::CancelToken,
    ) -> Result<rho_core::ProviderStream, rho_core::ProviderError> {
        Ok(Box::pin(futures::stream::empty()))
    }
}

/// Build a session with a silent provider, for a wiring test with no network.
fn test_session() -> rho_core::Session {
    use std::sync::Arc;
    let dir = tempfile::tempdir().expect("a temp session root");
    let provider: Arc<dyn rho_core::Provider> = Arc::new(SilentProvider);
    let tools = Arc::new(rho_core::ToolRegistry::new());
    let hooks = Arc::new(rho_core::HookChain::default());
    let config = rho_core::SessionConfig::new(
        "sonnet-4.5",
        dir.path().to_path_buf(),
        Arc::new(rho_core::AllowAllPolicy),
    );
    let context = rho_core::Context::new(None, tools.specs());
    rho_core::Session::with_config(config, provider, tools, hooks, context)
}

// ---- Exit must lose no row, including one that outlives its turn. ---------------
//
// A task row and a subagent row stay live after the turn ends, by their own contract. So
// the finality rule cannot freeze them, and the exit path must not use that rule. If it
// does, the row never reaches the scrollback and the user's transcript loses it.

#[test]
fn exit_freezes_a_running_task_row() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart, 1_000);
    state.apply(
        &AgentEvent::TaskStart {
            id: rho_core::TaskId("t1".to_string()),
            command: "cargo test --workspace".to_string(),
            reason: rho_core::BackgroundReason::KnownLongRunning,
        },
        1_100,
    );
    state.apply(
        &AgentEvent::AgentEnd {
            stop_reason: AgentStopReason::EndTurn,
        },
        2_000,
    );

    // The running loop must leave it in the band, because the task still reports.
    assert!(
        next_freeze(&state, 100).is_none(),
        "a running task must stay in the band while the session lives"
    );

    // The exit path must take it, because no event can arrive after the loop leaves.
    let batch = freeze_all(&state, 100).expect("exit must freeze the remaining rows");
    assert_eq!(batch.rows, 1, "the task row must reach the scrollback");
    assert!(
        batch.lines.iter().any(|line| line.contains("cargo test")),
        "the frozen line must hold the command, got {:?}",
        batch.lines
    );
}

// ---- The band must draw where the viewport is, not at (0, 0). -------------------
//
// An inline viewport is anchored to the cursor row, so `Frame::area()` has a non-zero
// origin. The ratatui docs say it plainly: code that assumes (0, 0) is correct only for a
// fullscreen viewport. Every earlier test used `TestBackend` through `Terminal::new`, which
// is fullscreen, so 226 tests passed while the real binary panicked on its first frame:
//
//     index outside of buffer: the area is Rect { x: 0, y: 2, width: 100, height: 14 }
//     but index is (0, 0)
//
// This test reproduces the offset origin with a fixed viewport, which needs no pty.

#[test]
fn the_band_draws_at_the_viewport_origin() {
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::{Terminal, TerminalOptions, Viewport};

    let mut state = TuiState::default();
    state.model = "sonnet-4.5".to_string();
    state.rows.push(Row::User {
        text: "a prompt".to_string(),
    });

    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            // The band sits two rows down, exactly as an inline viewport does.
            viewport: Viewport::Fixed(Rect::new(0, 2, 100, 14)),
        },
    )
    .expect("the terminal builds");

    terminal
        .draw(|frame| rho_tui::render(&state, frame))
        .expect("the band must draw inside its own area");

    // The composer's rules prove the band drew, and they must sit inside the viewport. The
    // landmark was `╭` and `╰` until `D-ledger-wins-the-band` opened the composer's sides.
    let buffer = terminal.backend().buffer();
    let drew_below_the_origin = (2..16).any(|y| {
        (0..100).any(|x| {
            let symbol = buffer[(x, y)].symbol();
            symbol == "─"
        })
    });
    assert!(
        drew_below_the_origin,
        "the composer rules must draw inside the viewport area"
    );
}

// ---- The banner must carry real context, not empty separators. ------------------
//
// `banner_line` reads `state.cwd`, `state.branch`, and `state.provider`. Nothing in the
// product ever wrote them, so the live banner read `ρ rho   ·  · model ·`. The renderer drew
// three separators around nothing. A frontend must be able to fill them.

#[test]
fn set_context_fills_the_banner() {
    let mut state = TuiState::default();
    state.model = "sonnet-4.5".to_string();
    state.set_context("~/Work/Vibe/rho", "main", "openrouter");
    let line = banner_line(&state, 100);

    for part in ["~/Work/Vibe/rho", "main", "openrouter", "sonnet-4.5"] {
        assert!(
            line.contains(part),
            "the banner must hold {part}, got {line:?}"
        );
    }
}

#[test]
fn a_banner_with_no_context_holds_no_empty_separator_run() {
    let mut state = TuiState::default();
    state.model = "sonnet-4.5".to_string();
    let line = banner_line(&state, 100);
    assert!(
        !line.contains("·  ·"),
        "an empty field must not draw its separators, got {line:?}"
    );
}

// ---- A row must promise no key that does not exist. -----------------------------
//
// Every tool row drew the fold caret `▸`. No key folds a row: `ctrl-o` and `ctrl-e` are
// out of scope for this spec, and `ctrl-e` now moves the cursor to the end of a row. So the
// caret promised a key nobody can press, which is `D-a-panel-nobody-can-open`. The caret
// returns with the fold keys, and its test returns with it.

#[test]
fn a_tool_row_draws_no_fold_caret() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart, 0);
    state.apply(
        &AgentEvent::ToolStart {
            id: "call-1".to_string(),
            name: "bash".to_string(),
            kind: ToolKind::Execute,
        },
        100,
    );
    state.apply(
        &AgentEvent::ToolEnd {
            id: "call-1".to_string(),
            output: ToolOutput {
                content: vec![rho_core::ContentBlock::Text {
                    text: "ok".to_string(),
                }],
                is_error: false,
            },
        },
        600,
    );

    let batch = freeze_all(&state, 100).expect("the finished row freezes");
    let line = batch.lines.join("\n");
    for caret in ["▸", "▾"] {
        assert!(
            !line.contains(caret),
            "a tool row must draw no fold caret while no key folds it, got {line:?}"
        );
    }
    assert!(
        line.contains("0.5s"),
        "the duration still draws, got {line:?}"
    );
}
