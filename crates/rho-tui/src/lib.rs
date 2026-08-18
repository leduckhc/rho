//! `rho-tui` is the minimal terminal user interface for rho.
//!
//! The state is a pure function of the events applied so far. A pure reducer
//! folds one `AgentEvent` into the state. A pure renderer draws the state into a
//! `ratatui` frame. The event loop is the only part that does IO. A frontend
//! author can replace this crate without a change in `rho-core`. See `SPEC-tui`.

mod app;
mod render;
mod sanitize;
mod state;

pub use app::{App, TuiError};
pub use render::render;
pub use sanitize::{fit_to_width, sanitize_line};
pub use state::{ActivityState, KeyAction, Row, ToolRowStatus, TuiState};
