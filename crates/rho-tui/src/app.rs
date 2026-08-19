//! The interactive event loop.
//!
//! The loop owns the terminal for its lifetime. It reads two sources: terminal
//! input and the agent event stream. It never blocks one on the other. It uses
//! `tokio::select!` over a `crossterm` event stream and the `rho_core`
//! `AgentEvents` stream. See `SPEC-tui` section 5.
//!
//! rho draws an inline band and never enters the alternate screen. Each final row
//! leaves the band for the terminal's scrollback through `Terminal::insert_before`.
//! See `SPEC-tui-inline-and-composer` and `D-inline-viewport-not-alternate-screen`.

use std::io::{self, Stdout, Write};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crossterm::event::{Event, EventStream, KeyEventKind, MouseButton, MouseEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Style;
use ratatui::{Terminal, TerminalOptions, Viewport};
use rho_core::{AgentEvent, AgentEvents, CancelToken, ContentBlock, Session};

use crate::editor::{editor_argv, editor_command};
use crate::render::{
    FreezeBatch, band_rows, banner_freeze, composer_text_width, freeze_all, help_visible_rows,
    next_freeze, render,
};
use crate::slash_row_index;
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

/// The escape sequences that start the interface. `mouse` adds mouse capture.
///
/// The sequences are data, so a test reads them with no terminal. Neither sequence
/// enters the alternate screen, because the band lives in the main screen.
pub fn setup_sequences(mouse: bool) -> String {
    if mouse {
        // Report button presses, drags, and the wheel, in SGR form.
        "\u{1b}[?1000h\u{1b}[?1002h\u{1b}[?1003h\u{1b}[?1006h".to_string()
    } else {
        String::new()
    }
}

/// The sequences that give the terminal back. It disables only what the setup enabled.
///
/// It mirrors `setup_sequences`, so it turns off no mode the setup skipped.
pub fn restore_sequences(mouse: bool) -> String {
    if mouse {
        "\u{1b}[?1006l\u{1b}[?1003l\u{1b}[?1002l\u{1b}[?1000l".to_string()
    } else {
        String::new()
    }
}

/// The interactive TUI app. It owns the session and the current run state.
pub struct App {
    state: TuiState,
    session: Session,
    cancel: Option<CancelToken>,
    events: Option<AgentEvents>,
    /// Whether the app captures the mouse. Off by default, so native selection works.
    mouse: bool,
    /// The session start, which is the clock the reducer folds with. The reducer reads no
    /// clock itself, so time arrives as data. See `D-the-reducer-owns-the-row-metadata`.
    started: Instant,
}

impl App {
    /// Build an app for a session. The model id shows on the banner.
    pub fn new(session: Session, model: impl Into<String>) -> Self {
        let mut state = TuiState::default();
        state.model = model.into();
        state.status = "type a prompt, then press Enter".to_string();
        Self {
            state,
            session,
            cancel: None,
            events: None,
            mouse: false,
            started: Instant::now(),
        }
    }

    /// State the session context the banner reports: the directory, the branch, and the
    /// provider. Without it the banner draws separators around empty fields.
    pub fn with_context(
        mut self,
        cwd: impl Into<String>,
        branch: impl Into<String>,
        provider: impl Into<String>,
    ) -> Self {
        self.state.set_context(cwd, branch, provider);
        self
    }

    /// Turn mouse capture on. Off by default. On, rho gets the wheel and the clickable
    /// slash list, and the user loses drag-select.
    pub fn with_mouse(mut self, enabled: bool) -> Self {
        self.mouse = enabled;
        self
    }

    /// Set how the TUI draws reasoning. Default is `Summary`.
    pub fn with_reasoning(mut self, mode: rho_core::ReasoningDisplay) -> Self {
        self.state.reasoning_display = mode;
        self
    }

    /// The escape sequences this app writes at startup. A test reads the wiring.
    pub fn setup_sequence(&self) -> String {
        setup_sequences(self.mouse)
    }

    /// Run the UI until the user exits. Owns the terminal for its lifetime.
    pub async fn run(&mut self) -> Result<(), TuiError> {
        let mut terminal = setup_terminal(self.mouse)?;
        let result = self.event_loop(&mut terminal).await;
        // Always restore the terminal, even after an error.
        let restore = restore_terminal(&mut terminal, self.mouse);
        result.and(restore)
    }

    async fn event_loop(&mut self, terminal: &mut Term) -> Result<(), TuiError> {
        let mut input = EventStream::new();

        let App {
            state,
            session,
            cancel,
            events,
            started,
            mouse,
            ..
        } = self;
        let mouse = *mouse;

        // Freeze the banner once, above the band, then draw the first frame.
        let width = frame_width(terminal)?;
        if let Some(banner) = banner_freeze(state, width, false) {
            insert_batch(terminal, &banner)?;
        }
        freeze_and_draw(terminal, state)?;

        loop {
            tokio::select! {
                maybe_key = input.next() => {
                    match maybe_key {
                        Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
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
                                KeyAction::Exit => {
                                    exit_freeze(terminal, state, cancel)?;
                                    break;
                                }
                                // Edit the draft in the editor, then replace it with the
                                // result. The band leaves the screen for the editor, and it
                                // returns after. A failed run keeps the draft.
                                KeyAction::EditDraft(text) => {
                                    run_editor(terminal, state, mouse, &text)?;
                                }
                            }
                            freeze_and_draw(terminal, state)?;
                        }
                        // A resize must redraw. This arm used to fall into the catch-all
                        // below, so the frame kept the old width until the next key.
                        Some(Ok(Event::Resize(_, _))) => {
                            freeze_and_draw(terminal, state)?;
                        }
                        // A click on a slash row runs that command, so the list the
                        // renderer draws is selectable by mouse as well as by arrow.
                        Some(Ok(Event::Mouse(mouse))) => {
                            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                                let size = terminal
                                    .size()
                                    .map_err(|error| TuiError::Io(error.to_string()))?;
                                if let Some(index) =
                                    slash_row_index(state, size.width, size.height, mouse.row)
                                {
                                    if state.click_slash_row(index) == KeyAction::Exit {
                                        exit_freeze(terminal, state, cancel)?;
                                        break;
                                    }
                                    freeze_and_draw(terminal, state)?;
                                }
                            }
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
                            state.apply(&event, elapsed_millis(started));
                            if ended {
                                *events = None;
                                *cancel = None;
                            }
                            freeze_and_draw(terminal, state)?;
                        }
                        Some(Err(error)) => {
                            // The transcript is the only channel the user reads. This used
                            // to write `state.status`, which no code draws, so a failed run
                            // reported nothing at all.
                            state.push_error(format!("run error: {error}"));
                            // The driver returns on a failed turn with no `AgentEnd`, so the
                            // frontend ends the run itself. Without this the state stays
                            // `Running` and Ctrl-C can never quit again.
                            state.end_run(true);
                            *events = None;
                            *cancel = None;
                            freeze_and_draw(terminal, state)?;
                        }
                        None => {
                            state.end_run(false);
                            *events = None;
                            *cancel = None;
                            freeze_and_draw(terminal, state)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// The milliseconds since the session started. The one clock read in this crate.
fn elapsed_millis(started: &Instant) -> i64 {
    i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)
}

/// The width the next frame draws at. Read once per frame, after any resize.
fn frame_width(terminal: &Term) -> Result<u16, TuiError> {
    terminal
        .size()
        .map(|size| size.width)
        .map_err(|error| TuiError::Io(error.to_string()))
}

/// Freeze every final prefix, then draw the band, in that order.
///
/// A draw that ran first would show a row the scrollback already holds, and the user
/// would read it twice. See `SPEC-tui-inline-and-composer` section 3.3.
fn freeze_and_draw(terminal: &mut Term, state: &mut TuiState) -> Result<(), TuiError> {
    freeze_final_prefix(terminal, state)?;
    // Tell the reducer the wrap width, so a row-motion key wraps like the screen.
    state.composer_width = composer_text_width(frame_width(terminal)?);
    // Tell the reducer how many help rows the band can show, so a scroll key clamps to
    // what the screen can draw. Clamping only at draw time let the offset climb past the
    // last row, and a press back then moved nothing.
    let size = terminal
        .size()
        .map_err(|error| TuiError::Io(error.to_string()))?;
    state.help_visible_rows = help_visible_rows(band_rows(size.height));
    draw(terminal, state)
}

/// Run the editor on the draft, then restore the band.
///
/// The band leaves the screen, so the editor owns the terminal. The editor value comes
/// from the environment. A failed run keeps the draft and pushes one error row.
fn run_editor(
    terminal: &mut Term,
    state: &mut TuiState,
    mouse: bool,
    text: &str,
) -> Result<(), TuiError> {
    let command = editor_command(
        std::env::var("VISUAL").ok().as_deref(),
        std::env::var("EDITOR").ok().as_deref(),
    );
    let argv = editor_argv(&command);
    restore_terminal(terminal, mouse)?;
    edit_draft(state, &argv, text);
    // Re-enter the band, so the session continues where it left off.
    enable_raw_mode().map_err(|error| TuiError::Io(error.to_string()))?;
    let mut stdout = io::stdout();
    stdout
        .write_all(setup_sequences(mouse).as_bytes())
        .and_then(|_| stdout.flush())
        .map_err(|error| TuiError::Io(error.to_string()))?;
    freeze_and_draw(terminal, state)
}

/// Edit `text` in the editor named by `argv`, then apply the result to `state`.
///
/// It writes a temporary file, runs the program with `std::process::Command`, and reads
/// the file back. It never passes the value to a shell. A failed run keeps the draft and
/// pushes one error row, so the user never loses work to a broken editor.
pub fn edit_draft(state: &mut TuiState, argv: &[String], text: &str) {
    let Some((program, args)) = argv.split_first() else {
        state.push_error("no editor command to run");
        return;
    };
    let path = temp_draft_path();
    if let Err(error) = std::fs::write(&path, text) {
        state.push_error(format!("cannot write the editor file: {error}"));
        return;
    }
    let status = Command::new(program).args(args).arg(&path).status();
    match status {
        Ok(status) if status.success() => match std::fs::read_to_string(&path) {
            Ok(content) => {
                let trimmed = content.strip_suffix('\n').unwrap_or(&content);
                state.draft.set_text(trimmed);
            }
            Err(error) => state.push_error(format!("cannot read the editor file: {error}")),
        },
        Ok(status) => state.push_error(format!("the editor exited with {status}")),
        Err(error) => state.push_error(format!("cannot run the editor: {error}")),
    }
    let _ = std::fs::remove_file(&path);
}

/// A unique temporary file path for the editor draft.
///
/// It joins the process id and a nanosecond stamp, so two edits never clash.
fn temp_draft_path() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|span| span.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("rho-draft-{}-{nanos}.txt", std::process::id()))
}

/// Freeze every batch the finality rule allows now. The width is read once for the loop.
///
/// `mark_frozen` runs after the insert that wrote the rows, never before. A failed
/// insert ends the session, and the rows stay unfrozen in the state.
fn freeze_final_prefix(terminal: &mut Term, state: &mut TuiState) -> Result<(), TuiError> {
    let width = frame_width(terminal)?;
    while let Some(batch) = next_freeze(state, width) {
        insert_batch(terminal, &batch)?;
        state.mark_frozen(batch.rows);
    }
    Ok(())
}

/// Freeze every remaining row, whatever its state. Only the exit path may call this.
///
/// A task row and a subagent row outlive their turn, so the finality rule leaves them in
/// the band. After the loop leaves, no event can arrive for them, so they freeze as they
/// stand. Without this the user's scrollback would lose those rows.
fn freeze_remaining(terminal: &mut Term, state: &mut TuiState) -> Result<(), TuiError> {
    let width = frame_width(terminal)?;
    if let Some(batch) = freeze_all(state, width) {
        insert_batch(terminal, &batch)?;
        state.mark_frozen(batch.rows);
    }
    Ok(())
}

/// End the run and freeze every remaining row, so exit leaves each row final.
///
/// It cancels an active run first, then makes every row final, then freezes the rest.
/// A row that never finished freezes as it stands. See section 3.6.
fn exit_freeze(
    terminal: &mut Term,
    state: &mut TuiState,
    cancel: &mut Option<CancelToken>,
) -> Result<(), TuiError> {
    if let Some(token) = cancel.take() {
        token.cancel();
    }
    state.end_run(false);
    // Every remaining row freezes, including a task or a child that outlived the turn.
    // The finality rule must not decide here, because it would leave those rows behind.
    freeze_remaining(terminal, state)
}

/// Write one batch above the band. A zero-row batch still writes its lines.
fn insert_batch(terminal: &mut Term, batch: &FreezeBatch) -> Result<(), TuiError> {
    let height = u16::try_from(batch.lines.len()).unwrap_or(u16::MAX);
    if height == 0 {
        return Ok(());
    }
    terminal
        .insert_before(height, |buf| write_lines(buf, &batch.lines))
        .map_err(|error| TuiError::Io(error.to_string()))
}

/// Write one line per row into an insert buffer, left aligned.
fn write_lines(buf: &mut Buffer, lines: &[String]) {
    for (index, text) in lines.iter().enumerate() {
        let y = buf.area.top() + u16::try_from(index).unwrap_or(0);
        buf.set_string(0, y, text, Style::default());
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

/// Enter raw mode and open the inline band. Never enter the alternate screen.
fn setup_terminal(mouse: bool) -> Result<Term, TuiError> {
    enable_raw_mode().map_err(|error| TuiError::Io(error.to_string()))?;
    let mut stdout = io::stdout();
    stdout
        .write_all(setup_sequences(mouse).as_bytes())
        .and_then(|_| stdout.flush())
        .map_err(|error| TuiError::Io(error.to_string()))?;
    let (_, height) =
        crossterm::terminal::size().map_err(|error| TuiError::Io(error.to_string()))?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(band_rows(height)),
        },
    )
    .map_err(|error| TuiError::Io(error.to_string()))
}

/// Give the terminal back, and leave the cursor below the band.
fn restore_terminal(terminal: &mut Term, mouse: bool) -> Result<(), TuiError> {
    // Clear the band before the modes go back. The last frame holds a composer box and a
    // footer that promises keys rho no longer answers. A live run left
    // `ctrl-c again quits` on screen after rho had exited, which is a lie the user reads.
    // The transcript above the band is ordinary output, so the clear never touches it.
    terminal
        .clear()
        .map_err(|error| TuiError::Io(error.to_string()))?;
    let mut stdout = io::stdout();
    stdout
        .write_all(restore_sequences(mouse).as_bytes())
        .and_then(|_| stdout.flush())
        .map_err(|error| TuiError::Io(error.to_string()))?;
    disable_raw_mode().map_err(|error| TuiError::Io(error.to_string()))?;
    terminal
        .show_cursor()
        .map_err(|error| TuiError::Io(error.to_string()))?;
    Ok(())
}
