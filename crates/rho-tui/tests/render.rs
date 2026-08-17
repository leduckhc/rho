//! Render tests. Every test renders into a `ratatui` `TestBackend`. No test
//! opens a real terminal. See `SPEC-05` section 7.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rho_core::{AgentEvent, StreamEvent};
use rho_tui::{TuiState, render};
use unicode_width::UnicodeWidthStr;

/// Render the state into a backend of the given size and return the frame text.
fn render_to_lines(state: &TuiState, width: u16, height: u16) -> Vec<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("build test terminal");
    terminal
        .draw(|frame| render(state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    let mut lines = Vec::new();
    for y in 0..height {
        let mut line = String::new();
        for x in 0..width {
            line.push_str(buffer[(x, y)].symbol());
        }
        lines.push(line);
    }
    lines
}

fn render_to_string(state: &TuiState, width: u16, height: u16) -> String {
    render_to_lines(state, width, height).join("\n")
}

#[test]
fn render_shows_transcript_rows() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::Stream(StreamEvent::TextStart { index: 0 }));
    state.apply(&AgentEvent::Stream(StreamEvent::TextDelta {
        index: 0,
        delta: "the answer".to_string(),
    }));

    let text = render_to_string(&state, 40, 10);
    assert!(text.contains("the answer"), "buffer was:\n{text}");
}

#[test]
fn render_shows_status_line() {
    let mut state = TuiState::default();
    state.model = "openai/gpt-4o".to_string();
    state.apply(&AgentEvent::TurnStart);

    let text = render_to_string(&state, 60, 6);
    assert!(text.contains("running"), "buffer was:\n{text}");
    assert!(text.contains("openai/gpt-4o"), "buffer was:\n{text}");
}

#[test]
fn render_shows_input_buffer() {
    let mut state = TuiState::default();
    state.input = "hello there".to_string();

    let text = render_to_string(&state, 40, 6);
    assert!(text.contains("hello there"), "buffer was:\n{text}");
}

#[test]
fn render_thinking_row_is_dimmed_and_collapsed() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::Stream(StreamEvent::ThinkingStart { index: 0 }));
    state.apply(&AgentEvent::Stream(StreamEvent::ThinkingDelta {
        index: 0,
        delta: "first line\nsecond line\nthird line".to_string(),
    }));

    let lines = render_to_lines(&state, 60, 8);
    // The thinking block collapses to one line, so the second line never renders.
    assert!(
        lines.iter().any(|line| line.contains("first line")),
        "lines were:\n{lines:#?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("second line")),
        "the thinking block did not collapse; lines were:\n{lines:#?}"
    );
}

#[test]
fn render_lines_fit_width() {
    let mut state = TuiState::default();
    state.apply(&AgentEvent::Stream(StreamEvent::TextStart { index: 0 }));
    state.apply(&AgentEvent::Stream(StreamEvent::TextDelta {
        index: 0,
        delta: "x".repeat(500),
    }));
    state.input = "y".repeat(500);

    let width = 40u16;
    let lines = render_to_lines(&state, width, 10);
    for line in &lines {
        assert!(
            line.trim_end().width() <= width as usize,
            "a rendered line was wider than the frame: {line:?}"
        );
    }
}

#[test]
fn render_sanitises_a_tool_preview_with_an_escape_sequence() {
    // Tool output is untrusted. A real escape sequence must not reach the buffer.
    let mut state = TuiState::default();
    state.apply(&AgentEvent::ToolStart {
        id: "call-1".to_string(),
        name: "bash".to_string(),
        kind: rho_core::ToolKind::Execute,
    });
    state.apply(&AgentEvent::ToolUpdate {
        id: "call-1".to_string(),
        output: "red\u{1b}[31mtext\u{1b}[0m".to_string(),
    });

    let text = render_to_string(&state, 60, 6);
    assert!(
        !text.contains('\u{1b}'),
        "an escape byte reached the buffer:\n{text:?}"
    );
    // The visible letters survive.
    assert!(text.contains("red"), "buffer was:\n{text}");
}
