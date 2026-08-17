//! The interactive event loop.
//!
//! The loop owns the terminal for its lifetime. It reads two sources: terminal
//! input and the agent event stream. It never blocks one on the other. It uses
//! `tokio::select!` over a `crossterm` event stream and the `rho_core`
//! `AgentEvents` stream. See `SPEC-05` section 5.

use std::io::{self, Stdout};

use crossterm::event::{Event, EventStream, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use rho_core::{AgentEvent, AgentEvents, CancelToken, ContentBlock, Session};

use crate::render::render;
use crate::state::{KeyAction, TuiState};

/// A terminal backed by standard output.
type Term = Terminal<CrosstermBackend<Stdout>>;

/// The error type of the interactive app.
#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    /// A terminal input or output fault. The message states the cause.
    #[error("terminal io error: {0}")]
    Io(String),
}

/// The interactive TUI app. It owns the session and the current run state.
pub struct App {
    state: TuiState,
    session: Session,
    cancel: Option<CancelToken>,
    events: Option<AgentEvents>,
}

impl App {
    /// Build an app for a session. The model id shows on the status line.
    pub fn new(session: Session, model: impl Into<String>) -> Self {
        let mut state = TuiState::default();
        state.model = model.into();
        state.status = "type a prompt, then press Enter".to_string();
        Self {
            state,
            session,
            cancel: None,
            events: None,
        }
    }

    /// Run the UI until the user exits. Owns the terminal for its lifetime.
    pub async fn run(&mut self) -> Result<(), TuiError> {
        let mut terminal = setup_terminal()?;
        let result = self.event_loop(&mut terminal).await;
        // Always restore the terminal, even after an error.
        let restore = restore_terminal(&mut terminal);
        result.and(restore)
    }

    async fn event_loop(&mut self, terminal: &mut Term) -> Result<(), TuiError> {
        let mut input = EventStream::new();
        // Draw the first frame before any token arrives. See F-86.
        draw(terminal, &self.state)?;

        let App {
            state,
            session,
            cancel,
            events,
        } = self;

        loop {
            tokio::select! {
                maybe_key = input.next() => {
                    match maybe_key {
                        Some(Ok(Event::Key(key))) if key.kind != KeyEventKind::Release => {
                            match state.handle_key(key) {
                                KeyAction::None => {}
                                KeyAction::Submit(text) => {
                                    let token = CancelToken::new();
                                    let stream = session.prompt(
                                        vec![ContentBlock::Text { text }],
                                        token.clone(),
                                    );
                                    *cancel = Some(token);
                                    *events = Some(stream);
                                }
                                KeyAction::Cancel => {
                                    if let Some(token) = cancel.as_ref() {
                                        token.cancel();
                                    }
                                }
                                KeyAction::Exit => break,
                            }
                            draw(terminal, state)?;
                        }
                        Some(Ok(_)) => {}
                        Some(Err(error)) => return Err(TuiError::Io(error.to_string())),
                        None => break,
                    }
                }
                maybe_event = next_agent_event(events), if events.is_some() => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            let ended = matches!(event, AgentEvent::AgentEnd { .. });
                            state.apply(&event);
                            if ended {
                                *events = None;
                                *cancel = None;
                            }
                            draw(terminal, state)?;
                        }
                        Some(Err(error)) => {
                            state.status = format!("run error: {error}");
                            *events = None;
                            *cancel = None;
                            draw(terminal, state)?;
                        }
                        None => {
                            *events = None;
                            *cancel = None;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Await the next agent event, or wait forever when no run is active. The
/// `select!` guard skips this branch when `events` is `None`, so the pending
/// future never resolves in that case.
async fn next_agent_event(
    events: &mut Option<AgentEvents>,
) -> Option<Result<AgentEvent, rho_core::Error>> {
    match events {
        Some(stream) => stream.next().await,
        None => std::future::pending().await,
    }
}

/// Draw one frame. Map a terminal fault onto `TuiError::Io`.
fn draw(terminal: &mut Term, state: &TuiState) -> Result<(), TuiError> {
    terminal
        .draw(|frame| render(state, frame))
        .map_err(|error| TuiError::Io(error.to_string()))?;
    Ok(())
}

/// Enter raw mode and the alternate screen. Return a ready terminal.
fn setup_terminal() -> Result<Term, TuiError> {
    enable_raw_mode().map_err(|error| TuiError::Io(error.to_string()))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(|error| TuiError::Io(error.to_string()))?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).map_err(|error| TuiError::Io(error.to_string()))
}

/// Leave the alternate screen and raw mode. Restore the terminal for the user.
fn restore_terminal(terminal: &mut Term) -> Result<(), TuiError> {
    disable_raw_mode().map_err(|error| TuiError::Io(error.to_string()))?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .map_err(|error| TuiError::Io(error.to_string()))?;
    terminal
        .show_cursor()
        .map_err(|error| TuiError::Io(error.to_string()))
}
