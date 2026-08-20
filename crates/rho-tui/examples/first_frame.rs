//! Measure the time to the first rendered frame.
//!
//! The app draws the first frame before the first token arrives. This example
//! measures the cost of that first frame with a `ratatui` `TestBackend`, so it
//! runs with no real terminal. It prints the elapsed time from the start of
//! `main` to the completed first render. Wrap the whole run in `/usr/bin/time`
//! to include process start and dynamic linking. See `SPEC-tui` section 6.

use std::time::Instant;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rho_tui::{TuiState, render};

fn main() {
    let start = Instant::now();
    let state = TuiState::default();
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("build test terminal");
    terminal
        .draw(|frame| render(&state, frame))
        .expect("draw the first frame");
    let elapsed = start.elapsed();
    println!("first frame in {} us", elapsed.as_micros());
}
