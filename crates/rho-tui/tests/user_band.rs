//! A submitted prompt sits on a band, so the eye finds where each turn began.
//!
//! Both Claude Code and pi draw a full-width background behind a submitted user message, and the
//! owner asked for the same. It is the cheapest way to scan a long transcript: the bands are the
//! turn boundaries.
//!
//! **The band must cover every column**, including the columns past the end of the text. A band
//! that stops at the last word is a ragged stripe, which reads as a defect rather than a boundary.
//!
//! See `D-a-submitted-prompt-sits-on-a-band`.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;
use rho_tui::{Row, TuiState, render};

/// Every drawn row as its text and the background of each of its cells.
fn drawn(rows_in: Vec<Row>, width: u16, height: u16) -> Vec<(String, Vec<ratatui::style::Color>)> {
    let mut state = TuiState::default();
    state.rows = rows_in;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render(&state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| {
            let text: String = (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>();
            let bg = (0..width)
                .map(|x| buffer[(x, y)].style().bg.unwrap())
                .collect();
            (text, bg)
        })
        .collect()
}

#[test]
fn a_submitted_prompt_draws_a_band_across_every_column() {
    let rows = drawn(
        vec![Row::User {
            text: "run the tests".to_string(),
        }],
        60,
        14,
    );
    let (text, bg) = rows
        .iter()
        .find(|(text, _)| text.contains("run the tests"))
        .expect("the prompt draws");
    let first = bg[0];
    assert_ne!(
        first,
        ratatui::style::Color::Reset,
        "the prompt row carries a background: {text:?}"
    );
    for (column, cell) in bg.iter().enumerate() {
        assert_eq!(
            *cell, first,
            "the band must cover column {column} too, so it is not a ragged stripe: {text:?}"
        );
    }
}

#[test]
fn an_answer_carries_no_band() {
    // Only the prompt is banded. Banding the answer as well would make the whole screen a band and
    // mark nothing.
    let rows = drawn(
        vec![Row::Assistant {
            text: "the tests pass".to_string(),
        }],
        60,
        14,
    );
    let (_, bg) = rows
        .iter()
        .find(|(text, _)| text.contains("the tests pass"))
        .expect("the answer draws");
    assert!(
        bg.iter().all(|cell| *cell == ratatui::style::Color::Reset),
        "an answer keeps the terminal background"
    );
}

#[test]
fn a_wrapped_prompt_bands_every_one_of_its_rows() {
    // A long prompt takes several rows, and a band that covered only the first would look like a
    // rendering fault rather than one message.
    let long = "please run the whole test suite and then explain in detail what each failure means and why it happened".to_string();
    let rows = drawn(vec![Row::User { text: long }], 50, 16);
    let banded: Vec<&(String, Vec<ratatui::style::Color>)> = rows
        .iter()
        .filter(|(_, bg)| bg[0] != ratatui::style::Color::Reset)
        .collect();
    assert!(
        banded.len() >= 2,
        "a wrapped prompt bands every row, found {}",
        banded.len()
    );
    for (text, bg) in banded {
        assert!(
            bg.iter().all(|cell| *cell == bg[0]),
            "every column of a wrapped prompt row is banded: {text:?}"
        );
    }
}

#[test]
fn the_band_does_not_bleed_into_the_row_above_or_below() {
    // The blank separator rows around a turn stay unbanded, so each band is one message.
    let rows = drawn(
        vec![
            Row::Assistant {
                text: "before".to_string(),
            },
            Row::User {
                text: "middle".to_string(),
            },
            Row::Assistant {
                text: "after".to_string(),
            },
        ],
        60,
        18,
    );
    let banded: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, (_, bg))| bg[0] != ratatui::style::Color::Reset)
        .map(|(index, _)| index)
        .collect();
    assert_eq!(banded.len(), 1, "exactly one row is banded: {banded:?}");
    let row = banded[0];
    assert!(
        rows[row].0.contains("middle"),
        "and it is the prompt row: {:?}",
        rows[row].0
    );
}

#[test]
fn a_submitted_prompt_is_bold_in_every_colour_mode() {
    // The weight is intended, not a side effect. `style_for` reads modifiers from the 16-colour
    // table whatever the colour depth, so a prompt is bold beside its band as well as instead of
    // it. A 16-colour terminal has no subtle grey to band with, and the words are the user's own.
    let mut state = TuiState::default();
    state.rows.push(Row::User {
        text: "run the tests".to_string(),
    });
    let backend = TestBackend::new(60, 14);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render(&state, frame))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    let mut found = false;
    for y in 0..14u16 {
        let line: String = (0..60).map(|x| buffer[(x, y)].symbol()).collect();
        if line.contains("run the tests") {
            let index = line.find("run").expect("the text");
            let style = buffer[(index as u16, y)].style();
            assert!(
                style.add_modifier.contains(Modifier::BOLD),
                "a submitted prompt carries weight: {style:?}"
            );
            found = true;
        }
    }
    assert!(found, "the prompt drew");
}
