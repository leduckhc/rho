# SPEC-05 — Minimal TUI

Status: draft for sprint 1.
Owning crate: `rho-tui`. Consumed by `rho-cli`.

The TUI renders a conversation. Its state is a pure function of events. The
renderer is tested with a `ratatui` test backend, never a real terminal. Input
never blocks on model work.

Features covered: F-80 (minimal TUI), F-81 (pure-function render), F-82
(non-blocking input, Ctrl-C behaviour), F-86 (time to first frame).

`rho-tui` depends on `ratatui`, `crossterm`, `unicode-width`, and `rho-core`. It
does not depend on any provider crate. A different frontend can replace it.

## 1. Design rules

- The state is a value. A pure reducer folds one `AgentEvent` into the state. The
  reducer does no IO. A test drives it with a scripted event list.
- The renderer is a pure function of the state and the frame area. It does no IO.
- The event loop reads two sources: terminal input and the agent event stream. It
  never blocks one on the other.
- Ctrl-C cancels the running turn. A second Ctrl-C, when idle, exits.

## 2. State model

```rust
use rho_core::{AgentStopReason, StopReason, ToolKind};

/// One rendered transcript row.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// A finished or streaming user message.
    User { text: String },
    /// A finished or streaming assistant answer.
    Assistant { text: String },
    /// A thinking block. Collapsed to one line by default.
    Thinking { text: String },
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolRowStatus {
    Pending,
    Running,
    Ok,
    Failed,
}

/// Whether the agent is idle or running a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Running,
}

/// The whole TUI state. A pure function of the events applied so far, plus the
/// local input buffer and one flag for the Ctrl-C exit gate.
#[derive(Clone, Debug, Default)]
pub struct TuiState {
    pub rows: Vec<Row>,
    pub input: String,
    pub activity: ActivityState,
    pub status: String,
    /// True after a first Ctrl-C while idle. A second Ctrl-C then exits.
    pub exit_armed: bool,
    /// Set when the run ends. Drives the status line.
    pub last_stop: Option<AgentStopReason>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ActivityState {
    #[default]
    Idle,
    Running,
}
```

## 3. The reducer

The reducer is the only way to change transcript state from an agent event. It is
pure and total. It appends and edits rows, but it never does IO.

```rust
use rho_core::AgentEvent;

impl TuiState {
    /// Fold one agent event into the state. Pure. No IO.
    pub fn apply(&mut self, event: &AgentEvent);

    /// Append the submitted user input as a `User` row and clear the input.
    pub fn submit_input(&mut self) -> String;
}
```

Reducer rules:
- `TurnStart`: set `activity` to `Running`.
- `Stream(TextStart)`: push an empty `Assistant` row.
- `Stream(TextDelta)`: append the delta to the current `Assistant` row.
- `Stream(ThinkingStart)`: push an empty `Thinking` row.
- `Stream(ThinkingDelta)`: append the delta to the current `Thinking` row.
- `Stream(ToolCallEnd)`: push a `Tool` row with `status: Pending`.
- `ToolStart`: set the matching `Tool` row to `Running`.
- `ToolUpdate`: set the matching `Tool` row `preview` to the latest line.
- `ToolEnd`: set the matching `Tool` row to `Ok` or `Failed` from `is_error`.
- `TurnEnd`: no state change beyond keeping rows.
- `AgentEnd`: set `activity` to `Idle`, record `last_stop`, set the status line.

## 4. Rendering

The renderer draws the state into a `ratatui` frame. It is a pure function. A test
renders into a `TestBackend` and asserts on the buffer.

```rust
use ratatui::Frame;

/// Draw the whole UI. Pure. No IO. Safe to call every frame.
pub fn render(state: &TuiState, frame: &mut Frame<'_>);
```

Layout, top to bottom:
- The transcript area. It shows rows in order. It scrolls to the newest row.
- The status line. It shows the activity, the model name, and the last stop
  reason. It shows a spinner glyph while `Running`.
- The input editor. One or more lines. It shows the current `input` buffer and a
  cursor.

Rules:
- Each rendered line fits the frame width. Use `unicode-width` to measure.
- A `Thinking` row renders dimmed and collapsed to its first line by default.
- A `Tool` row renders its name, a kind glyph, a status glyph, and the preview.

## 5. The event loop and non-blocking input

The loop owns the terminal. It reads terminal input and agent events. It never
blocks input on model work. It uses `tokio::select!` over two sources: a
`crossterm` event stream and the `rho_core::AgentEvents` stream.

```rust
use rho_core::{AgentEvents, CancelToken, Session};

pub struct App {
    state: TuiState,
    session: Session,
    cancel: Option<CancelToken>,
    events: Option<AgentEvents>,
}

impl App {
    pub fn new(session: Session) -> Self;

    /// Run the UI until the user exits. Owns the terminal for its lifetime.
    pub async fn run(&mut self) -> Result<(), TuiError>;
}

#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    #[error("terminal io error: {0}")]
    Io(String),
}
```

Loop rules:
- On a printable key while the input is focused: append to `input`. This never
  waits on the model.
- On Enter: submit the input, append a `User` row, and call `session.prompt`. Keep
  the returned `AgentEvents` and its `CancelToken`.
- On an agent event: apply it to the state and request a redraw.
- On Ctrl-C while `Running`: call `cancel.cancel()`. Do not exit.
- On Ctrl-C while `Idle` and not armed: set `exit_armed`, show a hint. On the next
  Ctrl-C while armed: exit.
- Any key other than Ctrl-C clears `exit_armed`.
- A redraw is requested by state change, not by a timer. A spinner uses a short
  tick only while `Running`.

## 6. Time to first frame (F-86)

The app draws the first frame before the first token arrives. It draws an empty
transcript, the status line, and the input editor at start-up. The time to first
frame is measured in S11 and recorded in `docs/benchmarks.md`. This spec states
no number. See decision D-003.

## 7. Test cases

All render tests use `ratatui::backend::TestBackend`. No test opens a real
terminal.

Reducer tests, in `crates/rho-tui/tests/`:
- `reducer_text_delta_appends_to_assistant_row` — two `TextDelta` events build one
  `Assistant` row with the joined text.
- `reducer_thinking_delta_builds_thinking_row` — thinking deltas build a
  `Thinking` row.
- `reducer_tool_call_end_pushes_pending_tool_row` — a `ToolCallEnd` pushes a
  `Tool` row with `status: Pending`.
- `reducer_tool_start_sets_running` — `ToolStart` sets the row to `Running`.
- `reducer_tool_end_error_sets_failed` — `ToolEnd` with `is_error` sets `Failed`.
- `reducer_agent_end_sets_idle_and_stop` — `AgentEnd` sets `Idle` and records the
  stop reason.
- `reducer_is_pure_same_events_same_state` — applying the same event list twice
  from `default` yields equal states.

Render tests:
- `render_shows_transcript_rows` — the buffer holds the assistant text.
- `render_shows_status_line` — the buffer holds the activity and model name.
- `render_shows_input_buffer` — the buffer holds the typed input.
- `render_thinking_row_is_dimmed_and_collapsed` — a long thinking block renders on
  one line.
- `render_lines_fit_width` — no rendered line is wider than the frame width.

Input tests:
- `input_key_appends_while_running` — a key press appends to `input` even while
  `activity` is `Running`.
- `ctrl_c_while_running_cancels_not_exits` — one Ctrl-C during a run calls cancel
  and does not exit.
- `ctrl_c_twice_while_idle_exits` — two Ctrl-C presses while idle exit the app.
- `key_after_first_ctrl_c_disarms_exit` — a normal key after one Ctrl-C clears the
  exit arm.

## 8. Out of scope for sprint 1

- Themes (F-83) and keybinding config (F-84).
- A custom tool renderer (F-85).
- Mouse input and scrollback search.
- Markdown and syntax highlighting in the transcript. Sprint 1 renders plain text.
- Image rendering in the terminal. An image row shows a placeholder line.
- A model picker and a session picker in the TUI.
