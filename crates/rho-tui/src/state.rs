//! The TUI state and its pure reducer.
//!
//! The state is a value. A pure reducer folds one `AgentEvent` into the state. A
//! pure key handler folds one key press into the state. Neither does IO, so a
//! test drives them with no terminal at all. See `SPEC-tui` sections 1 to 3.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rho_core::{
    AgentEvent, AgentStopReason, StreamEvent, TaskId, TaskProgress, TaskState, ToolKind,
};

use crate::concise::RowFold;

/// One rendered transcript row.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// A finished or streaming user message.
    User { text: String },
    /// A finished or streaming assistant answer.
    Assistant { text: String },
    /// A thinking block. Collapsed to one line by default.
    Thinking { text: String },
    /// A subagent row. It stays after the turn ends, because a child outlives the turn
    /// that spawned it, and because its cost is worth keeping on screen.
    Agent {
        id: u64,
        /// The agent name from its definition, for example `scout`.
        name: String,
        /// How deep in the tree, so a fan-out is legible.
        depth: u32,
        turns: u32,
        /// A short cost summary, for example `1.2k in, 340 out`.
        cost: String,
        finished: bool,
        failed: bool,
        /// The outcome word, once finished.
        outcome: String,
    },
    /// A background task row. It stays after the turn ends, because a task outlives
    /// the turn that started it.
    Task {
        id: String,
        command: String,
        /// The current state, as a short word for the status column.
        state: String,
        /// True once the task reached a final state.
        finished: bool,
        /// True when the task finished and failed. Drives the colour.
        failed: bool,
        /// A short progress summary, for example `42%` or `6/10 compiling`.
        progress: String,
    },
    /// A tool row. Shows the tool name, kind, and status.
    Tool {
        id: String,
        name: String,
        kind: ToolKind,
        status: ToolRowStatus,
        /// The last streamed output line, shown as a preview.
        preview: String,
    },
    /// An error row. The message renders on the glyph line, the detail under it.
    Error {
        /// The one-line error message, shown after the `✗ error ·` glyph.
        message: String,
        /// The verbatim detail lines, indented under the message.
        detail: Vec<String>,
    },
}

/// The status of one tool row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolRowStatus {
    Pending,
    Running,
    Ok,
    Failed,
}

/// Whether the agent is running a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ActivityState {
    #[default]
    Idle,
    Running,
}

/// A transient panel shown above the composer. Only one shows at a time, and it
/// borrows its rows from the transcript. See `docs/tui-design.md` sections 5 and 10.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Panel {
    /// No panel. The resting interface.
    #[default]
    None,
    /// The caution-framed approval prompt.
    Approval(Approval),
    /// The slash-command list, open and filtering.
    SlashList(SlashList),
    /// The shortcut list, generated from the binding table.
    Help,
}

/// The approval prompt content. The command is verbatim, so the user sees exactly
/// what would run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    /// The request line, for example `bash asks to run`.
    pub title: String,
    /// The verbatim command awaiting approval.
    pub command: String,
    /// The elapsed span awaiting approval, for the duration slot.
    pub millis: Option<i64>,
}

/// The slash-list panel state: the typed query and the selected row.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SlashList {
    /// The typed query, with its leading slash, for example `/`.
    pub query: String,
    /// The selected row index into the filtered list.
    pub selected: usize,
}

/// The whole TUI state. A pure function of the events applied so far, plus the
/// local input buffer and one flag for the Ctrl-C exit gate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TuiState {
    pub rows: Vec<Row>,
    pub input: String,
    pub activity: ActivityState,
    pub status: String,
    /// The model id, shown on the status line.
    pub model: String,
    /// True after a first Ctrl-C while idle. A second Ctrl-C then exits.
    pub exit_armed: bool,
    /// Set when the run ends. Drives the status line.
    pub last_stop: Option<AgentStopReason>,
    /// The tool name and id for each tool-call index seen in the stream.
    ///
    /// A `ToolCallEnd` event carries only the block index, not the tool name.
    /// The name and id arrive earlier in the `ToolCallStart` event. The reducer
    /// keeps them here, so a `ToolCallEnd` can push a complete tool row. This is
    /// private, so it is not part of the public state contract.
    tool_call_names: Vec<(u32, String, String)>,
    /// The working directory, shown in the header, for example `~/Work/Vibe/rho`.
    pub cwd: String,
    /// The git branch, shown in the header, for example `main`.
    pub branch: String,
    /// The provider, shown in the header, for example `openrouter`.
    pub provider: String,
    /// The token counts, shown in the header, for example `48.2k in, 3.1k out`.
    pub tokens: String,
    /// The session span, for the header clock. `None` draws an empty slot.
    pub session_millis: Option<i64>,
    /// The turn span, for the footer clock. `None` draws an empty slot.
    pub turn_millis: Option<i64>,
    /// The tick count, advanced by the event loop while a turn runs. The one time
    /// source for the motion sweep, so the render stays a pure function of state.
    pub tick: u64,
    /// Whether the footer working word animates the sweep.
    pub animate: bool,
    /// Whether concise mode collapses new tool rows. Off by default.
    pub concise: bool,
    /// The transient panel, if any.
    pub panel: Panel,
    /// True when the last turn ended in an error, for the footer word.
    pub last_error: bool,
    /// True after a Ctrl-C cancel while a turn runs, until the turn ends. It drives the
    /// footer word, so a cancel the model has not answered yet still shows on screen.
    pub canceling: bool,
    /// The finished span of each row, parallel to `rows`. `None` for a row with no
    /// duration, or a row the index does not reach.
    pub row_durations: Vec<Option<i64>>,
    /// The fold of each tool row, parallel to `rows`. A missing entry is collapsed.
    pub row_folds: Vec<RowFold>,
    /// The expanded body lines of each tool row, parallel to `rows`.
    pub row_bodies: Vec<Vec<String>>,
}

/// What the app must do after a key press. The key handler is pure. It returns
/// the action, so the event loop stays the only place that does IO.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// Do nothing more than the state change already applied.
    None,
    /// Submit the given prompt text to the session.
    Submit(String),
    /// Cancel the running turn.
    Cancel,
    /// Exit the app.
    Exit,
}

impl TuiState {
    /// Fold one agent event into the state. Pure. No IO. See `SPEC-tui` section 3.
    pub fn apply(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::TurnStart => {
                self.activity = ActivityState::Running;
                self.canceling = false;
            }
            AgentEvent::Stream(stream) => self.apply_stream(stream),
            AgentEvent::ToolStart { id, name, kind } => self.on_tool_start(id, name, *kind),
            AgentEvent::ToolUpdate { id, output } => self.on_tool_update(id, output),
            AgentEvent::ToolEnd { id, output } => self.on_tool_end(id, output.is_error),
            AgentEvent::TurnEnd { .. } => {}
            AgentEvent::AgentSpawned { id, agent, depth } => {
                self.on_agent_spawned(id.0, agent, *depth)
            }
            AgentEvent::AgentProgressed { id, turns, usage } => {
                self.on_agent_progressed(id.0, *turns, usage)
            }
            AgentEvent::AgentFinished { id, report } => self.on_agent_finished(id.0, report),
            AgentEvent::TaskStart { id, command, .. } => self.on_task_start(id, command),
            AgentEvent::TaskProgressed { id, progress } => self.on_task_progress(id, progress),
            AgentEvent::TaskEnd { id, state, .. } => self.on_task_end(id, state),
            AgentEvent::AgentEnd { stop_reason } => self.on_agent_end(*stop_reason),
        }
    }

    fn apply_stream(&mut self, event: &StreamEvent) {
        match event {
            StreamEvent::TextStart { .. } => {
                self.rows.push(Row::Assistant {
                    text: String::new(),
                });
            }
            StreamEvent::TextDelta { delta, .. } => {
                if let Some(Row::Assistant { text }) = self.last_assistant_mut() {
                    text.push_str(delta);
                }
            }
            StreamEvent::ThinkingStart { .. } => {
                self.rows.push(Row::Thinking {
                    text: String::new(),
                });
            }
            StreamEvent::ThinkingDelta { delta, .. } => {
                if let Some(Row::Thinking { text }) = self.last_thinking_mut() {
                    text.push_str(delta);
                }
            }
            StreamEvent::ToolCallStart { index, id, name } => {
                self.tool_call_names
                    .push((*index, id.clone(), name.clone()));
            }
            StreamEvent::ToolCallEnd { index, .. } => {
                let found = self
                    .tool_call_names
                    .iter()
                    .rev()
                    .find(|(i, _, _)| i == index)
                    .cloned();
                if let Some((_, id, name)) = found {
                    self.rows.push(Row::Tool {
                        id,
                        name,
                        kind: ToolKind::Other,
                        status: ToolRowStatus::Pending,
                        preview: String::new(),
                    });
                }
            }
            StreamEvent::MessageStart { .. }
            | StreamEvent::TextEnd { .. }
            | StreamEvent::ThinkingEnd { .. }
            | StreamEvent::ToolCallDelta { .. }
            | StreamEvent::Usage(_)
            | StreamEvent::Done { .. } => {}
        }
    }

    /// Find a subagent row by id.
    fn agent_row_mut(&mut self, id: u64) -> Option<&mut Row> {
        self.rows
            .iter_mut()
            .find(|row| matches!(row, Row::Agent { id: row_id, .. } if *row_id == id))
    }

    fn on_agent_spawned(&mut self, id: u64, name: &str, depth: u32) {
        // A definition name is untrusted text, because it comes from a file on disk.
        self.rows.push(Row::Agent {
            id,
            name: crate::sanitize_line(name),
            depth,
            turns: 0,
            cost: String::new(),
            finished: false,
            failed: false,
            outcome: "running".to_string(),
        });
    }

    fn on_agent_progressed(&mut self, id: u64, turns: u32, usage: &rho_core::Usage) {
        let cost = summarise_usage(usage);
        if let Some(Row::Agent {
            turns: row_turns,
            cost: row_cost,
            ..
        }) = self.agent_row_mut(id)
        {
            *row_turns = turns;
            *row_cost = cost;
        }
    }

    fn on_agent_finished(&mut self, id: u64, report: &rho_core::AgentReport) {
        let cost = summarise_usage(&report.usage);
        let (word, failed) = outcome_label(&report.outcome);
        let turns = report.turns;
        if let Some(Row::Agent {
            turns: row_turns,
            cost: row_cost,
            finished,
            failed: row_failed,
            outcome,
            ..
        }) = self.agent_row_mut(id)
        {
            *row_turns = turns;
            *row_cost = cost;
            *finished = true;
            *row_failed = failed;
            *outcome = word;
        }
    }

    /// Find a task row by id.
    fn task_row_mut(&mut self, id: &str) -> Option<&mut Row> {
        self.rows
            .iter_mut()
            .find(|row| matches!(row, Row::Task { id: row_id, .. } if row_id == id))
    }

    fn on_task_start(&mut self, id: &TaskId, command: &str) {
        // A command is untrusted text, because the model wrote it. Sanitise it before
        // it reaches the screen.
        self.rows.push(Row::Task {
            id: id.0.clone(),
            command: crate::sanitize_line(command),
            state: "running".to_string(),
            finished: false,
            failed: false,
            progress: String::new(),
        });
    }

    fn on_task_progress(&mut self, id: &TaskId, progress: &TaskProgress) {
        let summary = summarise_progress(progress);
        if let Some(Row::Task {
            progress: row_progress,
            ..
        }) = self.task_row_mut(&id.0)
        {
            *row_progress = summary;
        }
    }

    fn on_task_end(&mut self, id: &TaskId, state: &TaskState) {
        let label = state_label(state);
        let failed = !state.is_success();
        if let Some(Row::Task {
            state: row_state,
            finished,
            failed: row_failed,
            ..
        }) = self.task_row_mut(&id.0)
        {
            *row_state = label;
            *finished = true;
            *row_failed = failed;
        }
    }

    fn on_tool_start(&mut self, id: &str, name: &str, kind: ToolKind) {
        if let Some(row) = self.tool_row_mut(id) {
            if let Row::Tool {
                kind: row_kind,
                status,
                ..
            } = row
            {
                *row_kind = kind;
                *status = ToolRowStatus::Running;
            }
        } else {
            self.rows.push(Row::Tool {
                id: id.to_string(),
                name: name.to_string(),
                kind,
                status: ToolRowStatus::Running,
                preview: String::new(),
            });
        }
    }

    fn on_tool_update(&mut self, id: &str, output: &str) {
        if let Some(Row::Tool { preview, .. }) = self.tool_row_mut(id) {
            *preview = output.to_string();
        }
    }

    fn on_tool_end(&mut self, id: &str, is_error: bool) {
        if let Some(Row::Tool { status, .. }) = self.tool_row_mut(id) {
            *status = if is_error {
                ToolRowStatus::Failed
            } else {
                ToolRowStatus::Ok
            };
        }
    }

    fn on_agent_end(&mut self, stop_reason: AgentStopReason) {
        self.activity = ActivityState::Idle;
        self.canceling = false;
        self.last_stop = Some(stop_reason);
        self.status = format!("done: {}", stop_reason_label(stop_reason));
    }

    fn last_assistant_mut(&mut self) -> Option<&mut Row> {
        self.rows
            .iter_mut()
            .rev()
            .find(|row| matches!(row, Row::Assistant { .. }))
    }

    fn last_thinking_mut(&mut self) -> Option<&mut Row> {
        self.rows
            .iter_mut()
            .rev()
            .find(|row| matches!(row, Row::Thinking { .. }))
    }

    fn tool_row_mut(&mut self, id: &str) -> Option<&mut Row> {
        self.rows
            .iter_mut()
            .find(|row| matches!(row, Row::Tool { id: row_id, .. } if row_id == id))
    }

    /// Append the submitted user input as a `User` row and clear the input.
    /// Return the submitted text, so the caller can start a run with it.
    pub fn submit_input(&mut self) -> String {
        let text = std::mem::take(&mut self.input);
        self.rows.push(Row::User { text: text.clone() });
        text
    }

    /// Fold one key press into the state. Pure. No IO. Return the action the event
    /// loop must run. Input never waits on model work, so a printable key always
    /// appends, even while a turn streams. See `SPEC-tui` section 5.
    ///
    /// A panel owns the keyboard while it is open, because a list the user is reading
    /// must answer the arrow keys rather than type into the draft.
    pub fn handle_key(&mut self, key: KeyEvent) -> KeyAction {
        if is_ctrl_c(&key) {
            return self.handle_ctrl_c();
        }
        // Any key other than Ctrl-C clears the exit arm.
        self.exit_armed = false;

        // A chord is never text. The old handler pushed the letter of every chord into
        // the draft, so Ctrl-D typed a `d` instead of leaving.
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return self.handle_chord(key.code);
        }

        match &self.panel {
            Panel::SlashList(list) => return self.handle_slash_key(key.code, list.clone()),
            Panel::Help => return self.handle_help_key(key.code),
            // An approval prompt answers its own keys, which stage U6 wires.
            Panel::Approval(_) | Panel::None => {}
        }

        match key.code {
            KeyCode::Enter => {
                if self.input.is_empty() {
                    KeyAction::None
                } else {
                    KeyAction::Submit(self.submit_input())
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
                KeyAction::None
            }
            // The composer promises `/ for commands` and `? for help`, so both keys open
            // their panel on an empty draft. Inside a draft they stay ordinary text,
            // because `why?` is a question and not a request for help.
            KeyCode::Char('/') if self.input.is_empty() => {
                self.input.push('/');
                self.panel = Panel::SlashList(SlashList {
                    query: "/".to_string(),
                    selected: 0,
                });
                KeyAction::None
            }
            KeyCode::Char('?') if self.input.is_empty() => {
                self.panel = Panel::Help;
                KeyAction::None
            }
            KeyCode::Char(ch) => {
                self.input.push(ch);
                KeyAction::None
            }
            _ => KeyAction::None,
        }
    }

    /// Answer a chord. A chord never types its letter.
    fn handle_chord(&mut self, code: KeyCode) -> KeyAction {
        match code {
            // Ctrl-D on an empty draft leaves rho, the way it ends a shell. With a draft
            // it does nothing, because a stray chord must not throw away written work.
            KeyCode::Char('d') if self.input.is_empty() && self.panel == Panel::None => {
                KeyAction::Exit
            }
            _ => KeyAction::None,
        }
    }

    /// Answer a key while the slash list is open.
    fn handle_slash_key(&mut self, code: KeyCode, mut list: SlashList) -> KeyAction {
        match code {
            KeyCode::Esc => {
                // Close the panel, and keep every character. A path starts with a slash,
                // so a user reaches this state by accident, and a draft is theirs.
                self.panel = Panel::None;
            }
            KeyCode::Up => {
                list.selected = list.selected.saturating_sub(1);
                self.panel = Panel::SlashList(list);
            }
            KeyCode::Down => {
                let count = crate::filter_slash_commands(&list.query).len();
                list.selected = (list.selected + 1).min(count.saturating_sub(1));
                self.panel = Panel::SlashList(list);
            }
            KeyCode::Backspace => {
                self.input.pop();
                if self.input.is_empty() {
                    self.panel = Panel::None;
                } else {
                    list.query = self.input.clone();
                    list.selected = 0;
                    self.panel = Panel::SlashList(list);
                }
            }
            KeyCode::Char(ch) => {
                self.input.push(ch);
                list.query = self.input.clone();
                list.selected = 0;
                self.panel = Panel::SlashList(list);
            }
            KeyCode::Enter => return self.run_selected_command(&list),
            // Tab completes the draft to the selected command, the way a shell completes
            // a path. It runs nothing, so the user still sees what will run.
            KeyCode::Tab => {
                if let Some(command) = crate::filter_slash_commands(&list.query).get(list.selected)
                {
                    self.input = command.name.to_string();
                    list.query = self.input.clone();
                    list.selected = 0;
                    self.panel = Panel::SlashList(list);
                }
            }
            _ => {}
        }
        KeyAction::None
    }

    /// Answer a key while the help panel is open. Esc closes it, and so does `?`.
    fn handle_help_key(&mut self, code: KeyCode) -> KeyAction {
        match code {
            KeyCode::Esc | KeyCode::Char('?') => {
                self.panel = Panel::None;
                KeyAction::None
            }
            _ => KeyAction::None,
        }
    }

    /// Run the selected row of the slash list. A command never reaches the model, and
    /// a command that does nothing yet says so, because silence reads as a defect.
    fn run_selected_command(&mut self, list: &SlashList) -> KeyAction {
        let chosen = crate::filter_slash_commands(&list.query)
            .get(list.selected)
            .map(|command| command.name.to_string());

        let Some(name) = chosen else {
            // Nothing matched, so report the text the user typed, and keep it. The draft
            // may be a path rather than a command, and a report must not eat it.
            let message = match crate::run_slash_command(&self.input.clone()) {
                crate::SlashOutcome::Unknown(message) => message,
                crate::SlashOutcome::Run(name) => format!("unknown command: {name}"),
            };
            self.panel = Panel::None;
            self.push_error(message);
            return KeyAction::None;
        };

        // A matched command consumes the draft, because the draft was the command.
        self.input.clear();
        self.panel = Panel::None;

        match name.as_str() {
            "/quit" => KeyAction::Exit,
            "/help" => {
                self.panel = Panel::Help;
                KeyAction::None
            }
            other => {
                self.push_error(format!(
                    "{other} is not built yet. See F-slash-commands in docs/features.md."
                ));
                KeyAction::None
            }
        }
    }

    /// End the run when the event stream stops with no `AgentEnd`.
    ///
    /// `rho_core::Driver::run` returns on `TurnOutcome::Failed` and on `Closed` without
    /// emitting a stop event. The frontend must not stay `Running` after that, because a
    /// stuck `Running` sends the next Ctrl-C to a cancel on a token that is gone, and the
    /// user then cannot quit with Ctrl-C at all. Pass `failed` for a stream error.
    pub fn end_run(&mut self, failed: bool) {
        let was_canceling = self.canceling;
        self.activity = ActivityState::Idle;
        self.canceling = false;
        if failed {
            self.last_error = true;
            self.status = "the run ended with an error".to_string();
        } else if was_canceling {
            self.last_stop = Some(AgentStopReason::Canceled);
            self.status = "canceled".to_string();
        }
    }

    /// Select and run a slash row by index, for a mouse click.
    pub fn click_slash_row(&mut self, index: usize) -> KeyAction {
        // A click is a key press to the user, and the footer promises that any key keeps
        // the session. So a click disarms the exit gate too.
        self.exit_armed = false;
        let Panel::SlashList(list) = &self.panel else {
            return KeyAction::None;
        };
        let mut list = list.clone();
        list.selected = index;
        self.run_selected_command(&list)
    }

    /// Push a one-line error row. The transcript is the only channel the user reads,
    /// so a message that matters goes here and not into a field nobody draws.
    pub fn push_error(&mut self, message: impl Into<String>) {
        self.rows.push(Row::Error {
            message: crate::sanitize_line(&message.into()),
            detail: Vec::new(),
        });
    }

    fn handle_ctrl_c(&mut self) -> KeyAction {
        match self.activity {
            ActivityState::Running => {
                // A first Ctrl-C during a run cancels the turn. It does not exit.
                self.canceling = true;
                self.status = "canceling…".to_string();
                KeyAction::Cancel
            }
            ActivityState::Idle => {
                if self.exit_armed {
                    KeyAction::Exit
                } else {
                    self.exit_armed = true;
                    self.status = "press Ctrl-C again to exit".to_string();
                    KeyAction::None
                }
            }
        }
    }
}

/// True when a key event is Ctrl-C.
fn is_ctrl_c(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
}

/// A short label for a stop reason, shown on the status line.
fn stop_reason_label(reason: AgentStopReason) -> &'static str {
    match reason {
        AgentStopReason::EndTurn => "end turn",
        AgentStopReason::MaxTokens => "max tokens",
        AgentStopReason::MaxTurnRequests => "max turns",
        AgentStopReason::Refusal => "refusal",
        AgentStopReason::Canceled => "canceled",
    }
}

/// A one-line summary of a progress report, for the status column.
///
/// A progress `message` is untrusted, because a child prints whatever it likes. So it
/// is sanitised here, exactly like tool output. See `SPEC-background-tasks` section 9.
fn summarise_progress(progress: &TaskProgress) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(percent) = progress.percent {
        parts.push(format!("{percent}%"));
    }
    if let (Some(done), Some(total)) = (progress.done, progress.total) {
        parts.push(format!("{done}/{total}"));
    }
    if let Some(message) = &progress.message {
        let clean = crate::sanitize_line(message);
        if !clean.is_empty() {
            parts.push(clean);
        }
    }
    parts.join(" ")
}

/// A short word for a task state, for the status column.
fn state_label(state: &TaskState) -> String {
    match state {
        TaskState::Running => "running".to_string(),
        TaskState::Exited { code } if *code == 0 => "done".to_string(),
        TaskState::Exited { code } => format!("failed ({code})"),
        TaskState::Signaled { signal } => match signal {
            Some(name) => format!("killed ({name})"),
            None => "killed".to_string(),
        },
        TaskState::Canceled => "canceled".to_string(),
        TaskState::TimedOut => "timed out".to_string(),
    }
}

/// A short cost summary for a status row.
///
/// Tokens rather than money, because two of the three providers report no charge. See
/// decision D-measured-cost-and-cache.
fn summarise_usage(usage: &rho_core::Usage) -> String {
    let mut text = format!(
        "{} in, {} out",
        thousands(usage.input_tokens),
        thousands(usage.output_tokens)
    );
    if usage.cache_read_tokens > 0 {
        text.push_str(&format!(", {} cached", thousands(usage.cache_read_tokens)));
    }
    if let Some(cost) = usage.cost_usd {
        text.push_str(&format!(", ${cost:.4}"));
    }
    text
}

/// Render a count compactly, so a wide number does not push a row off screen.
fn thousands(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

/// The outcome word for a finished child, and whether it counts as a failure.
fn outcome_label(outcome: &rho_core::AgentOutcome) -> (String, bool) {
    match outcome {
        rho_core::AgentOutcome::Done => ("done".to_string(), false),
        rho_core::AgentOutcome::OutOfTurns => ("out of turns".to_string(), true),
        rho_core::AgentOutcome::Canceled => ("canceled".to_string(), true),
        rho_core::AgentOutcome::Failed { reason } => {
            (format!("failed: {}", crate::sanitize_line(reason)), true)
        }
    }
}
