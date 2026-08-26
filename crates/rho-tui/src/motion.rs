//! The one motion: a highlight band that sweeps the working word.
//!
//! The frame is a pure function of a tick. No function here reads a clock. The
//! tick is the only time source, so a test asserts any frame by its tick. See
//! `SPEC-tui-experience` section 8, and `docs/tui-prior-art.md`, the codex
//! section, for why the render must never read a process clock.

/// The sweep period, in ticks. Each tick is 100 milliseconds, so the period is 2 s.
pub const SWEEP_PERIOD_TICKS: u64 = 20;

/// The band half width, in columns. The raised cosine reaches zero at this distance,
/// so the band footprint spans at most eleven columns: five each side, plus the peak.
const BAND_HALF_WIDTH: f32 = 5.0;

/// The columns that pad the lead-in, so the band enters from off the word. The band
/// starts this far left of column zero and sweeps right, leaving cleanly to the right.
const LEAD_IN_PADDING: f32 = 10.0;

/// The columns the band centre advances each tick. Two columns per tick carries the
/// centre from the left padding, across the word, and out to the right before the wrap,
/// so the frame is plain at the wrap and the sweep is continuous across it.
const COLUMNS_PER_TICK: f32 = 2.0;

/// The raised-cosine weight of one column at one tick, from 0.0 to 1.0.
///
/// The frame is a pure function of `tick`, so a test asserts a frame by its tick. The
/// weight reads `tick % SWEEP_PERIOD_TICKS`, so `tick` and `tick + 20` are identical
/// and the sweep returns to its start after one period. No clock is read here: the
/// tick is the only time source. The band half width is five columns, with ten columns
/// of padding on the lead-in so the band enters and leaves cleanly.
pub fn sweep_weight(tick: u64, column: usize) -> f32 {
    let phase = (tick % SWEEP_PERIOD_TICKS) as f32;
    let center = phase * COLUMNS_PER_TICK - LEAD_IN_PADDING;
    let distance = (column as f32 - center).abs();
    if distance >= BAND_HALF_WIDTH {
        0.0
    } else {
        0.5 * (1.0 + (std::f32::consts::PI * distance / BAND_HALF_WIDTH).cos())
    }
}

/// The no-true-colour rendering of one swept cell, by weight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionCell {
    Dim,
    Plain,
    Bold,
}

/// The weight below which a cell is dimmed, and at or below which it is plain. Above
/// the plain ceiling the cell is bold. Three steps, matching the design's tiers.
const DIM_CEILING: f32 = 0.2;
const PLAIN_CEILING: f32 = 0.6;

/// Map a sweep weight to its 256-colour or no-colour tier. Below 0.2 the cell is dim,
/// to 0.6 it is plain, above 0.6 it is bold. A pure function of the weight.
pub fn motion_cell(weight: f32) -> MotionCell {
    if weight < DIM_CEILING {
        MotionCell::Dim
    } else if weight <= PLAIN_CEILING {
        MotionCell::Plain
    } else {
        MotionCell::Bold
    }
}

/// The inputs that decide whether the sweep animates.
///
/// The sweep stops, and the word renders plain, under any one stop condition. Each
/// field is one condition from `SPEC-tui-experience` section 8.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MotionInputs {
    /// The resolved motion choice: the `tui-motion` key, or `--no-motion`, whichever the
    /// merge kept. False stops the sweep.
    pub tui_motion: bool,
    /// Whether stdout is a terminal. A non-terminal stdout stops the sweep.
    pub stdout_is_terminal: bool,
}

impl MotionInputs {
    /// The inputs under which the sweep animates: motion on, a terminal stdout, and
    /// no reduced-motion preference.
    pub fn animating() -> Self {
        Self {
            tui_motion: true,
            stdout_is_terminal: true,
        }
    }
}

/// True when the sweep animates. False under any one stop condition.
pub fn motion_enabled(inputs: MotionInputs) -> bool {
    inputs.tui_motion && inputs.stdout_is_terminal
}

/// The rendered cells of the working word at one tick.
///
/// One `MotionCell` per column of `word`. When motion is off, every cell is
/// `Plain`, at every tick, so the word renders as a still, plain frame.
pub fn sweep_frame(word: &str, tick: u64, inputs: &MotionInputs) -> Vec<MotionCell> {
    let columns = word.chars().count();
    if !motion_enabled(*inputs) {
        return vec![MotionCell::Plain; columns];
    }
    (0..columns)
        .map(|column| motion_cell(sweep_weight(tick, column)))
        .collect()
}
