//! Screen setup, the banner, and the mouse. rho owns the whole terminal in the alternate
//! screen, so the inline band and its freeze are gone. These tests guard the escape
//! sequences, the banner content, the mouse default, and the viewport origin. See
//! `SPEC-tui-alternate-screen` sections 2, 6b, and 8.

use rho_core::{AgentEvent, ToolKind, ToolOutput};
use rho_tui::{App, Row, TuiState, banner_line, render, restore_sequences, setup_sequences};

// ---- Section 2: the setup and restore sequences. -------------------------------

#[test]
fn the_setup_enters_the_alternate_screen() {
    assert!(
        setup_sequences(false).contains("?1049h"),
        "rho enters the alternate screen at startup"
    );
    assert!(setup_sequences(true).contains("?1049h"));
}

#[test]
fn the_restore_matches_the_setup() {
    assert!(setup_sequences(false).contains("?1049h"));
    assert!(restore_sequences(false).contains("?1049l"));
    for mode in ["1000", "1002", "1003", "1006"] {
        let enabled = setup_sequences(true).contains(&format!("{mode}h"));
        let disabled = restore_sequences(true).contains(&format!("{mode}l"));
        assert_eq!(
            enabled, disabled,
            "mode {mode} must pair a setup with a restore"
        );
    }
}

// ---- Section 8: the mouse. -----------------------------------------------------

#[test]
fn mouse_off_adds_no_capture() {
    let setup = setup_sequences(false);
    assert!(setup.contains("?1049h"), "the setup enters the screen");
    assert!(
        !setup.contains("1006h"),
        "with mouse off, drag-select still works"
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
fn the_mouse_is_on_by_default() {
    // In the alternate screen the wheel is the only way to scroll, so capture is on by
    // default. A user whose terminal loses selection passes `--no-mouse`. See section 8.
    let app = App::new(test_session(), "sonnet-4.5");
    assert!(
        app.setup_sequence().contains("1006h"),
        "the app built with no flag captures the mouse by default"
    );
}

#[test]
fn with_mouse_false_reaches_the_setup() {
    let app = App::new(test_session(), "sonnet-4.5").with_mouse(false);
    assert!(
        !app.setup_sequence().contains("1006h"),
        "the --no-mouse flag turns capture off"
    );
}

// ---- The banner still carries real context. ------------------------------------

#[test]
fn the_banner_holds_the_cwd_and_the_model() {
    let mut state = TuiState::default();
    state.set_context("~/Work/Vibe/rho", "main", "openrouter");
    state.model = "sonnet-4.5".to_string();
    let banner = banner_line(&state, 100);
    assert!(
        banner.contains("~/Work/Vibe/rho"),
        "the banner holds the cwd"
    );
    assert!(banner.contains("sonnet-4.5"), "the banner holds the model");
}

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

#[test]
fn the_banner_draws_at_the_top_of_the_screen() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut state = TuiState::default();
    state.set_context("~/Work/Vibe/rho", "main", "openrouter");
    state.model = "sonnet-4.5".to_string();
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).expect("a test terminal");
    terminal.draw(|frame| render(&state, frame)).expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let top: String = (0..100).map(|x| buffer[(x, 0)].symbol()).collect();
    assert!(
        top.contains("~/Work/Vibe/rho"),
        "the banner must draw at the top row, and it was {top:?}"
    );
}

// ---- The renderer draws where the viewport is, not at (0, 0). -------------------

#[test]
fn the_screen_draws_at_the_viewport_origin() {
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
            viewport: Viewport::Fixed(Rect::new(0, 2, 100, 14)),
        },
    )
    .expect("the terminal builds");

    terminal
        .draw(|frame| render(&state, frame))
        .expect("the screen must draw inside its own area");

    let buffer = terminal.backend().buffer();
    let drew_below_the_origin = (2..16).any(|y| (0..100).any(|x| buffer[(x, y)].symbol() == "─"));
    assert!(
        drew_below_the_origin,
        "the composer rules must draw inside the viewport area"
    );
}

// ---- A tool row promises no key that does not exist. ---------------------------

#[test]
fn a_tool_row_draws_no_fold_caret() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
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
    state.apply(
        &AgentEvent::AgentEnd {
            stop_reason: rho_core::AgentStopReason::EndTurn,
        },
        700,
    );

    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).expect("a test terminal");
    terminal.draw(|frame| render(&state, frame)).expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let joined: String = (0..24)
        .map(|y| {
            (0..100)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    for caret in ["▸", "▾"] {
        assert!(
            !joined.contains(caret),
            "a tool row must draw no fold caret while no key folds it, got:\n{joined}"
        );
    }
    assert!(joined.contains("0.5s"), "the duration still draws");
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

// ---- The banner is untrusted text too. ------------------------------------------

#[test]
fn the_banner_sanitises_every_field_it_joins() {
    // A security review found `banner_line` joining the directory, the branch, the model, and the
    // provider with no filter, while every other row had one.
    //
    // The directory is the realistic vector: a name on a Unix filesystem may hold an escape byte, so
    // cloning a hostile archive and running rho inside it would put that byte on the banner. Git
    // rejects a control character in a ref name, so a branch is safer, and a model id comes from a
    // flag or a config file.
    //
    // Nothing escaped today, because ratatui drops an escape from a cell. That is a second filter and
    // not rho's, and the invariant is that one filter of rho's own guards the terminal on every path.
    let mut state = TuiState::default();
    state.set_context("/tmp/\u{1b}[2Jevil", "main\u{7}bell", "prov\u{202e}ider");
    state.model = "model\u{1b}[31m".to_string();
    let banner = banner_line(&state, 120);
    assert!(
        !banner.contains('\u{1b}'),
        "no escape reaches the banner: {banner:?}"
    );
    assert!(
        !banner.contains("[2J"),
        "and a dropped sequence takes its parameters: {banner:?}"
    );
    assert!(!banner.contains('\u{7}'), "no bell either: {banner:?}");
    assert!(
        !banner.contains('\u{202e}'),
        "and no bidi override: {banner:?}"
    );
    // The readable parts survive, so the filter is not a blunt instrument.
    assert!(banner.contains("evil"), "the real text stays: {banner:?}");
    assert!(banner.contains("main"), "and the branch: {banner:?}");
}
