//! The TUI state and its pure reducer.
//!
//! The state is a value. A pure reducer folds one `AgentEvent` into the state. A
//! pure key handler folds one key press into the state. Neither does IO, so a
//! test drives them with no terminal at all. See `SPEC-05` sections 1 to 3.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rho_core::{
    AgentEvent, AgentStopReason, StreamEvent, TaskId, TaskProgress, TaskState, ToolKind,
};

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
    /// Fold one agent event into the state. Pure. No IO. See `SPEC-05` section 3.
    pub fn apply(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::TurnStart => self.activity = ActivityState::Running,
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
    /// appends, even while a turn streams. See `SPEC-05` section 5.
    pub fn handle_key(&mut self, key: KeyEvent) -> KeyAction {
        if is_ctrl_c(&key) {
            return self.handle_ctrl_c();
        }
        // Any key other than Ctrl-C clears the exit arm.
        self.exit_armed = false;
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
            KeyCode::Char(ch) => {
                self.input.push(ch);
                KeyAction::None
            }
            _ => KeyAction::None,
        }
    }

    fn handle_ctrl_c(&mut self) -> KeyAction {
        match self.activity {
            ActivityState::Running => {
                // A first Ctrl-C during a run cancels the turn. It does not exit.
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
/// is sanitised here, exactly like tool output. See `SPEC-07` section 9.
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
/// decision D-032.
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
