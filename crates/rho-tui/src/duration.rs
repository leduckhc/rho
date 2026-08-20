//! The duration ladder and the duration slot.
//!
//! The format is Makit's ladder, taken exactly, with its carry defect class. The
//! rounding happens once, at the top, then integer arithmetic drives every tier.
//! No render reads a clock. A test passes milliseconds as data. See
//! `SPEC-tui-experience` sections 2 to 4.

/// The reserved width of every duration slot, in columns.
pub const DURATION_SLOT_COLUMNS: usize = 7;

/// Milliseconds in one second.
const MILLIS_PER_SECOND: i64 = 1_000;
/// Milliseconds in one tenth of a second.
const MILLIS_PER_DECISECOND: i64 = 100;
/// Seconds in one minute.
const SECONDS_PER_MINUTE: i64 = 60;
/// Seconds in one hour.
const SECONDS_PER_HOUR: i64 = 3_600;
/// Seconds in one day.
const SECONDS_PER_DAY: i64 = 86_400;
/// Tenths of a second in one whole second.
const DECISECONDS_PER_SECOND: i64 = 10;
/// The largest span, in milliseconds, that still shows one decimal of a second.
/// A span of `9.95s` or more rounds to a whole second, so it leaves this tier.
const DECIMAL_TIER_MAX_MILLIS: i64 = 9_950;
/// Milliseconds past which a live duration renders in `warn`, which is one minute.
const AMBER_THRESHOLD_MILLIS: i64 = 60_000;

/// Format a span on Makit's duration ladder.
///
/// `span_millis` is the time between a start event and its end event.
/// `None` means the span is open, because no end event has arrived yet.
/// A `Some` value below zero means the clock stepped back, so `end < start`.
/// Round exactly once here at the top, then use integer arithmetic for every tier.
/// Return `None` for an unrepresentable span, so the caller draws an empty slot.
pub fn format_duration(span_millis: Option<i64>) -> Option<String> {
    // An open span (`None`) or a backward step (negative) is unrepresentable.
    let millis = match span_millis {
        Some(millis) if millis >= 0 => millis,
        _ => return None,
    };

    // The sub-ten-second tier is the only tier that shows a fraction. Round once, to
    // tenths of a second, then read the whole and the fractional part off that value.
    if millis < DECIMAL_TIER_MAX_MILLIS {
        let deciseconds = round_div(millis, MILLIS_PER_DECISECOND);
        let whole = deciseconds / DECISECONDS_PER_SECOND;
        let tenths = deciseconds % DECISECONDS_PER_SECOND;
        return Some(if tenths == 0 {
            format!("{whole}s")
        } else {
            format!("{whole}.{tenths}s")
        });
    }

    // Every tier at or above ten seconds rounds once, to whole seconds. Coarser units
    // come from integer division of that one rounded value, so a carry cannot escape a
    // tier: `59.5s` rounds to `60s`, which reads as `1m 00s`, never `60s`.
    let seconds = round_div(millis, MILLIS_PER_SECOND);

    Some(if seconds < SECONDS_PER_MINUTE {
        format!("{seconds}s")
    } else if seconds < SECONDS_PER_HOUR {
        let minutes = seconds / SECONDS_PER_MINUTE;
        let rest = seconds % SECONDS_PER_MINUTE;
        format!("{minutes}m {rest:02}s")
    } else if seconds < SECONDS_PER_DAY {
        let hours = seconds / SECONDS_PER_HOUR;
        let minutes = (seconds % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE;
        format!("{hours}h {minutes:02}m")
    } else {
        let days = seconds / SECONDS_PER_DAY;
        let hours = (seconds % SECONDS_PER_DAY) / SECONDS_PER_HOUR;
        format!("{days}d {hours}h")
    })
}

/// Divide and round half away from zero. The input is non-negative here, so this is
/// round half up. This is the single rounding step for a whole tier.
fn round_div(numerator: i64, denominator: i64) -> i64 {
    (numerator + denominator / 2) / denominator
}

/// Render a duration into its fixed seven-column slot, right aligned.
///
/// An unrepresentable span renders seven blank columns, never a zero. A fixed slot
/// means a live tick never reflows the text beside it.
pub fn duration_slot(span_millis: Option<i64>) -> String {
    match format_duration(span_millis) {
        Some(text) => format!("{text:>DURATION_SLOT_COLUMNS$}"),
        None => " ".repeat(DURATION_SLOT_COLUMNS),
    }
}

/// True when a live duration renders in `warn`, because it passed one minute.
///
/// The amber cue is Makit's escalation. It applies only while the span is live, and
/// it returns to the normal role when the span completes.
pub fn live_duration_is_amber(span_millis: i64) -> bool {
    span_millis > AMBER_THRESHOLD_MILLIS
}
