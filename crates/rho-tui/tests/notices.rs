//! A startup notice must reach the screen the user is about to look at.
//!
//! rho printed its startup notices to the terminal, and then opened the alternate screen
//! over them. Measured on the release binary: the notices wrote at byte 5 and byte 320,
//! and the alternate screen opened at byte 535. So every notice was on screen for a few
//! milliseconds, on a buffer the user never sees again.
//!
//! One of the hidden lines says a project skill stays unloaded until the user trusts it.
//! That is a security notice, and silence was the worst possible outcome for it.
//!
//! No test could catch this before, because the fault was an ordering rule between two
//! crates: `rho-cli` owned the notices, and `rho-tui` owned the screen. These tests pin the
//! contract that now joins them. See `D-a-notice-reaches-the-transcript` and
//! `SPEC-tui-alternate-screen` section 6c.

use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rho_tui::{App, Row, TuiState, render};

// ---- The row itself. -----------------------------------------------------------

#[test]
fn a_notice_becomes_a_transcript_row() {
    let mut state = TuiState::default();
    state.push_notice("no model given, so using the default");
    match state.rows.first() {
        Some(Row::Notice { message }) => {
            assert_eq!(message, "no model given, so using the default");
        }
        other => panic!("expected a notice row, got {other:?}"),
    }
}

#[test]
fn a_notice_is_not_an_error() {
    // A default model and an unloaded skill are worth saying, and neither one failed.
    // Reusing the error row would have told the user that rho broke.
    let mut state = TuiState::default();
    state.push_notice("a project skill stays unloaded");
    assert!(
        !matches!(state.rows.first(), Some(Row::Error { .. })),
        "a notice must not claim an error"
    );
}

#[test]
fn a_notice_row_is_sanitised() {
    // A notice text comes from a skill file and a provider name, so it is not trusted
    // input. An escape sequence in a row would rewrite the screen rho just drew.
    let mut state = TuiState::default();
    state.push_notice("a\u{1b}[31mred\u{7} notice");
    match state.rows.first() {
        Some(Row::Notice { message }) => {
            assert!(!message.contains('\u{1b}'), "escape survived: {message:?}");
            assert!(!message.contains('\u{7}'), "bell survived: {message:?}");
        }
        other => panic!("expected a notice row, got {other:?}"),
    }
}

// ---- The invariant: the pairing is complete, and ordered. -----------------------

#[test]
fn every_notice_reaches_the_transcript_in_order() {
    // This asserts the pairing, not one example. The defect dropped *all* of them, and a
    // future one is as likely to drop the last as the first.
    let notices = [
        "no model given, so using the default for openrouter",
        "10 skills: the frontmatter sets allowed-tools",
        "1 project skill was found and not loaded: tui-design",
    ];
    let mut state = TuiState::default();
    for notice in notices {
        state.push_notice(notice);
    }
    let seen: Vec<&str> = state
        .rows
        .iter()
        .filter_map(|row| match row {
            Row::Notice { message } => Some(message.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        seen, notices,
        "every notice reaches the transcript, in the caller's order"
    );
}

// ---- The app seeds them, which is the wiring that was missing. ------------------

#[test]
fn the_app_seeds_its_notices_into_the_transcript() {
    let app = App::new(test_session(), "sonnet-4.5").with_notices([
        "no model given, so using the default",
        "1 project skill was found and not loaded",
    ]);
    let seen: Vec<&str> = app
        .live_rows()
        .iter()
        .filter_map(|row| match row {
            Row::Notice { message } => Some(message.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        seen,
        [
            "no model given, so using the default",
            "1 project skill was found and not loaded"
        ],
        "the app carries its notices onto the screen it opens"
    );
}

#[test]
fn an_app_with_no_notice_shows_no_notice_row() {
    let app = App::new(test_session(), "sonnet-4.5");
    assert!(
        !app.live_rows()
            .iter()
            .any(|row| matches!(row, Row::Notice { .. })),
        "a quiet startup stays quiet"
    );
}

// ---- The drawing. --------------------------------------------------------------

#[test]
fn a_notice_row_draws_its_text_and_says_notice() {
    let mut state = TuiState::default();
    state.push_notice("1 project skill was found and not loaded");
    let drawn = render_rows(&state, 100).join("\n");
    assert!(
        drawn.contains("1 project skill was found and not loaded"),
        "the notice text must reach the screen:\n{drawn}"
    );
    assert!(
        drawn.contains("notice"),
        "the row names itself, so the user knows it is not an answer:\n{drawn}"
    );
    assert!(
        !drawn.contains("error"),
        "a notice must not draw as an error:\n{drawn}"
    );
}

// ---- The splash must survive a notice, and a notice must never be hidden. -------

#[test]
fn the_splash_survives_a_few_notices() {
    // A notice is chrome, not conversation. Seeding one used to empty the splash, because
    // the splash was gated on `rows.is_empty()`. Every real startup in this repository
    // raises a skill notice, so the splash would have vanished for good.
    let mut state = TuiState::default();
    state.push_notice("1 project skill was found and not loaded");
    let drawn = render_rows(&state, 100).join("\n");
    assert!(
        drawn.contains("the harness, unbundled"),
        "the splash still draws beside a notice:\n{drawn}"
    );
    assert!(
        drawn.contains("1 project skill was found and not loaded"),
        "and the notice still draws:\n{drawn}"
    );
}

#[test]
fn many_notices_stay_reachable_instead_of_truncated() {
    // The splash block is capped to the window, so folding notices into it would hide any
    // that overflow. That would rebuild the very defect this file exists to prevent. When
    // the notices cannot fit, rho must fall back to the scrollable transcript.
    let mut state = TuiState::default();
    for index in 0..40 {
        state.push_notice(format!("notice number {index}"));
    }
    let (total, visible) = rho_tui::transcript_metrics(&state, 100, 24);
    assert!(visible > 0, "the window has rows");
    assert!(
        total > visible,
        "the transcript must report more rows than fit, so the wheel can reach them: \
         total {total}, visible {visible}"
    );
}

#[test]
fn a_long_notice_keeps_its_tail() {
    // The real skill notice ends with the action, "Pass --trust-project to load them."
    // A padded single line clipped exactly that part at the screen edge, so the user read a
    // warning and never read what to do about it.
    //
    // This test first asserted that one exact phrase appeared in the drawn rows. That was
    // wrong: a wrap may split the phrase across two rows, and it did. The assertion now
    // pins the real invariant, which is that no word is lost. It rejoins the drawn rows and
    // compares the whole text, so a clip anywhere fails, not only a clip at the tail.
    let notice = "1 project skill(s) were found and not loaded: tui-design. A skill can \
                  instruct the model and can carry scripts, so a skill from this repository \
                  stays off until you trust it. Pass --trust-project to load them.";
    let mut state = TuiState::default();
    state.push_notice(notice);
    let drawn = render_rows(&state, 100).join(" ");
    let flat = drawn.split_whitespace().collect::<Vec<_>>().join(" ");
    let want = notice.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains(&want),
        "the notice must wrap and keep every word.\nwant: {want}\ngot:  {flat}"
    );
}

/// The rendered text of a state, one string per row.
fn render_rows(state: &TuiState, width: u16) -> Vec<String> {
    let rows = 24u16;
    let backend = TestBackend::new(width, rows);
    let mut terminal = Terminal::new(backend).expect("build test terminal");
    terminal
        .draw(|frame| render(state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    (0..rows)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
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
