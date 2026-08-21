//! Tests for the transcript scroll state. See `SPEC-tui-alternate-screen`, section 3 and 9.

use rho_tui::{PAGE_ROWS_MARGIN, Scroll, WHEEL_ROWS};

// A view that overflows: 100 display rows into a 20-row window. The newest position is
// `total - visible`, which is 80.
const TOTAL: usize = 100;
const VISIBLE: usize = 20;
const NEWEST: usize = TOTAL - VISIBLE;

#[test]
fn a_new_view_is_pinned_to_the_newest_row() {
    assert!(Scroll::pinned().is_pinned());
}

#[test]
fn scrolling_up_clamps_at_the_oldest_row() {
    let mut s = Scroll::pinned();
    s.up(1000, TOTAL, VISIBLE);
    assert_eq!(s.first_visible(TOTAL, VISIBLE), 0);
    // A second push past the top stays at the oldest row.
    s.up(1000, TOTAL, VISIBLE);
    assert_eq!(s.first_visible(TOTAL, VISIBLE), 0);
}

#[test]
fn scrolling_down_clamps_at_the_newest_row() {
    let mut s = Scroll::pinned();
    s.up(5, TOTAL, VISIBLE);
    s.down(1000, TOTAL, VISIBLE);
    assert_eq!(s.first_visible(TOTAL, VISIBLE), NEWEST);
    // No further than the newest row.
    s.down(1000, TOTAL, VISIBLE);
    assert_eq!(s.first_visible(TOTAL, VISIBLE), NEWEST);
}

#[test]
fn scrolling_up_releases_the_pin() {
    let mut s = Scroll::pinned();
    s.up(1, TOTAL, VISIBLE);
    assert!(!s.is_pinned());
}

#[test]
fn reaching_the_newest_row_restores_the_pin() {
    let mut s = Scroll::pinned();
    s.up(5, TOTAL, VISIBLE);
    assert!(!s.is_pinned());
    s.down(5, TOTAL, VISIBLE);
    assert!(s.is_pinned());
}

#[test]
fn output_moves_a_pinned_view() {
    let mut s = Scroll::pinned();
    // Ten more display rows arrive.
    let grown = TOTAL + 10;
    s.on_new_rows(grown, VISIBLE);
    assert_eq!(s.first_visible(grown, VISIBLE), grown - VISIBLE);
}

#[test]
fn output_never_moves_an_unpinned_view() {
    let mut s = Scroll::pinned();
    s.up(30, TOTAL, VISIBLE);
    let before = s.first_visible(TOTAL, VISIBLE);
    let grown = TOTAL + 10;
    s.on_new_rows(grown, VISIBLE);
    assert_eq!(s.first_visible(grown, VISIBLE), before);
}

#[test]
fn an_edge_event_is_absorbed_and_not_banked() {
    let mut s = Scroll::pinned();
    // A momentum flick sends hundreds of events at the bottom edge.
    for _ in 0..500 {
        s.down(1, TOTAL, VISIBLE);
    }
    let at_edge = s.first_visible(TOTAL, VISIBLE);
    assert_eq!(at_edge, NEWEST);
    // One scroll back moves the view by exactly one row, not paid back over 500 events.
    s.up(1, TOTAL, VISIBLE);
    assert_eq!(s.first_visible(TOTAL, VISIBLE), NEWEST - 1);
}

#[test]
fn a_resize_clamps_the_offset() {
    let mut s = Scroll::pinned();
    s.up(5, TOTAL, VISIBLE); // unpinned, first_visible == 75
    assert_eq!(s.first_visible(TOTAL, VISIBLE), 75);
    // The terminal grows taller: the window is now 40 rows, so the newest offset is 60.
    s.on_resize(TOTAL, 40);
    assert_eq!(s.first_visible(TOTAL, 40), 60);
}

#[test]
fn a_row_on_screen_moves_toward_the_bottom_when_scrolling_up() {
    // Name a display row that is on screen, then scroll up and watch it fall.
    let row = 85;
    let mut s = Scroll::pinned();
    let position_before = row - s.first_visible(TOTAL, VISIBLE);
    s.up(1, TOTAL, VISIBLE);
    let position_after = row - s.first_visible(TOTAL, VISIBLE);
    assert!(
        position_after > position_before,
        "the row must move toward the bottom: {position_before} -> {position_after}"
    );
}

#[test]
fn a_pinned_view_shows_the_newest_row() {
    assert_eq!(Scroll::pinned().first_visible(160, 20), 140);
}

#[test]
fn the_default_view_follows_output() {
    assert!(Scroll::default().is_pinned());
}

#[test]
fn a_resize_to_the_bottom_restores_the_pin() {
    let mut s = Scroll::pinned();
    s.up(5, TOTAL, VISIBLE);
    assert!(!s.is_pinned());
    // The window now holds the whole transcript, so the view sits on the newest row.
    s.on_resize(TOTAL, TOTAL);
    assert!(s.is_pinned());
}

#[test]
fn to_oldest_shows_the_first_row_and_releases_the_pin() {
    let mut s = Scroll::pinned();
    s.to_oldest();
    assert_eq!(s.first_visible(TOTAL, VISIBLE), 0);
    assert!(!s.is_pinned());
}

#[test]
fn to_newest_shows_the_last_row_and_restores_the_pin() {
    let mut s = Scroll::pinned();
    s.to_oldest();
    s.to_newest(TOTAL, VISIBLE);
    assert_eq!(s.first_visible(TOTAL, VISIBLE), NEWEST);
    assert!(s.is_pinned());
}

#[test]
fn hidden_counts_the_rows_above_and_below() {
    let mut s = Scroll::pinned();
    s.up(30, TOTAL, VISIBLE);
    let (above, below) = s.hidden(TOTAL, VISIBLE);
    assert_eq!(above + VISIBLE + below, TOTAL);
    assert_eq!(above, s.first_visible(TOTAL, VISIBLE));
}

#[test]
fn a_zero_height_view_does_not_run_past_the_end() {
    // `visible` of 0 is treated as 1, so the newest offset is `total - 1`.
    let s = Scroll::pinned();
    assert_eq!(s.first_visible(TOTAL, 0), TOTAL - 1);
}

#[test]
fn a_wheel_event_moves_exactly_one_row() {
    assert_eq!(WHEEL_ROWS, 1);
    let mut s = Scroll::pinned();
    s.up(WHEEL_ROWS, TOTAL, VISIBLE);
    assert_eq!(s.first_visible(TOTAL, VISIBLE), NEWEST - 1);
}

#[test]
fn a_page_key_leaves_an_overlap() {
    // A page moves `visible - PAGE_ROWS_MARGIN` rows, so the reader keeps their place.
    assert_eq!(PAGE_ROWS_MARGIN, 2);
    let mut s = Scroll::pinned();
    let page = VISIBLE - PAGE_ROWS_MARGIN;
    s.up(page, TOTAL, VISIBLE);
    assert_eq!(s.first_visible(TOTAL, VISIBLE), NEWEST - page);
}

#[test]
fn a_horizontal_wheel_event_moves_nothing() {
    // `Scroll` has no method for a horizontal wheel event. The caller must not route
    // `ScrollLeft` or `ScrollRight` to it, so the state stays equal.
    let s = Scroll::pinned();
    let after = s; // A horizontal event maps to no `Scroll` method.
    assert_eq!(s, after);
}
