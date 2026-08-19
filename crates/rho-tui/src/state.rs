//! The TUI state and its pure reducer.
//!
//! The state is a value. A pure reducer folds one `AgentEvent` into the state. A
//! pure key handler folds one key press into the state. Neither does IO, so a
//! test drives them with no terminal at all. See `SPEC-tui` sections 1 to 3.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rho_core::{
    AgentEvent, AgentStopReason, ReasoningDisplay, StreamEvent, TaskId, TaskProgress, TaskState,
    ThinkingPiece, ThinkingSplitter, ToolKind,
};

use crate::concise::RowFold;
use crate::paste::Composer;

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
    /// The reverse history search, opened by `ctrl-r`.
    HistorySearch(HistorySearch),
}

/// The approval prompt content. The command is verbatim, so the user sees exactly
/// what would run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    /// The request line, for example `bash asks to run`.
    pub title: String,
    /// The verbatim command awaiting approval.
    pub command: String,
    /// The directory the command runs in.
    ///
    /// A command means nothing without the tree it acts on. `rm -rf target` is routine in
    /// a build directory and ruinous in a home directory, so the panel states the root
    /// beside the command, and it never drops the row to save space. See
    /// `D-ledger-wins-the-band`.
    pub root: String,
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

/// The reverse-search panel state: the typed query and the selected match.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct HistorySearch {
    /// The typed query. An empty query matches every entry.
    pub query: String,
    /// The selected row index into the filtered matches.
    pub selected: usize,
}

/// The history entries that hold `query`, newest first, as indexes into the history.
///
/// The match is a case-insensitive substring. An empty query matches every entry. The
/// result lists the newest match first, so the panel opens on the most recent draft.
pub fn filter_history(history: &[String], query: &str) -> Vec<usize> {
    let needle = query.to_lowercase();
    history
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, entry)| entry.to_lowercase().contains(&needle))
        .map(|(index, _)| index)
        .collect()
}

/// The whole TUI state. A pure function of the events applied so far, plus the
/// local input buffer and one flag for the Ctrl-C exit gate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TuiState {
    pub rows: Vec<Row>,
    /// The draft, with its paste chips and its cursor.
    pub draft: Composer,
    /// The drafts submitted in this session, oldest first. It lives for the session only.
    pub history: Vec<String>,
    /// The wrap width the composer last drew at. The loop sets it, so a row motion key
    /// wraps the same way the screen does. Zero wraps on newlines only.
    pub composer_width: usize,
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
    /// The first binding the help window shows, counted from zero.
    ///
    /// The table is longer than the band, so the window scrolls. The arrows move it, and
    /// they clamp it to `help_visible_rows`. See `D-ledger-wins-the-band`.
    pub help_offset: usize,
    /// The count of help rows the band can draw, set by the app loop each frame.
    ///
    /// The reducer needs it to clamp a scroll key. Without it the offset climbed past the
    /// last row, and a press back moved nothing until every press was paid back.
    pub help_visible_rows: usize,
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
    /// The count of rows the terminal already owns, oldest first.
    ///
    /// A row below this index is in the terminal's scrollback. rho can never repaint it,
    /// so the reducer must never write it. See `D-a-frozen-row-never-repaints`.
    pub frozen_rows: usize,
    /// The count of events dropped because their row was already frozen.
    ///
    /// A provider that repeats itself is visible here. A silent drop would hide a defect.
    /// See `D-a-late-event-for-a-frozen-row-is-dropped`.
    pub late_events: usize,
    /// The start instant of each row, parallel to `rows`. The reducer writes it when it
    /// pushes the row, and reads it when the row finishes. This is private, because a
    /// duration is the public fact.
    row_started: Vec<Option<i64>>,
    /// The start instant of the running turn, for the footer clock.
    turn_started: Option<i64>,
    /// True after one `esc` with no panel. A second `esc` then clears the draft.
    esc_armed: bool,
    /// True after `ctrl-x`. The next `ctrl-e` then opens the editor.
    awaiting_editor: bool,
    /// The recall position in the history, or `None` while the live draft is shown.
    history_pos: Option<usize>,
    /// The live draft, saved when a recall starts, so a forward recall restores it.
    history_stash: Option<String>,
    /// How the frontend draws reasoning. It changes only what rho draws, never the
    /// stream and never what reaches a provider. See `SPEC-reasoning-across-providers`.
    pub reasoning_display: ReasoningDisplay,
    /// The splitter that lifts a leading `<thinking>` tag out of the answer text. It is
    /// reset at each text-block start, because the leading rule is per block. It is
    /// private, so it is not part of the public state contract.
    text_split: ThinkingSplitter,
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
    /// Edit this text in the editor, then replace the draft with the result.
    EditDraft(String),
}

impl TuiState {
    /// Fold one agent event into the state, at `now_millis` on the caller's clock.
    ///
    /// Pure. No IO. The clock arrives as data, so the reducer reads no clock and a test
    /// drives time. The same call writes the row metadata, so a row and its duration can
    /// never disagree. See `D-the-reducer-owns-the-row-metadata`.
    pub fn apply(&mut self, event: &AgentEvent, now_millis: i64) {
        match event {
            AgentEvent::TurnStart => {
                self.activity = ActivityState::Running;
                self.canceling = false;
                self.turn_started = Some(now_millis);
            }
            AgentEvent::Stream(stream) => self.apply_stream(stream, now_millis),
            AgentEvent::ToolStart { id, name, kind } => {
                self.on_tool_start(id, name, *kind, now_millis)
            }
            AgentEvent::ToolUpdate { id, output } => self.on_tool_update(id, output),
            AgentEvent::ToolEnd { id, output } => self.on_tool_end(id, output.is_error, now_millis),
            AgentEvent::TurnEnd { .. } => {}
            AgentEvent::AgentSpawned { id, agent, depth } => {
                self.on_agent_spawned(id.0, agent, *depth, now_millis)
            }
            AgentEvent::AgentProgressed { id, turns, usage } => {
                self.on_agent_progressed(id.0, *turns, usage)
            }
            AgentEvent::AgentFinished { id, report } => self.on_agent_finished(id.0, report),
            AgentEvent::TaskStart { id, command, .. } => {
                self.on_task_start(id, command, now_millis)
            }
            AgentEvent::TaskProgressed { id, progress } => self.on_task_progress(id, progress),
            AgentEvent::TaskEnd { id, state, .. } => self.on_task_end(id, state),
            AgentEvent::AgentEnd { stop_reason } => self.on_agent_end(*stop_reason, now_millis),
        }
    }

    /// Push a row, and keep every parallel array the same length.
    ///
    /// Every push goes through here. A row whose metadata array is short would read another
    /// row's duration, or none at all.
    fn push_row(&mut self, row: Row, started: Option<i64>) {
        self.rows.push(row);
        self.row_durations.push(None);
        self.row_started.push(started);
    }

    /// State the session context the banner reports.
    ///
    /// The banner names where rho runs, so a user with two sessions can tell them apart.
    /// The renderer read these fields from the first frame, and nothing wrote them, so the
    /// banner drew three separators around nothing.
    pub fn set_context(
        &mut self,
        cwd: impl Into<String>,
        branch: impl Into<String>,
        provider: impl Into<String>,
    ) {
        self.cwd = cwd.into();
        self.branch = branch.into();
        self.provider = provider.into();
    }

    /// The rows the band still owns, oldest first.
    pub fn live_rows(&self) -> &[Row] {
        let start = self.frozen_rows.min(self.rows.len());
        &self.rows[start..]
    }

    /// Record that `rows` more rows left the band. A count past the end saturates.
    pub fn mark_frozen(&mut self, rows: usize) {
        self.frozen_rows = self.frozen_rows.saturating_add(rows).min(self.rows.len());
    }

    fn apply_stream(&mut self, event: &StreamEvent, now_millis: i64) {
        match event {
            StreamEvent::TextStart { .. } => {
                // A new text block, so the tag splitter starts fresh. A leading
                // `<thinking>` tag is stripped only from the block's start.
                self.text_split = ThinkingSplitter::new();
                self.push_row(
                    Row::Assistant {
                        text: String::new(),
                    },
                    Some(now_millis),
                );
            }
            StreamEvent::TextDelta { delta, .. } => {
                let pieces = self.text_split.push(delta);
                self.route_thinking_pieces(pieces, now_millis);
            }
            StreamEvent::ThinkingStart { .. } => {
                self.push_row(
                    Row::Thinking {
                        text: String::new(),
                    },
                    Some(now_millis),
                );
            }
            StreamEvent::ThinkingDelta { delta, .. } => {
                if let Some(Row::Thinking { text }) = self.last_thinking_mut() {
                    text.push_str(delta);
                }
            }
            // A thinking block ends, so its span is settled. The slot is written here,
            // because a row must carry its duration before it can freeze.
            StreamEvent::ThinkingEnd { .. } => {
                if let Some(index) = self.last_live_index(|row| matches!(row, Row::Thinking { .. }))
                {
                    self.settle_duration(index, now_millis);
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
                    self.push_row(
                        Row::Tool {
                            id,
                            name,
                            kind: ToolKind::Other,
                            status: ToolRowStatus::Pending,
                            preview: String::new(),
                        },
                        Some(now_millis),
                    );
                }
            }
            StreamEvent::MessageStart { .. }
            | StreamEvent::ToolCallDelta { .. }
            | StreamEvent::Usage(_)
            | StreamEvent::Done { .. } => {}
            StreamEvent::TextEnd { .. } => {
                // Flush a held-back lead or an unclosed tag. An unclosed tag becomes
                // reasoning, so a truncated stream loses no text.
                let pieces = self.text_split.finish();
                self.route_thinking_pieces(pieces, now_millis);
                // A reasoning-only block left its thinking row open. Settle its span here,
                // so the summary reads `thought for` and not a bare `thinking`.
                if let Some(index) = self.open_split_thinking_index() {
                    self.settle_duration(index, now_millis);
                }
            }
        }
    }

    /// Route the tag splitter's output to the right rows.
    ///
    /// Reasoning that leads the block goes to a thinking row, drawn dimmed. The answer
    /// that follows goes to an assistant row. See `SPEC-reasoning-across-providers`.
    fn route_thinking_pieces(&mut self, pieces: Vec<ThinkingPiece>, now_millis: i64) {
        for piece in pieces {
            match piece {
                ThinkingPiece::Reasoning(text) => self.append_reasoning(&text, now_millis),
                ThinkingPiece::Text(text) => self.append_answer(&text, now_millis),
            }
        }
    }

    /// The index of the newest live row, when it is within the live range.
    fn newest_live_index(&self) -> Option<usize> {
        let start = self.frozen_rows.min(self.rows.len());
        let index = self.rows.len().checked_sub(1)?;
        (index >= start).then_some(index)
    }

    /// Append reasoning to the open thinking row, or open one.
    ///
    /// Reasoning always leads a block. So the newest row is either the empty assistant
    /// row from `TextStart`, which becomes the thinking row, or the thinking row itself.
    fn append_reasoning(&mut self, delta: &str, now_millis: i64) {
        match self.newest_live_index() {
            Some(index) if matches!(self.rows[index], Row::Thinking { .. }) => {
                if let Row::Thinking { text } = &mut self.rows[index] {
                    text.push_str(delta);
                }
            }
            Some(index) if matches!(&self.rows[index], Row::Assistant { text } if text.is_empty()) =>
            {
                // Reuse the empty assistant row and its start instant, so the span runs
                // from the block start and no empty answer row is left behind.
                self.rows[index] = Row::Thinking {
                    text: delta.to_string(),
                };
            }
            _ => self.push_row(
                Row::Thinking {
                    text: delta.to_string(),
                },
                Some(now_millis),
            ),
        }
    }

    /// Append answer text to the open assistant row, or open one after the reasoning.
    fn append_answer(&mut self, delta: &str, now_millis: i64) {
        match self.newest_live_index() {
            Some(index) if matches!(self.rows[index], Row::Assistant { .. }) => {
                if let Row::Assistant { text } = &mut self.rows[index] {
                    text.push_str(delta);
                }
            }
            other => {
                // The answer begins after reasoning. Settle the reasoning span, then open
                // the answer row, so `Live` mode collapses the reasoning at this point.
                if let Some(index) = other
                    && matches!(self.rows[index], Row::Thinking { .. })
                {
                    self.settle_duration(index, now_millis);
                }
                self.push_row(
                    Row::Assistant {
                        text: delta.to_string(),
                    },
                    Some(now_millis),
                );
            }
        }
    }

    /// The newest live row when it is an unsettled thinking row, else `None`.
    fn open_split_thinking_index(&self) -> Option<usize> {
        let index = self.newest_live_index()?;
        let is_thinking = matches!(self.rows[index], Row::Thinking { .. });
        let unsettled = self.row_durations.get(index).copied().flatten().is_none();
        (is_thinking && unsettled).then_some(index)
    }

    /// Write the span of the row at `index`, from its start instant to `now_millis`.
    ///
    /// The slot is always written, even with no start instant, because a row that never
    /// settles its metadata could never freeze. A span below zero means the clock stepped
    /// back, and the duration ladder renders an empty slot for it.
    fn settle_duration(&mut self, index: usize, now_millis: i64) {
        let span = self
            .row_started
            .get(index)
            .copied()
            .flatten()
            .map(|start| now_millis - start);
        if let Some(slot) = self.row_durations.get_mut(index) {
            *slot = span;
        }
    }

    /// The index of the newest live row that matches, or `None`.
    fn last_live_index(&self, matches: impl Fn(&Row) -> bool) -> Option<usize> {
        let start = self.frozen_rows.min(self.rows.len());
        self.rows
            .iter()
            .enumerate()
            .skip(start)
            .rev()
            .find(|(_, row)| matches(row))
            .map(|(index, _)| index)
    }

    /// Find a subagent row by id, among the live rows only.
    ///
    /// A frozen row is unreachable here by design. See
    /// `D-a-late-event-for-a-frozen-row-is-dropped`.
    fn agent_row_mut(&mut self, id: u64) -> Option<&mut Row> {
        let start = self.frozen_rows.min(self.rows.len());
        self.rows[start..]
            .iter_mut()
            .find(|row| matches!(row, Row::Agent { id: row_id, .. } if *row_id == id))
    }

    /// True when a frozen row already holds this subagent id.
    fn frozen_has_agent(&self, id: u64) -> bool {
        let end = self.frozen_rows.min(self.rows.len());
        self.rows[..end]
            .iter()
            .any(|row| matches!(row, Row::Agent { id: row_id, .. } if *row_id == id))
    }

    fn on_agent_spawned(&mut self, id: u64, name: &str, depth: u32, now_millis: i64) {
        // A definition name is untrusted text, because it comes from a file on disk.
        self.push_row(
            Row::Agent {
                id,
                name: crate::sanitize_line(name),
                depth,
                turns: 0,
                cost: String::new(),
                finished: false,
                failed: false,
                outcome: "running".to_string(),
            },
            Some(now_millis),
        );
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
        } else if self.frozen_has_agent(id) {
            self.late_events += 1;
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
        } else if self.frozen_has_agent(id) {
            self.late_events += 1;
        }
    }

    /// Find a task row by id, among the live rows only.
    fn task_row_mut(&mut self, id: &str) -> Option<&mut Row> {
        let start = self.frozen_rows.min(self.rows.len());
        self.rows[start..]
            .iter_mut()
            .find(|row| matches!(row, Row::Task { id: row_id, .. } if row_id == id))
    }

    /// True when a frozen row already holds this task id.
    fn frozen_has_task(&self, id: &str) -> bool {
        let end = self.frozen_rows.min(self.rows.len());
        self.rows[..end]
            .iter()
            .any(|row| matches!(row, Row::Task { id: row_id, .. } if row_id == id))
    }

    fn on_task_start(&mut self, id: &TaskId, command: &str, now_millis: i64) {
        // A command is untrusted text, because the model wrote it. Sanitise it before
        // it reaches the screen.
        self.push_row(
            Row::Task {
                id: id.0.clone(),
                command: crate::sanitize_line(command),
                state: "running".to_string(),
                finished: false,
                failed: false,
                progress: String::new(),
            },
            Some(now_millis),
        );
    }

    fn on_task_progress(&mut self, id: &TaskId, progress: &TaskProgress) {
        let summary = summarise_progress(progress);
        if let Some(Row::Task {
            progress: row_progress,
            ..
        }) = self.task_row_mut(&id.0)
        {
            *row_progress = summary;
        } else if self.frozen_has_task(&id.0) {
            self.late_events += 1;
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
        } else if self.frozen_has_task(&id.0) {
            self.late_events += 1;
        }
    }

    fn on_tool_start(&mut self, id: &str, name: &str, kind: ToolKind, now_millis: i64) {
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
            return;
        }
        // A start for an id the terminal already owns is late. It must not push a second
        // row, because the transcript would then report the call twice.
        if self.frozen_has_tool(id) {
            self.late_events += 1;
            return;
        }
        self.push_row(
            Row::Tool {
                id: id.to_string(),
                name: name.to_string(),
                kind,
                status: ToolRowStatus::Running,
                preview: String::new(),
            },
            Some(now_millis),
        );
    }

    fn on_tool_update(&mut self, id: &str, output: &str) {
        if let Some(Row::Tool { preview, .. }) = self.tool_row_mut(id) {
            *preview = output.to_string();
        } else if self.frozen_has_tool(id) {
            self.late_events += 1;
        }
    }

    fn on_tool_end(&mut self, id: &str, is_error: bool, now_millis: i64) {
        let Some(index) = self.live_tool_index(id) else {
            if self.frozen_has_tool(id) {
                self.late_events += 1;
            }
            return;
        };
        if let Some(Row::Tool { status, .. }) = self.rows.get_mut(index) {
            *status = if is_error {
                ToolRowStatus::Failed
            } else {
                ToolRowStatus::Ok
            };
        }
        // The status flip makes the row final, so its span settles in the same call.
        self.settle_duration(index, now_millis);
    }

    fn on_agent_end(&mut self, stop_reason: AgentStopReason, now_millis: i64) {
        self.activity = ActivityState::Idle;
        self.canceling = false;
        self.last_stop = Some(stop_reason);
        self.status = format!("done: {}", stop_reason_label(stop_reason));
        if let Some(start) = self.turn_started {
            self.turn_millis = Some(now_millis - start);
        }
    }

    fn last_thinking_mut(&mut self) -> Option<&mut Row> {
        let start = self.frozen_rows.min(self.rows.len());
        self.rows[start..]
            .iter_mut()
            .rev()
            .find(|row| matches!(row, Row::Thinking { .. }))
    }

    /// Find a tool row by id, among the live rows only.
    fn tool_row_mut(&mut self, id: &str) -> Option<&mut Row> {
        let start = self.frozen_rows.min(self.rows.len());
        self.rows[start..]
            .iter_mut()
            .find(|row| matches!(row, Row::Tool { id: row_id, .. } if row_id == id))
    }

    /// The absolute index of a live tool row with this id.
    fn live_tool_index(&self, id: &str) -> Option<usize> {
        let start = self.frozen_rows.min(self.rows.len());
        self.rows
            .iter()
            .enumerate()
            .skip(start)
            .find(|(_, row)| matches!(row, Row::Tool { id: row_id, .. } if row_id == id))
            .map(|(index, _)| index)
    }

    /// True when a frozen row already holds this tool id.
    fn frozen_has_tool(&self, id: &str) -> bool {
        let end = self.frozen_rows.min(self.rows.len());
        self.rows[..end]
            .iter()
            .any(|row| matches!(row, Row::Tool { id: row_id, .. } if row_id == id))
    }

    /// Append the submitted draft as a `User` row and empty the draft.
    ///
    /// It takes the model text with `Composer::take`, so a held paste reaches the model
    /// in full. It pushes the user row through `push_row`, so the parallel metadata arrays
    /// stay aligned. It records the text in the history, and it drops a repeat.
    pub fn submit_input(&mut self) -> String {
        let text = self.draft.take();
        self.push_row(Row::User { text: text.clone() }, None);
        self.push_history(&text);
        self.reset_history_nav();
        text
    }

    /// The draft as the user sees it, with a chip label for each held paste.
    pub fn draft_text(&self) -> String {
        self.draft.display_string()
    }

    /// True when the draft holds nothing.
    pub fn draft_is_empty(&self) -> bool {
        self.draft.is_empty()
    }

    /// Record a submitted draft in the history. It drops an empty draft and a repeat.
    fn push_history(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.history.last().map(String::as_str) == Some(text) {
            return;
        }
        self.history.push(text.to_string());
    }

    /// Reset the history recall, so the next `up` starts from the newest entry.
    fn reset_history_nav(&mut self) {
        self.history_pos = None;
        self.history_stash = None;
    }

    /// Recall the previous draft. Return `false` at the oldest entry.
    ///
    /// The first recall saves the live draft, so a forward recall can restore it.
    pub fn recall_previous(&mut self) -> bool {
        if self.history.is_empty() {
            return false;
        }
        let target = match self.history_pos {
            None => {
                self.history_stash = Some(self.draft.model_text());
                self.history.len() - 1
            }
            Some(0) => return false,
            Some(pos) => pos - 1,
        };
        self.history_pos = Some(target);
        self.draft.set_text(&self.history[target]);
        true
    }

    /// Recall the next draft, and then the live draft. Return `false` past the end.
    pub fn recall_next(&mut self) -> bool {
        let Some(pos) = self.history_pos else {
            return false;
        };
        if pos + 1 < self.history.len() {
            self.history_pos = Some(pos + 1);
            self.draft.set_text(&self.history[pos + 1]);
        } else {
            let live = self.history_stash.take().unwrap_or_default();
            self.draft.set_text(&live);
            self.history_pos = None;
        }
        true
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

        // A `ctrl-x` armed the editor. The next key resolves it. A `ctrl-e` opens the
        // editor. Any other key clears the arm and runs as usual.
        if self.awaiting_editor {
            self.awaiting_editor = false;
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('e') {
                return KeyAction::EditDraft(self.draft.model_text());
            }
        }

        // A single `esc` arms the draft clear. Any other key disarms it.
        if key.code != KeyCode::Esc {
            self.esc_armed = false;
        }

        // A chord is never text. The old handler pushed the letter of every chord into
        // the draft, so Ctrl-D typed a `d` instead of leaving.
        //
        // Shift+Enter is a chord too, and it is named one key at a time. SHIFT may never
        // join the mask below, because `shift+a` is how a capital letter arrives, and a
        // chord drops its text. A shifted Enter carries no text, so it is safe to route.
        let shift_enter = key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::SHIFT);
        if shift_enter
            || key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return self.handle_chord(key);
        }

        match &self.panel {
            Panel::SlashList(list) => return self.handle_slash_key(key.code, list.clone()),
            Panel::Help => return self.handle_help_key(key.code),
            Panel::HistorySearch(search) => {
                return self.handle_search_key(key.code, search.clone());
            }
            // An approval prompt answers its own keys, which stage U6 wires.
            Panel::Approval(_) | Panel::None => {}
        }

        match key.code {
            KeyCode::Enter => {
                if self.draft.is_empty() {
                    KeyAction::None
                } else {
                    KeyAction::Submit(self.submit_input())
                }
            }
            KeyCode::Backspace => {
                self.draft.backspace();
                KeyAction::None
            }
            KeyCode::Delete => {
                self.draft.delete_forward();
                KeyAction::None
            }
            KeyCode::Left => {
                self.draft.move_left();
                KeyAction::None
            }
            KeyCode::Right => {
                self.draft.move_right();
                KeyAction::None
            }
            KeyCode::Home => {
                self.draft.move_line_start();
                KeyAction::None
            }
            KeyCode::End => {
                self.draft.move_line_end();
                KeyAction::None
            }
            // Up moves the cursor one display row. At the top row it recalls the previous
            // draft, so a tall draft keeps the arrow and a one-row draft reaches history.
            KeyCode::Up => {
                if !self.draft.move_row_up(self.composer_width) {
                    self.recall_previous();
                }
                KeyAction::None
            }
            KeyCode::Down => {
                if !self.draft.move_row_down(self.composer_width) {
                    self.recall_next();
                }
                KeyAction::None
            }
            // A second `esc` with no panel clears the draft into the history.
            KeyCode::Esc => {
                if self.esc_armed {
                    self.esc_armed = false;
                    let text = self.draft.take();
                    self.push_history(&text);
                    self.reset_history_nav();
                } else {
                    self.esc_armed = true;
                }
                KeyAction::None
            }
            // The composer promises `/ for commands` and `? for help`, so both keys open
            // their panel on an empty draft. Inside a draft they stay ordinary text,
            // because `why?` is a question and not a request for help.
            KeyCode::Char('/') if self.draft.is_empty() => {
                self.draft.insert("/");
                self.panel = Panel::SlashList(SlashList {
                    query: "/".to_string(),
                    selected: 0,
                });
                KeyAction::None
            }
            KeyCode::Char('?') if self.draft.is_empty() => {
                self.panel = Panel::Help;
                KeyAction::None
            }
            KeyCode::Char(ch) => {
                self.draft.insert(&ch.to_string());
                KeyAction::None
            }
            _ => KeyAction::None,
        }
    }

    /// Answer a chord. A chord never types its letter. It carries the readline motions,
    /// the newline keys, the reverse search, and the two editor bindings.
    fn handle_chord(&mut self, key: KeyEvent) -> KeyAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        // A panel owns the keyboard. A chord while the search is open still edits the
        // query through its own handler, so a stray chord never leaks into the draft.
        if matches!(self.panel, Panel::HistorySearch(_)) {
            return KeyAction::None;
        }
        match key.code {
            // Ctrl-D on an empty draft leaves rho, the way it ends a shell. With a draft
            // it does nothing, because a stray chord must not throw away written work.
            KeyCode::Char('d') if ctrl && self.draft.is_empty() && self.panel == Panel::None => {
                KeyAction::Exit
            }
            // Shift+Enter is the newline key. Alt+Enter and Ctrl+Enter do it too, like
            // Ctrl-J, because a terminal that hides one modifier must leave a way in.
            KeyCode::Enter => {
                self.draft.insert_newline();
                KeyAction::None
            }
            KeyCode::Char('j') if ctrl => {
                self.draft.insert_newline();
                KeyAction::None
            }
            // Ctrl-X arms the editor. Ctrl-G opens it at once. Both edit the draft.
            KeyCode::Char('x') if ctrl => {
                self.awaiting_editor = true;
                KeyAction::None
            }
            KeyCode::Char('g') if ctrl => KeyAction::EditDraft(self.draft.model_text()),
            // Ctrl-R opens the reverse search. It keeps the draft, so Esc can restore it.
            KeyCode::Char('r') if ctrl => {
                self.panel = Panel::HistorySearch(HistorySearch::default());
                KeyAction::None
            }
            KeyCode::Char('a') if ctrl => {
                self.draft.move_line_start();
                KeyAction::None
            }
            KeyCode::Char('e') if ctrl => {
                self.draft.move_line_end();
                KeyAction::None
            }
            KeyCode::Char('k') if ctrl => {
                self.draft.kill_to_line_end();
                KeyAction::None
            }
            KeyCode::Char('u') if ctrl => {
                self.draft.kill_to_line_start();
                KeyAction::None
            }
            KeyCode::Char('w') if ctrl => {
                self.draft.kill_word_left();
                KeyAction::None
            }
            KeyCode::Char('y') if ctrl => {
                self.draft.yank();
                KeyAction::None
            }
            KeyCode::Char('b') if alt => {
                self.draft.move_word_left();
                KeyAction::None
            }
            KeyCode::Char('f') if alt => {
                self.draft.move_word_right();
                KeyAction::None
            }
            _ => KeyAction::None,
        }
    }

    /// Answer a key while the reverse search is open. The panel owns the keyboard, so a
    /// character edits the query and never the draft. Enter accepts the match. Esc keeps
    /// the draft that was there before the search opened.
    fn handle_search_key(&mut self, code: KeyCode, mut search: HistorySearch) -> KeyAction {
        match code {
            KeyCode::Esc => {
                self.panel = Panel::None;
            }
            KeyCode::Enter => {
                let matches = filter_history(&self.history, &search.query);
                if let Some(&index) = matches.get(search.selected) {
                    self.draft.set_text(&self.history[index]);
                }
                self.panel = Panel::None;
            }
            KeyCode::Up => {
                search.selected = search.selected.saturating_sub(1);
                self.panel = Panel::HistorySearch(search);
            }
            KeyCode::Down => {
                let count = filter_history(&self.history, &search.query).len();
                search.selected = (search.selected + 1).min(count.saturating_sub(1));
                self.panel = Panel::HistorySearch(search);
            }
            KeyCode::Backspace => {
                search.query.pop();
                search.selected = 0;
                self.panel = Panel::HistorySearch(search);
            }
            KeyCode::Char(ch) => {
                search.query.push(ch);
                search.selected = 0;
                self.panel = Panel::HistorySearch(search);
            }
            _ => {}
        }
        KeyAction::None
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
                self.draft.backspace();
                if self.draft.is_empty() {
                    self.panel = Panel::None;
                } else {
                    list.query = self.draft_text();
                    list.selected = 0;
                    self.panel = Panel::SlashList(list);
                }
            }
            KeyCode::Char(ch) => {
                self.draft.insert(&ch.to_string());
                list.query = self.draft_text();
                list.selected = 0;
                self.panel = Panel::SlashList(list);
            }
            KeyCode::Enter => return self.run_selected_command(&list),
            // Tab completes the draft to the selected command, the way a shell completes
            // a path. It runs nothing, so the user still sees what will run.
            KeyCode::Tab => {
                if let Some(command) = crate::filter_slash_commands(&list.query).get(list.selected)
                {
                    self.draft.set_text(command.name);
                    list.query = self.draft_text();
                    list.selected = 0;
                    self.panel = Panel::SlashList(list);
                }
            }
            _ => {}
        }
        KeyAction::None
    }

    /// Answer a key while the help panel is open. Esc closes it, and so does `?`.
    /// The largest help offset the screen can show.
    ///
    /// `help_visible_rows` is set by the app loop each frame, so the reducer clamps against
    /// the same geometry the renderer draws with.
    fn help_scroll_max(&self) -> usize {
        // Zero means the app loop has not drawn yet, and it must not mean "one row". A
        // zero here clamped the offset to almost nothing, so a scroll key banked presses
        // that the screen never answered. The standard band is the honest fallback.
        let visible = if self.help_visible_rows == 0 {
            crate::render::help_visible_rows(crate::render::BAND_ROWS)
        } else {
            self.help_visible_rows
        };
        crate::bindings::bindings()
            .len()
            .saturating_sub(visible.max(1))
    }

    fn handle_help_key(&mut self, code: KeyCode) -> KeyAction {
        match code {
            KeyCode::Esc | KeyCode::Char('?') => {
                self.panel = Panel::None;
                self.help_offset = 0;
                KeyAction::None
            }
            // The window states `↓ n more below`, and the footer offers `↑ ↓ scroll`. Both
            // are promises, so the arrows answer them. The panel drew both while these
            // keys did nothing, which is `D-a-panel-nobody-can-open`.
            // The offset is clamped here, where it moves, and not only where it draws.
            // Clamping at draw time let this value climb past the last row, so a press
            // back moved nothing until the user had paid back every press. Scroll state
            // that the screen cannot show is scroll state that lies.
            KeyCode::Down => {
                self.help_offset = self
                    .help_offset
                    .saturating_add(1)
                    .min(self.help_scroll_max());
                KeyAction::None
            }
            KeyCode::Up => {
                self.help_offset = self.help_offset.saturating_sub(1);
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
            let message = match crate::run_slash_command(&self.draft_text()) {
                crate::SlashOutcome::Unknown(message) => message,
                crate::SlashOutcome::Run(name) => format!("unknown command: {name}"),
            };
            self.panel = Panel::None;
            self.push_error(message);
            return KeyAction::None;
        };

        // A matched command consumes the draft, because the draft was the command.
        self.draft.take();
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
        self.push_row(
            Row::Error {
                message: crate::sanitize_line(&message.into()),
                detail: Vec::new(),
            },
            None,
        );
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

/// True when nothing can change the row at `index` again.
///
/// A frozen row lives in the terminal's scrollback, and rho can never repaint it. So this
/// answers the only question that matters before a row leaves the band.
///
/// There is one arm per `Row` variant and no wildcard arm, so a new variant fails the
/// build. Its author must then state the rule. See `D-row-finality-is-explicit`.
///
/// Two invariants hold the assistant and thinking rules up. The row list is append-only,
/// so no row moves. Both delta paths search from the newest row, so only the newest row of
/// a kind can grow.
pub fn row_is_final(state: &TuiState, index: usize) -> bool {
    let Some(row) = state.rows.get(index) else {
        return false;
    };
    // A finished turn settles every row, because no event can arrive for it.
    let turn_ended = state.activity == ActivityState::Idle;
    match row {
        // No reducer path writes a user row after it is pushed.
        Row::User { .. } => true,
        // Nothing updates an error row.
        Row::Error { .. } => true,
        // `append_answer` writes only the newest assistant row.
        Row::Assistant { .. } => turn_ended || newer_row_of_kind(state, index, RowKind::Assistant),
        // `last_thinking_mut` reaches only the newest thinking row.
        Row::Thinking { .. } => turn_ended || newer_row_of_kind(state, index, RowKind::Thinking),
        // `tool_row_mut` finds a pending or a running row by id. A tool call cannot
        // outlive its turn, so a finished turn settles it too.
        Row::Tool { status, .. } => {
            turn_ended || matches!(status, ToolRowStatus::Ok | ToolRowStatus::Failed)
        }
        // `agent_row_mut` finds a live child by id. A child outlives the turn that spawned
        // it, so a finished turn proves nothing here.
        Row::Agent { finished, .. } => *finished,
        // `task_row_mut` finds a live task by id. A task outlives its turn too.
        Row::Task { finished, .. } => *finished,
    }
}

/// The two row kinds that grow by delta.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowKind {
    Assistant,
    Thinking,
}

/// True when a newer row of the same kind exists, so a delta cannot reach `index`.
fn newer_row_of_kind(state: &TuiState, index: usize, kind: RowKind) -> bool {
    state.rows.iter().skip(index + 1).any(|row| match kind {
        RowKind::Assistant => matches!(row, Row::Assistant { .. }),
        RowKind::Thinking => matches!(row, Row::Thinking { .. }),
    })
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
