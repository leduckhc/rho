//! Theme tests. A theme is a role table, never a colour list. Every role maps to
//! a 256-colour value, a 16-colour fallback, and a no-colour modifier set. See
//! `SPEC-tui-experience` section 10.

use rho_tui::{Ansi16, Role, role_16, role_256, role_bg_256, role_none};

#[test]
fn theme_resolves_every_role() {
    // Each role maps to a 256-colour, a 16-colour, and a no-colour style. A missing
    // mapping fails here, because each resolver runs for each role. It never falls
    // back silently.
    for role in Role::ALL {
        // A `None` from `role_256` is a valid mapping: the terminal default
        // foreground. The call resolving without a panic is the mapping.
        let _ = role_256(role);
        let _ = role_bg_256(role);
        let _ = role_16(role);
        let _ = role_none(role);
    }
}

#[test]
fn no_colour_mode_uses_no_raw_colour() {
    // No role resolves to a raw colour in the no-colour mode. Every meaning rides on
    // a modifier, so a monochrome terminal loses nothing. Not a spec-named test; the
    // stage task requires this assertion.
    for role in Role::ALL {
        let style = role_none(role);
        assert_eq!(
            style.color,
            Ansi16::Default,
            "role {role:?} resolved to a raw colour in the no-colour mode"
        );
    }
}

#[test]
fn only_the_user_band_paints_a_background() {
    // A background is loud, so it is spent once. Every other role keeps the terminal's own, or the
    // screen becomes a patchwork and the band marks nothing.
    for role in Role::ALL {
        let bg = role_bg_256(role);
        if role == Role::UserBand {
            assert!(bg.is_some(), "the user band needs a background");
        } else {
            assert!(
                bg.is_none(),
                "role {role:?} must not paint a background: {bg:?}"
            );
        }
    }
}
