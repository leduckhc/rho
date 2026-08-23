//! The screen layout, and the small-terminal path.
//!
//! `plan_screen` is public and had no direct test. `STARTUP_MIN_ROWS` and
//! `TuiError::TooSmall` had none either. Every defect in this project that reached a user
//! sat in public surface no test touched, so this file closes that gap for the layout.
//!
//! These tests assert bounds and totals, not one expected frame. A frame test already
//! exists in `frames.rs`. What was missing is the arithmetic underneath it, at the sizes a
//! fixture never covers.
//!
//! `SPEC-tui-alternate-screen` sections 6 and 7 name these tests.

use rho_tui::{STARTUP_MIN_ROWS, ScreenLayout, TuiError, plan_screen};

/// The rows every region takes, summed. It must equal the terminal height exactly: a row
/// unaccounted for is a row that draws twice or not at all.
fn rows_used(layout: &ScreenLayout) -> usize {
    let rules = if layout.composer_border { 2 } else { 0 };
    usize::from(layout.banner)
        + layout.transcript_rows
        + layout.panel_rows
        + layout.composer_rows
        + rules
        + usize::from(layout.footer)
}

// ---- Section 7: a terminal too short. ------------------------------------------

#[test]
fn a_terminal_too_short_reports_and_does_not_draw() {
    // A two-row terminal cannot hold the composer and the footer. It reports, and it draws
    // no panel and no banner. Verified live too: the release binary exits 1 and says
    // "rho: the terminal is 2 rows, and rho needs at least 4".
    let layout = plan_screen(2, 1, 0, 0);
    assert!(layout.too_small, "two rows is below the minimum");
    assert_eq!(layout.panel_rows, 0, "a small screen draws no panel");
    assert!(!layout.banner, "a small screen draws no banner");
    assert!(!layout.composer_border, "a small screen draws no rules");
    assert_eq!(
        layout.composer_rows, 1,
        "the draft row is kept first, because the user owns it"
    );
    assert_eq!(rows_used(&layout), 2, "every row is accounted for");
}

#[test]
fn the_too_small_boundary_is_exactly_the_startup_minimum() {
    // The boundary, not one example. An off-by-one here either kills a session that could
    // have drawn, or draws a broken screen that should have reported.
    assert_eq!(
        STARTUP_MIN_ROWS, 4,
        "three composer rows and one footer row"
    );
    for height in 0..STARTUP_MIN_ROWS {
        assert!(
            plan_screen(height, 1, 0, 0).too_small,
            "height {height} is below the minimum and must report"
        );
    }
    for height in STARTUP_MIN_ROWS..40 {
        assert!(
            !plan_screen(height, 1, 0, 0).too_small,
            "height {height} is at or above the minimum and must draw"
        );
    }
}

#[test]
fn a_small_screen_never_hides_the_draft_row() {
    // The keep order is the draft row, then the footer, then the transcript. At one row the
    // draft still wins, because a user who cannot see what they type has no interface.
    for height in 1..STARTUP_MIN_ROWS {
        let layout = plan_screen(height, 3, 4, 2);
        assert_eq!(
            layout.composer_rows, 1,
            "at {height} rows the draft row is kept"
        );
        assert_eq!(layout.panel_rows, 0, "at {height} rows no panel draws");
        assert_eq!(rows_used(&layout), height as usize, "rows add up");
    }
}

#[test]
fn a_zero_row_terminal_plans_nothing_and_does_not_panic() {
    // A zero height reaches here during a resize storm. Arithmetic on it must not underflow.
    let layout = plan_screen(0, 1, 0, 0);
    assert!(layout.too_small);
    assert_eq!(rows_used(&layout), 0);
}

#[test]
fn the_too_small_error_states_both_numbers() {
    // The message has to say what the terminal is and what rho needs. A bare "too small"
    // leaves the user guessing how far to drag.
    let error = TuiError::TooSmall {
        rows: 2,
        need: STARTUP_MIN_ROWS,
    };
    let text = error.to_string();
    assert!(text.contains('2'), "the message states the size: {text}");
    assert!(text.contains('4'), "and the requirement: {text}");
}

// ---- Section 6: the layout. -----------------------------------------------------

#[test]
fn the_composer_keeps_its_ten_row_cap() {
    // From `D-ledger-wins-the-band`. A paste of 500 lines must not take the screen.
    for draft in [10usize, 11, 50, 500] {
        let layout = plan_screen(40, draft, 0, 0);
        assert_eq!(
            layout.composer_rows, 10,
            "a {draft} row draft caps at ten rows"
        );
    }
    // And it does not cap below its request when the request is small.
    for draft in 1..=9usize {
        let layout = plan_screen(40, draft, 0, 0);
        assert_eq!(
            layout.composer_rows, draft,
            "a {draft} row draft gets {draft} rows"
        );
    }
}

#[test]
fn the_transcript_takes_the_rows_the_composer_leaves() {
    // The transcript is the remainder, so the total must always close. This sweep is the
    // guard: a region that grows without taking from the transcript silently overdraws.
    for height in STARTUP_MIN_ROWS..60 {
        for draft in [1usize, 3, 10, 40] {
            for (want, floor) in [(0usize, 0usize), (4, 2), (12, 2), (30, 30)] {
                let layout = plan_screen(height, draft, want, floor);
                assert_eq!(
                    rows_used(&layout),
                    height as usize,
                    "height {height}, draft {draft}, panel want {want} floor {floor}: \
                     the regions must sum to the height"
                );
            }
        }
    }
}

#[test]
fn the_transcript_shrinks_when_the_composer_grows() {
    // The direction of the trade, stated. A taller draft must cost the transcript, never
    // the footer and never the draft cap.
    let short = plan_screen(30, 1, 0, 0);
    let tall = plan_screen(30, 8, 0, 0);
    assert!(
        tall.transcript_rows < short.transcript_rows,
        "a taller draft costs transcript rows: {} then {}",
        short.transcript_rows,
        tall.transcript_rows
    );
    assert!(short.footer && tall.footer, "the footer never yields");
}

#[test]
fn a_panel_floor_survives_a_tall_draft() {
    // From `D-ledger-wins-the-band`. An approval states a destructive command, so it keeps
    // its floor even under a full ten-row draft. `an_approval_states_its_session_root`
    // covers the content; this covers the arithmetic that leaves room for it.
    //
    // This test was vacuous when first written. It asked for 20 rows, where the panel gets
    // its whole want of 6 and the floor never decides anything, so it passed against a
    // build with the floor deleted. The floor only binds in the tight band, where the
    // ten-row draft would otherwise squeeze the panel out. That band is what it checks now.
    for height in 8..=16u16 {
        let layout = plan_screen(height, 10, 6, 3);
        assert_eq!(
            layout.panel_rows, 3,
            "at {height} rows the panel holds exactly its floor under a ten-row draft"
        );
        assert_eq!(rows_used(&layout), height as usize, "rows add up");
    }
    // Below the band the floor yields, because the composer minimum outranks it.
    let cramped = plan_screen(6, 10, 6, 3);
    assert!(
        cramped.panel_rows <= 2,
        "a floor cannot outrank the composer minimum, got {}",
        cramped.panel_rows
    );
    // Above the band the panel gets its whole want, so the floor stops deciding.
    assert_eq!(plan_screen(21, 10, 6, 3).panel_rows, 6);
}

#[test]
fn the_banner_yields_before_the_transcript_starves() {
    // The banner is dropped first when the screen is tight, by design. At the minimum
    // height there is no room for it.
    let tight = plan_screen(STARTUP_MIN_ROWS, 1, 0, 0);
    assert!(!tight.banner, "no banner at the minimum height");
    let roomy = plan_screen(30, 1, 0, 0);
    assert!(roomy.banner, "a roomy screen shows the banner");
}
