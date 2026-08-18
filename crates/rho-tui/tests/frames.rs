//! The design frames are fixtures, and a fixture that does not line up is a lie.
//!
//! `docs/design/tui-frames/` holds ten frames that `docs/tui-design.md` treats as the
//! layout acceptance criteria. This file guards the fixtures themselves: the exact
//! display width, the exact row count, and the absence of a character that would break
//! a monospace grid.
//!
//! **Why a display width and not a length.** A box character such as `─` is three bytes
//! and one column. The controller's first check of these frames used a byte length and
//! reported every line as wrong. So width here means what the terminal draws, measured
//! the same way `crates/rho-tui/src/sanitize.rs` measures it.
//!
//! The render comparison, which drives the real renderer and asserts it reproduces each
//! frame, needs the renderer that stage U4 builds. `SPEC-tui-experience` names those ten
//! tests, and they land with the implementation.

use std::path::{Path, PathBuf};

use unicode_width::UnicodeWidthStr;

/// The rows a frame holds, excluding the two fence lines.
const FRAME_ROWS: usize = 24;

fn frames_dir() -> PathBuf {
    // The tests run with the crate root as the working directory.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root")
        .join("docs/design/tui-frames")
}

/// The frame body, without the opening and closing fence.
fn frame_lines(name: &str) -> Vec<String> {
    let path = frames_dir().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    text.lines()
        .filter(|line| !line.starts_with("```"))
        .map(str::to_string)
        .collect()
}

/// Assert one frame is exactly `columns` wide on every row, and `FRAME_ROWS` tall.
fn assert_frame(name: &str, columns: usize) {
    let lines = frame_lines(name);
    assert_eq!(
        lines.len(),
        FRAME_ROWS,
        "{name} must hold {FRAME_ROWS} rows, it holds {}",
        lines.len()
    );
    for (number, line) in lines.iter().enumerate() {
        let width = UnicodeWidthStr::width(line.as_str());
        assert_eq!(
            width,
            columns,
            "{name} row {} is {width} columns, it must be {columns}: {line:?}",
            number + 1
        );
    }
}

/// A frame may hold no character that breaks the grid.
///
/// A wide character occupies two cells, and a combining character occupies none. Either
/// one makes a hand-drawn frame disagree with the terminal, and the disagreement is
/// invisible in a diff.
fn assert_grid_safe(name: &str) {
    for (number, line) in frame_lines(name).iter().enumerate() {
        for character in line.chars() {
            let width = UnicodeWidthStr::width(character.to_string().as_str());
            assert_eq!(
                width,
                1,
                "{name} row {} holds {character:?}, which is {width} columns wide",
                number + 1
            );
        }
    }
}

#[test]
fn frame_fixture_100_idle_is_exact() {
    assert_frame("100-idle.txt", 100);
}

#[test]
fn frame_fixture_100_streaming_is_exact() {
    assert_frame("100-streaming.txt", 100);
}

#[test]
fn frame_fixture_100_tool_run_is_exact() {
    assert_frame("100-tool-run.txt", 100);
}

#[test]
fn frame_fixture_100_approval_is_exact() {
    assert_frame("100-approval.txt", 100);
}

#[test]
fn frame_fixture_100_error_is_exact() {
    assert_frame("100-error.txt", 100);
}

#[test]
fn frame_fixture_100_empty_is_exact() {
    assert_frame("100-empty.txt", 100);
}

#[test]
fn frame_fixture_100_slash_list_is_exact() {
    assert_frame("100-slash-list.txt", 100);
}

#[test]
fn frame_fixture_100_help_is_exact() {
    assert_frame("100-help.txt", 100);
}

#[test]
fn frame_fixture_80_streaming_is_exact() {
    assert_frame("80-streaming.txt", 80);
}

#[test]
fn frame_fixture_40_streaming_is_exact() {
    assert_frame("40-streaming.txt", 40);
}

#[test]
fn every_frame_fixture_is_grid_safe() {
    for name in [
        "100-idle.txt",
        "100-streaming.txt",
        "100-tool-run.txt",
        "100-approval.txt",
        "100-error.txt",
        "100-empty.txt",
        "100-slash-list.txt",
        "100-help.txt",
        "80-streaming.txt",
        "40-streaming.txt",
    ] {
        assert_grid_safe(name);
    }
}

#[test]
fn the_frame_set_is_complete() {
    // A missing frame would silently reduce the acceptance criteria, so the count is
    // pinned. Adding a frame is a deliberate act that updates this test.
    let mut found: Vec<String> = std::fs::read_dir(frames_dir())
        .expect("the frames directory")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".txt"))
        .collect();
    found.sort();
    assert_eq!(
        found.len(),
        10,
        "the design states ten frames, the directory holds {}: {found:?}",
        found.len()
    );
}
