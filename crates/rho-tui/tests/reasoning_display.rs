//! Reasoning display tests, from `SPEC-reasoning-across-providers` section 5.
//!
//! Every test renders into a `ratatui` `TestBackend`. No test opens a real terminal.
//! The reasoning text must draw in `Role::Muted` (256-colour index 245), never in the
//! default foreground, because it must not read as the answer.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use rho_core::{AgentEvent, ReasoningDisplay, StreamEvent};
use rho_tui::{TuiState, render};

/// A rendered cell: its symbol and its foreground colour.
struct Grid {
    rows: Vec<Vec<(String, Color)>>,
}

impl Grid {
    fn render(state: &TuiState, width: u16, height: u16) -> Grid {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("build test terminal");
        terminal
            .draw(|frame| render(state, frame))
            .expect("draw frame");
        let buffer = terminal.backend().buffer().clone();
        let mut rows = Vec::new();
        for y in 0..height {
            let mut row = Vec::new();
            for x in 0..width {
                let cell = &buffer[(x, y)];
                row.push((cell.symbol().to_string(), cell.fg));
            }
            rows.push(row);
        }
        Grid { rows }
    }

    fn text(&self) -> String {
        self.rows
            .iter()
            .map(|row| row.iter().map(|(s, _)| s.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The foreground colour of the first cell of a row that contains `needle`.
    fn fg_of_row_with(&self, needle: &str) -> Option<Color> {
        for row in &self.rows {
            let line: String = row.iter().map(|(s, _)| s.as_str()).collect();
            if let Some(byte) = line.find(needle) {
                // Map the byte position to a cell index. Every cell holds one grapheme,
                // and the reasoning text is ASCII, so the cell index is the char count.
                let cell = line[..byte].chars().count();
                return row.get(cell).map(|(_, fg)| *fg);
            }
        }
        None
    }
}

/// Drive one settled reasoning block, from `start` to `end` on the caller's clock.
fn thinking_block(state: &mut TuiState, text: &str, start: i64, end: i64) {
    state.apply(
        &AgentEvent::Stream(StreamEvent::ThinkingStart { index: 0 }),
        start,
    );
    state.apply(
        &AgentEvent::Stream(StreamEvent::ThinkingDelta {
            index: 0,
            delta: text.to_string(),
        }),
        start,
    );
    state.apply(
        &AgentEvent::Stream(StreamEvent::ThinkingEnd {
            index: 0,
            state: None,
        }),
        end,
    );
}

#[test]
fn the_default_mode_is_summary() {
    let state = TuiState::default();
    assert_eq!(state.reasoning_display, ReasoningDisplay::Summary);
}

#[test]
fn the_summary_row_states_the_span() {
    let mut state = TuiState::default();
    // 2.4 seconds of thinking.
    thinking_block(&mut state, "private reasoning", 0, 2_400);
    let grid = Grid::render(&state, 60, 10);
    let text = grid.text();
    assert!(
        text.contains("thought for 2.4s"),
        "summary must state the span, buffer was:\n{text}"
    );
    assert!(
        !text.contains("private reasoning"),
        "summary mode must not draw the reasoning text, buffer was:\n{text}"
    );
}

#[test]
fn full_mode_draws_the_text_dimmed() {
    let mut state = TuiState::default();
    state.reasoning_display = ReasoningDisplay::Full;
    thinking_block(&mut state, "private reasoning", 0, 2_400);
    let grid = Grid::render(&state, 60, 10);
    assert!(
        grid.text().contains("private reasoning"),
        "full mode must draw the reasoning text, buffer was:\n{}",
        grid.text()
    );
    assert_eq!(
        grid.fg_of_row_with("private reasoning"),
        Some(Color::Indexed(245)),
        "the reasoning text must draw in Role::Muted, never the default foreground"
    );
}

#[test]
fn off_mode_draws_nothing() {
    let mut state = TuiState::default();
    state.reasoning_display = ReasoningDisplay::Off;
    thinking_block(&mut state, "private reasoning", 0, 2_400);
    let text = Grid::render(&state, 60, 10).text();
    assert!(
        !text.contains("private reasoning"),
        "off mode must not draw the reasoning text, buffer was:\n{text}"
    );
    assert!(
        !text.contains("thought for"),
        "off mode must not draw the summary row either, buffer was:\n{text}"
    );
    assert!(
        !text.contains('∴'),
        "off mode must not draw the thinking glyph, buffer was:\n{text}"
    );
}

#[test]
fn live_mode_collapses_when_the_answer_starts() {
    let mut state = TuiState::default();
    state.reasoning_display = ReasoningDisplay::Live;

    // While the reasoning streams, before it ends, the text shows.
    state.apply(
        &AgentEvent::Stream(StreamEvent::ThinkingStart { index: 0 }),
        0,
    );
    state.apply(
        &AgentEvent::Stream(StreamEvent::ThinkingDelta {
            index: 0,
            delta: "private reasoning".to_string(),
        }),
        0,
    );
    let streaming = Grid::render(&state, 60, 10).text();
    assert!(
        streaming.contains("private reasoning"),
        "live mode shows the text while it streams, buffer was:\n{streaming}"
    );

    // Once the answer starts, the reasoning collapses to the summary row.
    state.apply(
        &AgentEvent::Stream(StreamEvent::ThinkingEnd {
            index: 0,
            state: None,
        }),
        2_400,
    );
    state.apply(
        &AgentEvent::Stream(StreamEvent::TextStart { index: 1 }),
        2_400,
    );
    state.apply(
        &AgentEvent::Stream(StreamEvent::TextDelta {
            index: 1,
            delta: "the answer".to_string(),
        }),
        2_400,
    );
    let collapsed = Grid::render(&state, 60, 10).text();
    assert!(
        !collapsed.contains("private reasoning"),
        "live mode collapses the text once the answer starts, buffer was:\n{collapsed}"
    );
    assert!(
        collapsed.contains("thought for 2.4s"),
        "the summary stays after the collapse, buffer was:\n{collapsed}"
    );
    assert!(
        collapsed.contains("the answer"),
        "the answer draws, buffer was:\n{collapsed}"
    );
}
