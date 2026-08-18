//! The duration ladder and the duration slot.
//!
//! The format is Makit's ladder, taken exactly, with its carry defect class. The
//! rounding happens once, at the top, then integer arithmetic drives every tier.
//! No render reads a clock. A test passes milliseconds as data. See
//! `SPEC-tui-experience` sections 2 to 4.

/// The reserved width of every duration slot, in columns.
pub const DURATION_SLOT_COLUMNS: usize = 7;

/// Format a span on Makit's duration ladder.
///
/// `span_millis` is the time between a start event and its end event.
/// `None` means the span is open, because no end event has arrived yet.
/// A `Some` value below zero means the clock stepped back, so `end < start`.
/// Round exactly once here at the top, then use integer arithmetic for every tier.
/// Return `None` for an unrepresentable span, so the caller draws an empty slot.
pub fn format_duration(span_millis: Option<i64>) -> Option<String> {
    let _ = span_millis;
    todo!("format_duration is unimplemented in the red stage")
}

/// Render a duration into its fixed seven-column slot, right aligned.
///
/// An unrepresentable span renders seven blank columns, never a zero. A fixed slot
/// means a live tick never reflows the text beside it.
pub fn duration_slot(span_millis: Option<i64>) -> String {
    let _ = span_millis;
    todo!("duration_slot is unimplemented in the red stage")
}

/// True when a live duration renders in `warn`, because it passed one minute.
///
/// The amber cue is Makit's escalation. It applies only while the span is live, and
/// it returns to the normal role when the span completes.
pub fn live_duration_is_amber(span_millis: i64) -> bool {
    let _ = span_millis;
    todo!("live_duration_is_amber is unimplemented in the red stage")
}
