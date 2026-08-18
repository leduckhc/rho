//! The pure renderer.
//!
//! The renderer draws the state into a `ratatui` frame. It is a pure function of
//! the state and the frame area. It does no IO. A test renders into a
//! `TestBackend` and asserts on the buffer. See `SPEC-tui` section 4.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::sanitize::{fit_to_width, sanitize_line};
use crate::state::{ActivityState, Row, ToolRowStatus, TuiState};

/// The spinner glyph shown while a turn runs.
const SPINNER: &str = "*";

/// Draw the whole UI. Pure. No IO. Safe to call every frame.
///
/// The layout runs top to bottom: the transcript area, the status line, then the
/// input editor. Each rendered line fits the frame width, measured with
/// `unicode-width`.
pub fn render(state: &TuiState, frame: &mut Frame<'_>) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    let width = area.width as usize;
    render_transcript(state, frame, chunks[0], width);
    render_status(state, frame, chunks[1], width);
    render_input(state, frame, chunks[2], width);
}

fn render_transcript(
    state: &TuiState,
    frame: &mut Frame<'_>,
    area: ratatui::layout::Rect,
    width: usize,
) {
    let height = area.height as usize;
    let lines: Vec<Line> = state.rows.iter().map(|row| row_line(row, width)).collect();
    // Scroll so the newest rows stay visible.
    let start = lines.len().saturating_sub(height);
    let visible: Vec<Line> = lines.into_iter().skip(start).collect();
    frame.render_widget(Paragraph::new(visible), area);
}

fn row_line(row: &Row, width: usize) -> Line<'static> {
    match row {
        Row::User { text } => plain_line(
            format!("> {}", sanitize_line(text)),
            width,
            Style::default(),
        ),
        Row::Assistant { text } => plain_line(sanitize_line(text), width, Style::default()),
        Row::Thinking { text } => {
            // A thinking row renders dimmed and collapsed to its first line.
            let first = sanitize_line(text.lines().next().unwrap_or(""));
            plain_line(
                format!("thinking: {first}"),
                width,
                Style::default().add_modifier(Modifier::DIM),
            )
        }
        Row::Agent {
            name,
            depth,
            turns,
            cost,
            finished,
            failed,
            outcome,
            ..
        } => {
            // Indent by depth, so a fan-out is legible at a glance. A tree of children is
            // the point of the feature, and a flat list would hide it.
            let indent = "  ".repeat((*depth as usize).min(6));
            let glyph = if !*finished {
                "~"
            } else if *failed {
                "x"
            } else {
                "+"
            };
            let name = sanitize_line(name);
            let cost = sanitize_line(cost);
            let outcome = sanitize_line(outcome);
            let detail = if cost.is_empty() {
                format!("{outcome}, {turns} turn(s)")
            } else {
                format!("{outcome}, {turns} turn(s), {cost}")
            };
            let style = if *finished && *failed {
                Style::default().add_modifier(Modifier::BOLD)
            } else if !*finished {
                Style::default().add_modifier(Modifier::DIM)
            } else {
                Style::default()
            };
            plain_line(
                format!("{indent}{glyph} agent {name}  {detail}"),
                width,
                style,
            )
        }
        Row::Task {
            command,
            state,
            finished,
            failed,
            progress,
            ..
        } => {
            // A background task keeps its row after the turn ends, so the user can see
            // that work is still going. A running task shows a spinner glyph, a failed
            // one is marked, and progress goes on the same line.
            let glyph = if !*finished {
                "~"
            } else if *failed {
                "x"
            } else {
                "+"
            };
            // The command and the progress text are already sanitised in the reducer.
            // Sanitise again here, because render must never trust its input.
            let command = sanitize_line(command);
            let progress = sanitize_line(progress);
            let detail = if progress.is_empty() {
                state.clone()
            } else {
                format!("{state} {progress}")
            };
            let style = if *finished && *failed {
                Style::default().add_modifier(Modifier::BOLD)
            } else if !*finished {
                Style::default().add_modifier(Modifier::DIM)
            } else {
                Style::default()
            };
            plain_line(format!("{glyph} task {command}  {detail}"), width, style)
        }
        Row::Tool {
            name,
            status,
            preview,
            ..
        } => {
            let glyph = status_glyph(*status);
            let preview = sanitize_line(preview);
            plain_line(
                format!("{glyph} {name}  {preview}"),
                width,
                Style::default(),
            )
        }
    }
}

fn render_status(
    state: &TuiState,
    frame: &mut Frame<'_>,
    area: ratatui::layout::Rect,
    width: usize,
) {
    let activity = match state.activity {
        ActivityState::Idle => "idle".to_string(),
        ActivityState::Running => format!("{SPINNER} running"),
    };
    let model = if state.model.is_empty() {
        "no model".to_string()
    } else {
        state.model.clone()
    };
    let text = format!("[{activity}] {model}  {}", state.status);
    frame.render_widget(
        Paragraph::new(plain_line(
            text,
            width,
            Style::default().add_modifier(Modifier::REVERSED),
        )),
        area,
    );
}

fn render_input(
    state: &TuiState,
    frame: &mut Frame<'_>,
    area: ratatui::layout::Rect,
    width: usize,
) {
    // A cursor marks the end of the input line.
    let text = format!("{}\u{2588}", state.input);
    frame.render_widget(
        Paragraph::new(plain_line(text, width, Style::default())),
        area,
    );
}

/// Build one line that fits the width and carries the given style.
fn plain_line(text: String, width: usize, style: Style) -> Line<'static> {
    let fitted = fit_to_width(&text, width);
    Line::from(Span::styled(fitted, style))
}

/// A one-glyph marker for a tool status.
fn status_glyph(status: ToolRowStatus) -> &'static str {
    match status {
        ToolRowStatus::Pending => "·",
        ToolRowStatus::Running => "*",
        ToolRowStatus::Ok => "+",
        ToolRowStatus::Failed => "x",
    }
}
