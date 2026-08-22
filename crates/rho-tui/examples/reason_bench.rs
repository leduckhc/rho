//! Frame cost while one reasoning row grows, in `full` mode.
//!
//! A performance review reported an O(N squared) redraw: the whole accumulated reasoning text
//! is sanitised and wrapped on every frame, and only then truncated to the band. This example
//! is the measurement. See `docs/benchmarks.md` for the command and the numbers.

use std::time::Instant;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rho_tui::{Row, TuiState, render};

const COLS: u16 = 100;
const ROWS: u16 = 30;

fn main() {
    let deltas: usize = std::env::var("RHO_DELTAS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1000);

    let backend = TestBackend::new(COLS, ROWS);
    let mut terminal = Terminal::new(backend).expect("build a test terminal");
    let mut state = TuiState::default();
    state.reasoning_display = rho_core::ReasoningDisplay::Full;
    state.rows.push(Row::Thinking {
        text: String::new(),
    });

    let chunk = "the model considers the question carefully and at some length. ";
    let start = Instant::now();
    for _ in 0..deltas {
        if let Some(Row::Thinking { text }) = state.rows.last_mut() {
            text.push_str(chunk);
        }
        terminal
            .draw(|frame| render(&state, frame))
            .expect("draw a frame");
    }
    let total = start.elapsed();
    println!(
        "deltas={deltas} row_bytes={} total_ms={} avg_frame_us={}",
        chunk.len() * deltas,
        total.as_millis(),
        total.as_micros() / deltas.max(1) as u128
    );
}
