//! Concise-mode tests. A collapsed tool row keeps its header. An expanded row adds
//! the body. Concise mode is opt-in, and the default is off. See
//! `SPEC-tui-experience` section 7.

use rho_core::ToolKind;
use rho_tui::{
    CONCISE_MODE_DEFAULT, Row, RowFold, ToolRowStatus, fold_caret, initial_tool_fold, toggle_fold,
    tool_row_lines,
};

/// A finished tool row whose body holds one line of output.
fn tool_row(status: ToolRowStatus) -> Row {
    Row::Tool {
        id: "call-1".to_string(),
        name: "bash".to_string(),
        kind: ToolKind::Execute,
        status,
        preview: "the body output line".to_string(),
    }
}

#[test]
fn concise_mode_default_is_off() {
    // A fresh state shows tool bodies by default, because concise mode is opt-in.
    // With the default setting, a tool row is expanded, so its body shows. Assert
    // the default rather than assuming it.
    let fold = initial_tool_fold(CONCISE_MODE_DEFAULT, false);
    assert_eq!(
        fold,
        RowFold::Expanded,
        "the default is not off: a fresh tool row hid its body"
    );
}

#[test]
fn collapsed_row_expands() {
    // `enter` on a collapsed row shows its body. A collapsed row keeps the header
    // and hides the body. Toggling it expands the row, so the body shows.
    let row = tool_row(ToolRowStatus::Ok);

    let collapsed = tool_row_lines(&row, RowFold::Collapsed, 100);
    let header = collapsed.join("\n");
    assert!(
        header.contains("bash"),
        "the collapsed header dropped the verb:\n{header}"
    );
    assert!(
        !header.contains("the body output line"),
        "the collapsed row showed its body:\n{header}"
    );

    let expanded = tool_row_lines(&row, toggle_fold(RowFold::Collapsed), 100);
    let shown = expanded.join("\n");
    assert!(
        shown.contains("bash"),
        "the expanded row dropped the header:\n{shown}"
    );
    assert!(
        shown.contains("the body output line"),
        "the expanded row hid its body:\n{shown}"
    );
}

#[test]
fn failed_row_expands_itself() {
    // A failed tool row shows its output with no key press, even with concise mode
    // on, because the output is the point.
    let fold = initial_tool_fold(true, true);
    assert_eq!(
        fold,
        RowFold::Expanded,
        "a failed row stayed collapsed under concise mode"
    );
}

#[test]
fn concise_caret_shows_fold_state() {
    // A collapsed row shows `▸`, an expanded row shows `▾`.
    assert_eq!(fold_caret(RowFold::Collapsed), "▸");
    assert_eq!(fold_caret(RowFold::Expanded), "▾");
}
