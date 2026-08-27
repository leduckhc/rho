//! The sweep must be reachable, and stoppable. See `SPEC-wire-the-dead-switches`.
//!
//! `state.animate` was read by `apply_sweep` and never assigned anywhere, so the sweep
//! never drew. `motion_enabled`, `MotionInputs`, and `sweep_frame` had no production
//! caller at all. See `D-motion-answers-to-one-switch`, which corrects an earlier draft
//! that claimed the opposite.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rho_core::AgentEvent;
use rho_tui::{ActivityState, App, MotionInputs, TuiState, motion_enabled, render};

#[test]
fn motion_is_on_by_default_once_it_is_wired() {
    // The default is on, which is `F-working-motion` as specified. It is new on screen.
    assert!(motion_enabled(MotionInputs {
        tui_motion: true,
        stdout_is_terminal: true,
    }));
}

#[test]
fn the_motion_switch_stops_the_sweep() {
    assert!(!motion_enabled(MotionInputs {
        tui_motion: false,
        stdout_is_terminal: true,
    }));
}

#[test]
fn a_non_terminal_stdout_stops_the_sweep() {
    // A redirected stdout has no cursor to animate, and this rule predates the switch.
    assert!(!motion_enabled(MotionInputs {
        tui_motion: true,
        stdout_is_terminal: false,
    }));
}

/// The footer row of a rendered frame, as plain text.
fn footer(state: &TuiState) -> String {
    let mut terminal = Terminal::new(TestBackend::new(80, 10)).expect("a test terminal");
    terminal
        .draw(|frame| render(state, frame))
        .expect("the frame draws");
    let buffer = terminal.backend().buffer().clone();
    (0..80)
        .map(|x| buffer[(x, 9)].symbol().to_string())
        .collect::<String>()
}

/// A state mid-turn, with the tick that puts the sweep at its brightest.
fn running(animate: bool, tick: u64) -> TuiState {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::TurnStart, 0);
    assert_eq!(state.activity, ActivityState::Running);
    state.animate = animate;
    state.tick = tick;
    state
}

#[test]
fn the_renderer_draws_the_word_either_way() {
    // Motion never carries the state on its own. The word is there with or without it,
    // so a reader who stops the animation loses nothing but the movement.
    for animate in [true, false] {
        let text = footer(&running(animate, 5));
        assert!(
            text.contains("working"),
            "the state is named in words, animate={animate}: {text}"
        );
    }
}

#[test]
fn the_motion_flag_reaches_the_renderer() {
    // The missing call, at the seam that matters. `state.animate` was read here and
    // assigned nowhere, so this comparison held for the wrong reason: both sides were
    // still. A styled cell differs only when the sweep runs.
    let mut on = Terminal::new(TestBackend::new(80, 10)).expect("a terminal");
    let state_on = running(true, 5);
    on.draw(|frame| render(&state_on, frame)).expect("draws");
    let styled_on = on.backend().buffer().clone();

    let mut off = Terminal::new(TestBackend::new(80, 10)).expect("a terminal");
    let state_off = running(false, 5);
    off.draw(|frame| render(&state_off, frame)).expect("draws");
    let styled_off = off.backend().buffer().clone();

    assert_ne!(
        styled_on, styled_off,
        "with motion on, the swept word must render differently from the still one"
    );
}

// ---- The real builder, driven end to end. --------------------------------------
//
// The tests above prove `state.animate -> render`. They set `state.animate` by hand,
// so none of them proves `with_motion -> state.animate`, which is the one call that
// wires motion. A critic changed `with_motion` to ignore its argument and the whole
// suite above stayed green. This test drives the real builder, so it fails when
// `with_motion` stops honouring its argument. It never sets `state.animate` by hand.

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

/// The frame a real `App` draws for a running turn, with motion set through the
/// builder. `state.animate` comes only from `with_motion`. The test sets the activity
/// and the tick by hand, which are not the switch under test, so the working word is
/// on screen and the sweep is at a known position.
fn builder_frame(animates: bool, tick: u64) -> ratatui::buffer::Buffer {
    let app = App::new(test_session(), "sonnet-4.5").with_motion(animates);
    let mut state = app.state().clone();
    state.activity = ActivityState::Running;
    state.tick = tick;
    let mut terminal = Terminal::new(TestBackend::new(80, 10)).expect("a test terminal");
    terminal
        .draw(|frame| render(&state, frame))
        .expect("the frame draws");
    terminal.backend().buffer().clone()
}

#[test]
fn with_motion_reaches_the_renderer_through_the_builder() {
    // The seam the old tests skipped: `with_motion -> state.animate -> render`.
    let on = builder_frame(true, 5);
    let off = builder_frame(false, 5);
    assert_ne!(
        on, off,
        "with_motion(true) must render the swept word differently from with_motion(false)"
    );
}

#[test]
fn with_motion_false_is_a_still_frame_through_the_builder() {
    // `--no-motion` renders a still frame: it does not change as the tick advances.
    assert_eq!(
        builder_frame(false, 5),
        builder_frame(false, 12),
        "with_motion(false) must render a still frame at every tick"
    );
}

#[test]
fn with_motion_true_animates_through_the_builder() {
    // Motion on moves the band, so the frame changes as the tick advances.
    assert_ne!(
        builder_frame(true, 5),
        builder_frame(true, 12),
        "with_motion(true) must animate the word as the tick advances"
    );
}
