//! `rho-tui` is the minimal terminal user interface for rho.
//!
//! The state is a pure function of the events applied so far. A pure reducer
//! folds one `AgentEvent` into the state. A pure renderer draws the state into a
//! `ratatui` frame. The event loop is the only part that does IO. A frontend
//! author can replace this crate without a change in `rho-core`. See `SPEC-tui`.

mod app;
mod bindings;
mod concise;
mod duration;
mod editor;
mod markdown;
mod motion;
mod paste;
mod render;
mod sanitize;
mod screen;
mod scroll;
mod state;
mod styled;
mod theme;

pub use app::{App, TuiError, edit_draft, restore_sequences, setup_sequences};
pub use duration::{DURATION_SLOT_COLUMNS, duration_slot, format_duration, live_duration_is_amber};
pub use editor::{editor_argv, editor_command};
pub use markdown::{InlineRun, MarkdownKind, MarkdownLine, scan_inline, scan_markdown};
pub use paste::{
    AttachOutcome, BurstKey, COMPOSER_MAX_TEXT_ROWS, Composer, IMAGE_MAX_BYTES, ImageChip,
    LARGE_PASTE_CHARS, PasteChip, RoutedInput, Unit, attach_image, image_chip_label,
    paste_chip_label, route_burst,
};
pub use render::{
    STARTUP_MIN_ROWS, ScreenLayout, banner_line, plan_screen, render, slash_row_index,
    transcript_metrics,
};
pub use sanitize::{fit_to_width, sanitize_line};
pub use screen::{ScreenGuard, enter_sequences};
pub use scroll::{PAGE_ROWS_MARGIN, Scroll, WHEEL_ROWS};
pub use state::{
    ActivityState, Approval, HistorySearch, KeyAction, Panel, Row, SlashList, ToolRowStatus,
    TuiState, filter_history,
};
pub use styled::{StyledLine, styled_text, styled_width};

pub use bindings::{
    Binding, SlashCommand, SlashOutcome, bindings, filter_slash_commands, help_rows,
    run_slash_command, slash_commands,
};
pub use concise::{
    CONCISE_MODE_DEFAULT, RowFold, fold_caret, initial_tool_fold, toggle_fold, tool_row_lines,
};
pub use motion::{
    MotionCell, MotionInputs, SWEEP_PERIOD_TICKS, motion_cell, motion_enabled, sweep_frame,
    sweep_weight,
};
pub use theme::{Ansi16, Role, RoleStyle, role_16, role_256, role_bg_256, role_none};
