//! Motion tests. The frame is a pure function of a tick. No test reads a clock,
//! and no test sleeps. See `SPEC-tui-experience` section 8.

use rho_tui::{
    MotionCell, MotionInputs, SWEEP_PERIOD_TICKS, motion_cell, sweep_frame, sweep_weight,
};

/// The word that the sweep animates in the footer.
const WORD: &str = "working";

#[test]
fn motion_is_a_function_of_a_tick() {
    // The same tick must produce the same frame, twice, with no clock involved.
    let inputs = MotionInputs::animating();
    let first = sweep_frame(WORD, 7, &inputs);
    let second = sweep_frame(WORD, 7, &inputs);
    assert_eq!(first, second, "the same tick produced two different frames");
}

#[test]
fn motion_period_is_twenty_ticks() {
    // The band returns to its start after one period, so the sweep is continuous
    // across the wrap. `tick` and `tick + 20` yield the same weight in every column.
    for tick in 0..SWEEP_PERIOD_TICKS {
        for column in 0..40 {
            let here = sweep_weight(tick, column);
            let wrapped = sweep_weight(tick + SWEEP_PERIOD_TICKS, column);
            assert_eq!(
                here,
                wrapped,
                "tick {tick} and tick {} differ at column {column}",
                tick + SWEEP_PERIOD_TICKS
            );
        }
    }
}

#[test]
fn sweep_weight_follows_the_raised_cosine_band() {
    // The weight follows the raised-cosine band, at the stated half width of five
    // columns with ten columns of padding at each end. Not a spec-named test; the
    // stage task requires this assertion, and the spec names no test for it.
    let mut saw_peak = false;
    for tick in 0..SWEEP_PERIOD_TICKS {
        let mut nonzero = 0usize;
        for column in 0..60 {
            let weight = sweep_weight(tick, column);
            assert!(
                (0.0..=1.0).contains(&weight),
                "weight {weight} out of range at tick {tick}, column {column}"
            );
            if weight > 0.01 {
                nonzero += 1;
            }
            if weight >= 0.99 {
                saw_peak = true;
            }
        }
        // The band half width is five columns, so its footprint spans at most
        // eleven columns: five each side of the peak, plus the peak.
        assert!(
            nonzero <= 11,
            "the band spread over {nonzero} columns at tick {tick}, wider than a half width of five"
        );
    }
    assert!(
        saw_peak,
        "the raised-cosine band never reached its peak weight"
    );
}

#[test]
fn motion_off_when_tui_motion_false() {
    // `tui.motion = false` renders the word plain, at every tick.
    let inputs = MotionInputs {
        tui_motion: false,
        ..MotionInputs::animating()
    };
    assert_still_and_plain(&inputs);
}

#[test]
fn motion_off_when_no_motion_flag() {
    // `--no-motion` renders the word plain, at every tick.
    let inputs = MotionInputs {
        no_motion_flag: true,
        ..MotionInputs::animating()
    };
    assert_still_and_plain(&inputs);
}

#[test]
fn motion_off_when_stdout_not_a_terminal() {
    // A non-terminal stdout renders the word plain, at every tick.
    let inputs = MotionInputs {
        stdout_is_terminal: false,
        ..MotionInputs::animating()
    };
    assert_still_and_plain(&inputs);
}

#[test]
fn motion_off_when_reduce_motion_setting() {
    // `tui.reduce_motion = true` renders the word plain, at every tick.
    let inputs = MotionInputs {
        reduce_motion_setting: true,
        ..MotionInputs::animating()
    };
    assert_still_and_plain(&inputs);
}

#[test]
fn motion_off_when_reduce_motion_env() {
    // `RHO_REDUCE_MOTION=1` renders the word plain, at every tick.
    let inputs = MotionInputs {
        reduce_motion_env: true,
        ..MotionInputs::animating()
    };
    assert_still_and_plain(&inputs);
}

#[test]
fn render_never_reads_a_clock() {
    // The frame is a pure function of state. With every input fixed, the frame
    // never varies, so no ambient clock feeds it. The only time source is the tick.
    let inputs = MotionInputs::animating();
    let once = sweep_frame(WORD, 4, &inputs);
    let twice = sweep_frame(WORD, 4, &inputs);
    assert_eq!(
        once, twice,
        "the frame changed with no input change, so it read a clock"
    );
    // The no-colour tiers are a pure function of a weight too.
    assert_eq!(motion_cell(1.0), motion_cell(1.0));
}

/// Assert that motion off yields a still, plain frame: every cell is `Plain`, and
/// the frame does not change across ticks.
fn assert_still_and_plain(inputs: &MotionInputs) {
    let frame_a = sweep_frame(WORD, 0, inputs);
    let frame_b = sweep_frame(WORD, 9, inputs);
    assert!(
        frame_a.iter().all(|cell| *cell == MotionCell::Plain),
        "motion off did not render every cell plain"
    );
    assert_eq!(
        frame_a, frame_b,
        "motion off still animated across ticks; the frame must be still"
    );
}
