//! The interactive event loop.
//!
//! The loop owns the terminal for its lifetime. It reads two sources: terminal
//! input and the agent event stream. It never blocks one on the other. It uses
//! `tokio::select!` over a `crossterm` event stream and the `rho_core`
//! `AgentEvents` stream. See `SPEC-tui` section 5.
//!
//! rho owns the whole terminal in the alternate screen. The transcript scrolls inside
//! rho, and rho can repaint any row. There is no freeze and no `insert_before`. See
//! `SPEC-tui-alternate-screen` and `D-alternate-screen-after-all`.

use std::io::{self, Stdout};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crossterm::event::{Event, EventStream, KeyEventKind, MouseButton, MouseEventKind};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use rho_core::{AgentEvent, AgentEvents, CancelToken, ContentBlock, ModelSelection, Session};

use crate::editor::{editor_argv, editor_command};
use crate::render::{STARTUP_MIN_ROWS, composer_text_width, render, transcript_metrics};
use crate::screen::ScreenGuard;
use crate::scroll::WHEEL_ROWS;
use crate::slash_row_index;
use crate::state::{KeyAction, Row, TuiState};

/// A terminal backed by standard output.
type Term = Terminal<CrosstermBackend<Stdout>>;

/// The error type of the interactive app.
#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    /// A terminal input or output fault. The message states the cause.
    #[error("terminal io error: {0}")]
    Io(String),
    /// The terminal cannot hold the composer's draft row and the footer at startup.
    ///
    /// `need` is `COMPOSER_MIN_ROWS + FOOTER_ROWS`, which is four. This is fatal, and only
    /// at startup. A resize that makes the terminal too small never ends the session,
    /// because dragging a window narrow and wide again is ordinary. See
    /// `SPEC-tui-alternate-screen` section 7.
    #[error("the terminal is {rows} rows, and rho needs at least {need}")]
    TooSmall { rows: u16, need: u16 },
}

/// The escape sequences that start the interface. `mouse` adds mouse capture.
///
/// The sequences are data, so a test reads them with no terminal. rho enters the alternate
/// screen at startup, so this delegates to the screen guard. See
/// `D-alternate-screen-after-all`.
pub fn setup_sequences(mouse: bool) -> String {
    crate::screen::enter_sequences(mouse)
}

/// The sequences that give the terminal back. It mirrors `setup_sequences`.
pub fn restore_sequences(mouse: bool) -> String {
    crate::screen::restore_sequences(mouse)
}

/// The interactive TUI app. It owns the session and the current run state.
pub struct App {
    state: TuiState,
    session: Session,
    cancel: Option<CancelToken>,
    events: Option<AgentEvents>,
    /// Background task events, for the whole life of the interface.
    ///
    /// This is not `AgentEvents`, and it must not become part of one. A task outlives the
    /// tool call that started it and it outlives the run, so a run-scoped stream would
    /// drop the `TaskEnd` of a build that finished while the user was typing. See
    /// `SPEC-the-task-event-bridge` and `D-a-task-event-outlives-its-tool-call`.
    task_events: Option<rho_core::SessionEvents>,
    /// Whether the app captures the mouse. On by default, because in the alternate screen
    /// the wheel is the only way to scroll. See `SPEC-tui-alternate-screen` section 8.
    mouse: bool,
    /// The session start, which is the clock the reducer folds with. The reducer reads no
    /// clock itself, so time arrives as data. See `D-the-reducer-owns-the-row-metadata`.
    started: Instant,
}

impl App {
    /// Build an app for a session. The model id shows on the banner.
    ///
    /// The mouse is on by default, because the wheel is the only way to scroll the
    /// transcript in the alternate screen. A user whose terminal loses selection passes
    /// `--no-mouse`. See `SPEC-tui-alternate-screen` section 8.
    pub fn new(session: Session, model: impl Into<String>) -> Self {
        let mut state = TuiState::default();
        state.model = model.into();
        state.status = "type a prompt, then press Enter".to_string();
        Self {
            state,
            session,
            cancel: None,
            events: None,
            task_events: None,
            mouse: true,
            started: Instant::now(),
        }
    }

    /// Read background task events for the whole life of the interface.
    ///
    /// Without this call the interface draws no task row, whatever the renderer can draw.
    /// That was true of every shipped version until this lane: the renderer folded and drew
    /// a task row, and nothing in a binary ever subscribed to the registry.
    ///
    /// The stream outlives one run on purpose. A build usually ends between two prompts, and
    /// a run-scoped stream would leave that row saying `running` for good.
    ///
    /// See `SPEC-the-task-event-bridge`.
    pub fn with_task_events(mut self, events: rho_core::SessionEvents) -> Self {
        self.task_events = Some(events);
        self
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

    /// Turn mouse capture on or off. On by default. Off, the user keeps drag-select but
    /// loses the wheel, which is the only way to scroll.
    pub fn with_mouse(mut self, enabled: bool) -> Self {
        self.mouse = enabled;
        self
    }

    /// Set whether the working word animates.
    ///
    /// This is the call that was missing. `state.animate` was read by the renderer and
    /// assigned nowhere, so the sweep never drew. A flag, a config key, and
    /// `RHO_REDUCE_MOTION` all arrive here through the merge. See
    /// `D-motion-answers-to-one-switch`.
    pub fn with_motion(mut self, animates: bool) -> Self {
        self.state.animate = animates;
        self
    }

    /// Set how the TUI draws reasoning. Default is `Summary`.
    ///
    /// The mode reaches the renderer through `TuiState`, so a config file, a variable, and a
    /// flag all arrive here through the merge. See `SPEC-reasoning-across-providers` section 5.
    pub fn with_reasoning(mut self, mode: rho_core::ReasoningDisplay) -> Self {
        self.state.reasoning_display = mode;
        self
    }

    /// Seed the starred model list, from the file the frontend read at startup. See
    /// `D-starred-models-live-in-their-own-file`.
    pub fn with_starred_models(mut self, starred: Vec<String>) -> Self {
        self.state.set_starred_models(starred);
        self
    }

    /// Set the initial reasoning effort, mirroring `Session::selection`. Without this,
    /// the header would draw a stale value while the wire uses the mutex.
    pub fn with_reasoning_effort(mut self, effort: Option<rho_core::ReasoningEffort>) -> Self {
        self.state.reasoning_effort = effort;
        self
    }

    /// Seed the startup notices into the transcript, in the order the caller gives them.
    ///
    /// The caller used to print a notice to the terminal, and rho then opened the
    /// alternate screen over it. So the user never read a single one, including the line
    /// that says a project skill stays unloaded until the user trusts it. A notice belongs
    /// on the screen the user is about to look at. See `D-a-notice-reaches-the-transcript`.
    pub fn with_notices<I, S>(mut self, notices: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for notice in notices {
            self.state.push_notice(notice);
        }
        self
    }

    /// The transcript rows the user sees. A frontend or a test reads what rho drew.
    pub fn live_rows(&self) -> &[Row] {
        self.state.live_rows()
    }

    /// The current UI state the renderer draws. A test reads what the builder wired,
    /// so it can drive the real `with_motion`, `with_reasoning`, or `with_context`
    /// chain and render the result, instead of hand-building a `TuiState`.
    pub fn state(&self) -> &TuiState {
        &self.state
    }

    /// The escape sequences this app writes at startup. A test reads the wiring.
    pub fn setup_sequence(&self) -> String {
        setup_sequences(self.mouse)
    }

    /// Run the UI until the user exits. Owns the terminal for its lifetime.
    ///
    /// The screen guard restores the terminal on every exit path: a clean return, an error,
    /// a panic that unwinds through here, and a fatal signal. A terminal too small to hold
    /// the composer and the footer is fatal here, at startup, and only here. See
    /// `D-alternate-screen-after-all` and section 7.
    pub async fn run(&mut self) -> Result<(), TuiError> {
        // Fatal only at startup. A later resize below the minimum draws what fits.
        let (_, height) =
            crossterm::terminal::size().map_err(|error| TuiError::Io(error.to_string()))?;
        if height < STARTUP_MIN_ROWS {
            return Err(TuiError::TooSmall {
                rows: height,
                need: STARTUP_MIN_ROWS,
            });
        }
        let mut guard = ScreenGuard::enter(self.mouse)?;
        // Restore the terminal on SIGTERM, SIGHUP, or SIGINT. The task ends when run does.
        let signals = crate::screen::spawn_signal_restore(self.mouse);
        let mut terminal = build_terminal()?;
        let result = self.event_loop(&mut terminal, &mut guard).await;
        signals.abort();
        // Always restore the terminal, even after an error.
        let restore = restore_terminal(&mut terminal, &mut guard);
        result.and(restore)
    }

    async fn event_loop(
        &mut self,
        terminal: &mut Term,
        guard: &mut ScreenGuard,
    ) -> Result<(), TuiError> {
        let mut input = EventStream::new();

        let App {
            state,
            session,
            cancel,
            events,
            task_events,
            started,
            ..
        } = self;

        draw_frame(terminal, state)?;

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
                                    break;
                                }
                                // Edit the draft in the editor, then replace it with the
                                // result. The screen leaves for the editor and returns.
                                KeyAction::EditDraft(text) => {
                                    run_editor(terminal, guard, state, &text)?;
                                }
                                // A model or effort change from the picker or the slash
                                // command. Apply and mirror into the state, so the header
                                // shows the new model at once. The running turn (if any)
                                // keeps its old selection; the next turn uses the new
                                // one. See `D-model-selection-is-mutable-behind-a-mutex`.
                                KeyAction::ApplySelection(selection) => {
                                    apply_selection(session, state, selection);
                                }
                                // A picker star toggle. Persist to the file the state
                                // now agrees with, so an app crash cannot lose a star.
                                // See `D-starred-models-live-in-their-own-file`.
                                KeyAction::PersistStarred(list) => {
                                    persist_starred(state, &list);
                                }
                            }
                            draw_frame(terminal, state)?;
                        }
                        // A resize must redraw at the new size. A resize below the minimum
                        // draws what fits and never ends the session. See section 7.
                        Some(Ok(Event::Resize(_, _))) => {
                            update_metrics(terminal, state)?;
                            state.scroll_on_resize();
                            draw(terminal, state)?;
                        }
                        // The wheel scrolls the transcript, one row per event. A horizontal
                        // wheel event is ignored by name: a measured session sent 537 of
                        // them from trackpad drift. See section 5 and the spike.
                        Some(Ok(Event::Mouse(mouse))) => {
                            match mouse.kind {
                                MouseEventKind::ScrollUp => {
                                    state.scroll_up(WHEEL_ROWS);
                                    draw_frame(terminal, state)?;
                                }
                                MouseEventKind::ScrollDown => {
                                    state.scroll_down(WHEEL_ROWS);
                                    draw_frame(terminal, state)?;
                                }
                                // ScrollLeft and ScrollRight are ignored by name.
                                MouseEventKind::Down(MouseButton::Left) => {
                                    let size = terminal
                                        .size()
                                        .map_err(|error| TuiError::Io(error.to_string()))?;
                                    if let Some(index) =
                                        slash_row_index(state, size.width, size.height, mouse.row)
                                    {
                                        if state.click_slash_row(index) == KeyAction::Exit {
                                            break;
                                        }
                                        draw_frame(terminal, state)?;
                                    }
                                }
                                _ => {}
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
                            // New output moves a pinned view and leaves an unpinned one.
                            update_metrics(terminal, state)?;
                            state.scroll_on_new_rows();
                            draw(terminal, state)?;
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
                            update_metrics(terminal, state)?;
                            state.scroll_on_new_rows();
                            draw(terminal, state)?;
                        }
                        None => {
                            state.end_run(false);
                            *events = None;
                            *cancel = None;
                            update_metrics(terminal, state)?;
                            state.scroll_on_new_rows();
                            draw(terminal, state)?;
                        }
                    }
                }
                // Background task events, for the whole life of the interface and not for
                // the life of one run. A task outlives the tool call that started it, so
                // this arm is the only way a task row ever reaches a user. It runs while a
                // run is active and while rho sits idle, which is when a build usually
                // ends. See `SPEC-the-task-event-bridge`.
                maybe_task = next_task_event(task_events), if task_events.is_some() => {
                    match maybe_task {
                        Some(event) => {
                            state.apply(&event, elapsed_millis(started));
                            update_metrics(terminal, state)?;
                            state.scroll_on_new_rows();
                            draw(terminal, state)?;
                        }
                        // The registry is gone, so no later task event can arrive. Stop
                        // polling this arm, and leave every drawn row as it is.
                        None => *task_events = None,
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

/// Apply a new model selection to the session, and mirror it into the TUI state so the
/// header and the picker agree with the mutex. See
/// `D-model-selection-is-mutable-behind-a-mutex`.
fn apply_selection(session: &Session, state: &mut TuiState, selection: ModelSelection) {
    session.set_selection(selection.clone());
    state.set_current_selection(&selection);
}

/// Persist the starred list to `~/.rho/starred-models.toml`. A write failure pushes one
/// error row; the in-memory list already agrees with what a next-open would show. See
/// `D-starred-models-live-in-their-own-file`.
fn persist_starred(state: &mut TuiState, list: &[String]) {
    let Some(path) = crate::starred::default_path() else {
        state.push_error("starred-models: HOME is unset, so nothing was saved");
        return;
    };
    if let Err(error) = crate::starred::save(&path, list) {
        state.push_error(format!(
            "starred-models: cannot write {}: {error}",
            path.display()
        ));
    }
}

/// Write the frame geometry into the state, so the reducer clamps a scroll key against the
/// same layout the renderer draws. One source, so the clamp and the draw cannot drift.
fn update_metrics(terminal: &Term, state: &mut TuiState) -> Result<(), TuiError> {
    let size = terminal
        .size()
        .map_err(|error| TuiError::Io(error.to_string()))?;
    state.composer_width = composer_text_width(size.width);
    let (total, visible) = transcript_metrics(state, size.width, size.height);
    state.transcript_total = total;
    state.transcript_visible = visible;
    Ok(())
}

/// Set the metrics, then draw. The common path for a key press.
fn draw_frame(terminal: &mut Term, state: &mut TuiState) -> Result<(), TuiError> {
    update_metrics(terminal, state)?;
    draw(terminal, state)
}

/// Run the editor on the draft, then restore the screen.
///
/// The screen leaves for the editor, so the editor owns the terminal. The editor value
/// comes from the environment. A failed run keeps the draft and pushes one error row.
fn run_editor(
    terminal: &mut Term,
    guard: &mut ScreenGuard,
    state: &mut TuiState,
    text: &str,
) -> Result<(), TuiError> {
    let command = editor_command(
        std::env::var("VISUAL").ok().as_deref(),
        std::env::var("EDITOR").ok().as_deref(),
    );
    let argv = editor_argv(&command);
    // Leave the alternate screen and raw mode, so the editor owns the terminal.
    guard.restore()?;
    edit_draft(state, &argv, text);
    // Re-enter the alternate screen, so the session continues where it left off.
    guard.reenter()?;
    draw_frame(terminal, state)
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

/// Await the next background task event, or wait forever when no stream is wired.
///
/// The `select!` guard skips this branch when `task_events` is `None`, so the pending
/// future never resolves in that case. This mirrors `next_agent_event`, and it is a free
/// function for the same reason: the loop destructures `App`, so it cannot call a method
/// on `self` inside the `select!`.
///
/// `SessionEvents::next` is cancel-safe, so an arm that loses a race loses no event.
async fn next_task_event(events: &mut Option<rho_core::SessionEvents>) -> Option<AgentEvent> {
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

/// Build the ratatui terminal for the full screen.
///
/// The screen guard already entered raw mode and the alternate screen, so this writes no
/// sequences. `Terminal::new` uses the full-screen viewport, so rho owns every row.
fn build_terminal() -> Result<Term, TuiError> {
    let backend = CrosstermBackend::new(io::stdout());
    Terminal::new(backend).map_err(|error| TuiError::Io(error.to_string()))
}

/// Give the terminal back through the guard, and leave the cursor visible.
///
/// It does **not** call `Terminal::clear`. That method snapshots the cursor with
/// `get_cursor_position`, which asks the terminal a question and waits for a reply. On the
/// exit path the reply arrived too late, so the read timed out, rho exited with code 1, and
/// the late reply printed into the user's shell and corrupted the next command.
///
/// Nothing needs clearing here. Leaving the alternate screen discards the whole buffer, so
/// the last frame cannot linger.
fn restore_terminal(terminal: &mut Term, guard: &mut ScreenGuard) -> Result<(), TuiError> {
    guard.restore()?;
    terminal
        .show_cursor()
        .map_err(|error| TuiError::Io(error.to_string()))?;
    Ok(())
}
