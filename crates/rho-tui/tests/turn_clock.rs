//! The turn clock and the working state. See `SPEC-the-turn-clock-and-the-working-state`.
//!
//! These tests prove the live duration counter grows, the sweep animates only with motion
//! on, the amber cue fires past one minute, and a failed or cancelled run freezes the
//! clock. The reducer tests pass milliseconds as data and read no clock. The render tests
//! assert against the real frame. No test sleeps. No test uses the network.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Style};
use rho_core::{AgentEvent, AgentStopReason};
use rho_tui::{ActivityState, TuiState, render};

/// The foreground colour `Role::Warn` resolves to in 256-colour mode.
const WARN: Color = Color::Indexed(179);

/// Render and return the cells of one row as (symbol, style).
fn row_styles(state: &TuiState, width: u16, height: u16, row: u16) -> Vec<(String, Style)> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("build test terminal");
    terminal
        .draw(|frame| render(state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    (0..width)
        .map(|x| {
            let cell = &buffer[(x, row)];
            (cell.symbol().to_string(), cell.style())
        })
        .collect()
}

/// A running turn, with the tick and motion choice set by hand for a render test.
fn running(animate: bool, tick: u64) -> TuiState {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart, 0);
    assert_eq!(state.activity, ActivityState::Running);
    state.animate = animate;
    state.tick = tick;
    state
}

/// The footer row of a rendered frame, as plain text.
fn footer_text(state: &TuiState) -> String {
    let mut terminal = Terminal::new(TestBackend::new(80, 10)).expect("a test terminal");
    terminal
        .draw(|frame| render(state, frame))
        .expect("the frame draws");
    let buffer = terminal.backend().buffer().clone();
    (0..80)
        .map(|x| buffer[(x, 9)].symbol().to_string())
        .collect::<String>()
}

// ---- Reducer: the tick and the live clock. ------------------------------------

#[test]
fn on_tick_advances_the_tick() {
    let mut state = TuiState::default();
    state.on_tick(100);
    assert_eq!(state.tick, 1);
}

#[test]
fn on_tick_grows_the_live_turn_duration() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart, 0);
    state.on_tick(2_500);
    assert_eq!(state.turn_millis, Some(2_500));
}

#[test]
fn turn_start_resets_the_live_clock_to_zero() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart, 0);
    state.on_tick(5_000);
    assert_eq!(state.turn_millis, Some(5_000));
    // A new turn resets the live clock, so the footer shows 0s, not the last turn's 5s.
    state.apply(&AgentEvent::TurnStart, 5_000);
    assert_eq!(state.turn_millis, Some(0));
}

#[test]
fn on_tick_while_idle_leaves_the_turn_clock() {
    let mut state = TuiState::default();
    assert_eq!(state.activity, ActivityState::Idle);
    assert_eq!(state.turn_millis, None);
    state.on_tick(100);
    assert_eq!(state.tick, 1);
    assert_eq!(
        state.turn_millis, None,
        "an idle tick must not touch the turn clock"
    );
}

#[test]
fn a_failed_run_freezes_the_final_duration() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart, 0);
    state.on_tick(3_000);
    state.end_run(true, 3_200);
    assert_eq!(state.turn_millis, Some(3_200));
    assert!(state.last_error);
    let text = footer_text(&state);
    assert!(
        text.contains("error"),
        "the footer names the failure: {text}"
    );
    assert!(
        text.contains("3.2s"),
        "the footer carries the frozen duration: {text}"
    );
}

#[test]
fn a_canceled_run_freezes_the_final_duration() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart, 0);
    // A Ctrl-C while running arms the cancel word the footer draws until the run ends.
    state.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('c'),
        crossterm::event::KeyModifiers::CONTROL,
    ));
    assert!(state.canceling);
    state.end_run(false, 4_000);
    assert_eq!(state.turn_millis, Some(4_000));
    assert_eq!(state.last_stop, Some(AgentStopReason::Canceled));
    let text = footer_text(&state);
    assert!(
        text.contains("canceled"),
        "the footer names the cancel: {text}"
    );
}

// ---- Render: the sweep animates only with motion on. --------------------------

/// The styles of the working-word cells in the footer. The word `working` starts at
/// column 4: two margin columns, the activity glyph, one space.
fn word_styles(animate: bool, tick: u64) -> Vec<Style> {
    let state = running(animate, tick);
    let cells = row_styles(&state, 80, 10, 9);
    cells[4..11].iter().map(|(_, s)| *s).collect()
}

#[test]
fn no_motion_word_is_identical_across_ticks() {
    let at_zero = word_styles(false, 0);
    let at_seven = word_styles(false, 7);
    assert_eq!(
        at_zero, at_seven,
        "with motion off the word sits still across ticks"
    );
}

#[test]
fn motion_word_changes_across_ticks() {
    let at_zero = word_styles(true, 0);
    let at_seven = word_styles(true, 7);
    assert_ne!(
        at_zero, at_seven,
        "with motion on the sweep moves the word styles across ticks"
    );
}

// ---- Render: the amber cue. ---------------------------------------------------

/// The style of the duration cells in a running footer. The duration follows
/// `◈ working · `, so it starts at column 14.
fn duration_style(turn_millis: i64) -> Style {
    let mut state = running(true, 0);
    state.turn_millis = Some(turn_millis);
    let cells = row_styles(&state, 80, 10, 9);
    // The duration is the first non-space run at or after column 14.
    let mut start = 14;
    while start < cells.len() && cells[start].0 == " " {
        start += 1;
    }
    let mut end = start;
    while end < cells.len() && cells[end].0 != " " {
        end += 1;
    }
    assert!(end > start, "no duration cell found in the footer");
    // Every duration cell carries one role, so the first cell speaks for the run.
    cells[start].1
}

#[test]
fn live_duration_over_a_minute_renders_amber() {
    let style = duration_style(61_000);
    assert_eq!(
        style.fg,
        Some(WARN),
        "a live turn past one minute paints its duration amber"
    );
}

#[test]
fn live_duration_under_a_minute_is_normal() {
    let style = duration_style(59_000);
    assert_ne!(
        style.fg,
        Some(WARN),
        "a live turn under one minute keeps the normal duration role"
    );
}

#[test]
fn finished_duration_over_a_minute_is_not_amber() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart, 0);
    state.turn_millis = Some(61_000);
    // End the run with a real stop. The footer now draws the `done` branch, not the
    // running branch, and the duration keeps the normal role whatever its value.
    state.apply(
        &AgentEvent::AgentEnd {
            stop_reason: AgentStopReason::EndTurn,
        },
        61_000,
    );
    assert_eq!(state.activity, ActivityState::Idle);
    let text = footer_text(&state);
    assert!(text.contains("1m 01s"), "the duration still shows: {text}");
    // The duration cells are not amber once the turn has ended.
    let cells = row_styles(&state, 80, 10, 9);
    let dur_cells = cells
        .iter()
        .filter(|(sym, _)| !sym.is_empty() && sym != " ")
        .map(|(_, s)| s.fg);
    assert!(
        !dur_cells.clone().any(|fg| fg == Some(WARN)),
        "a finished turn does not paint its duration amber, even past one minute"
    );
}

// ---- The wiring guard. -------------------------------------------------------
//
// What no unit test can reach is the `select!` arm, because the loop owns a real
// terminal. This test reads the loop and states the rule where a contributor meets it,
// in the shape of `the_event_loop_reads_the_task_stream`. The live drive is the other
// half. See `SPEC-the-turn-clock-and-the-working-state` amendment 2.

#[test]
fn the_event_loop_advances_the_clock_while_a_turn_runs() {
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
    // The tick arm fires only while a run is live, so an idle screen never repaints on a
    // timer, and it advances the reducer.
    assert!(
        loop_body.contains("ticker.tick()"),
        "the event loop must have a tick arm, or the clock never grows"
    );
    assert!(
        loop_body.contains(
            "events.is_some() => {
                    state.on_tick("
        ),
        "the tick arm must be guarded by a live run and call the reducer"
    );
    // The submit arm resets the interval, so the first live tick lands one full period
    // after the turn begins.
    assert!(
        loop_body.contains("ticker.reset()"),
        "a new turn must reset the ticker, so the cadence is steady"
    );
    // The loop owns the timer, not the reducer.
    assert!(
        source.contains("TICK_PERIOD_MILLIS"),
        "the loop must read the tick period from the motion module"
    );
}
