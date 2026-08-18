//! Duration ladder and slot tests. See `SPEC-tui-experience` sections 2 to 4.
//!
//! A duration test passes milliseconds as data. No test reads a clock, and no test
//! sleeps. The ladder arrives with a proven carry defect class, so the four carry
//! cases come first.

use rho_tui::{DURATION_SLOT_COLUMNS, duration_slot, format_duration, live_duration_is_amber};

// --- The four carry cases, first, because every documented rung passes with the bug.

#[test]
fn duration_carry_59_5s_is_1m_00s() {
    assert_eq!(format_duration(Some(59_500)), Some("1m 00s".to_string()));
}

#[test]
fn duration_carry_119_7s_is_2m_00s() {
    assert_eq!(format_duration(Some(119_700)), Some("2m 00s".to_string()));
}

#[test]
fn duration_carry_3599_7s_is_1h_00m() {
    assert_eq!(format_duration(Some(3_599_700)), Some("1h 00m".to_string()));
}

#[test]
fn duration_carry_9_96s_is_10s() {
    assert_eq!(format_duration(Some(9_960)), Some("10s".to_string()));
}

// --- The documented rungs.

#[test]
fn duration_one_decimal_under_ten_seconds() {
    assert_eq!(format_duration(Some(2_400)), Some("2.4s".to_string()));
}

#[test]
fn duration_strips_trailing_zero() {
    assert_eq!(format_duration(Some(2_000)), Some("2s".to_string()));
}

#[test]
fn duration_whole_seconds_to_59() {
    assert_eq!(format_duration(Some(13_000)), Some("13s".to_string()));
    assert_eq!(format_duration(Some(59_000)), Some("59s".to_string()));
}

#[test]
fn duration_zero_padded_seconds_under_an_hour() {
    assert_eq!(format_duration(Some(161_000)), Some("2m 41s".to_string()));
}

#[test]
fn duration_pads_the_seconds_field() {
    assert_eq!(
        format_duration(Some(1_084_000)),
        Some("18m 04s".to_string())
    );
}

#[test]
fn duration_zero_padded_minutes_under_a_day() {
    assert_eq!(
        format_duration(Some(15_120_000)),
        Some("4h 12m".to_string())
    );
}

#[test]
fn duration_pads_the_minutes_field() {
    assert_eq!(format_duration(Some(3_600_000)), Some("1h 00m".to_string()));
}

#[test]
fn duration_days_and_hours() {
    assert_eq!(
        format_duration(Some(273_600_000)),
        Some("3d 4h".to_string())
    );
}

// --- The edges and the slot.

#[test]
fn duration_empty_slot_for_open_span() {
    assert_eq!(format_duration(None), None);
}

#[test]
fn duration_empty_slot_for_negative_span() {
    assert_eq!(format_duration(Some(-5)), None);
}

#[test]
fn duration_none_renders_an_empty_slot_not_a_zero() {
    let slot = duration_slot(None);
    assert_eq!(
        slot.chars().count(),
        DURATION_SLOT_COLUMNS,
        "an open span still fills the reserved slot: {slot:?}"
    );
    assert!(
        slot.chars().all(|c| c == ' '),
        "an open span draws blank columns, never a zero: {slot:?}"
    );
}

#[test]
fn duration_slot_is_seven_columns() {
    for millis in [2_400, 13_000, 161_000, 1_084_000, 15_120_000, 273_600_000] {
        let slot = duration_slot(Some(millis));
        assert_eq!(
            slot.chars().count(),
            DURATION_SLOT_COLUMNS,
            "every rung occupies seven columns, {millis}ms gave {slot:?}"
        );
    }
}

#[test]
fn duration_slot_right_aligns() {
    // `9.1s` is four columns, so it sits at the right edge with three leading spaces.
    assert_eq!(duration_slot(Some(9_100)), "   9.1s");
}

#[test]
fn duration_tick_growth_does_not_reflow() {
    // The text after the slot must not move as the value grows from `9.1s` to `2m 41s`.
    let short = format!("{}|tail", duration_slot(Some(9_100)));
    let long = format!("{}|tail", duration_slot(Some(161_000)));
    assert_eq!(
        short.find('|'),
        Some(DURATION_SLOT_COLUMNS),
        "the tail starts after the fixed slot: {short:?}"
    );
    assert_eq!(
        short.find('|'),
        long.find('|'),
        "a growing value moves no character after the slot: {short:?} vs {long:?}"
    );
}

#[test]
fn duration_amber_past_one_minute() {
    assert!(
        live_duration_is_amber(61_000),
        "a live duration past one minute renders in warn"
    );
    assert!(
        !live_duration_is_amber(59_000),
        "a live duration under one minute keeps its normal role"
    );
}
