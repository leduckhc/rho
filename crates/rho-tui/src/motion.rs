//! The one motion: a highlight band that sweeps the working word.
//!
//! The frame is a pure function of a tick. No function here reads a clock. The
//! tick is the only time source, so a test asserts any frame by its tick. See
//! `SPEC-tui-experience` section 8, and `docs/tui-prior-art.md`, the codex
//! section, for why the render must never read a process clock.

/// The sweep period, in ticks. Each tick is 100 milliseconds, so the period is 2 s.
pub const SWEEP_PERIOD_TICKS: u64 = 20;

/// The raised-cosine weight of one column at one tick, from 0.0 to 1.0.
///
/// The frame is a pure function of `tick`, so a test asserts a frame by its tick.
/// The band half width is five columns, with ten columns of padding at each end.
pub fn sweep_weight(_tick: u64, _column: usize) -> f32 {
    todo!("sweep_weight is unimplemented in the red stage")
}

/// The no-true-colour rendering of one swept cell, by weight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionCell {
    Dim,
    Plain,
    Bold,
}

/// Map a sweep weight to its 256-colour or no-colour tier.
pub fn motion_cell(_weight: f32) -> MotionCell {
    todo!("motion_cell is unimplemented in the red stage")
}

/// The inputs that decide whether the sweep animates.
///
/// The sweep stops, and the word renders plain, under any one stop condition. Each
/// field is one condition from `SPEC-tui-experience` section 8.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MotionInputs {
    /// The `tui.motion` setting. False stops the sweep.
    pub tui_motion: bool,
    /// The `--no-motion` flag. True stops the sweep.
    pub no_motion_flag: bool,
    /// Whether stdout is a terminal. A non-terminal stdout stops the sweep.
    pub stdout_is_terminal: bool,
    /// The `tui.reduce_motion` setting. True stops the sweep.
    pub reduce_motion_setting: bool,
    /// The `RHO_REDUCE_MOTION=1` variable. True stops the sweep.
    pub reduce_motion_env: bool,
}

impl MotionInputs {
    /// The inputs under which the sweep animates: motion on, a terminal stdout, and
    /// no reduced-motion preference.
    pub fn animating() -> Self {
        Self {
            tui_motion: true,
            no_motion_flag: false,
            stdout_is_terminal: true,
            reduce_motion_setting: false,
            reduce_motion_env: false,
        }
    }
}

/// True when the sweep animates. False under any one stop condition.
pub fn motion_enabled(_inputs: &MotionInputs) -> bool {
    todo!("motion_enabled is unimplemented in the red stage")
}

/// The rendered cells of the working word at one tick.
///
/// One `MotionCell` per column of `word`. When motion is off, every cell is
/// `Plain`, at every tick, so the word renders as a still, plain frame.
pub fn sweep_frame(_word: &str, _tick: u64, _inputs: &MotionInputs) -> Vec<MotionCell> {
    todo!("sweep_frame is unimplemented in the red stage")
}
