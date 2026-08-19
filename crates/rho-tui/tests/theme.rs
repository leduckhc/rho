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

#[test]
fn emphasis_never_borrows_a_status_colour() {
    // A design review measured italic 180 at 1.02x against warn 179 and 1.38x against caution 173,
    // all three in the same amber family. So emphasis inside an answer read as a warning, on a screen
    // that already carries amber notices. Italic is now a lavender: a hue shift, not a luminance one.
    let italic = role_256(Role::MdItalic).expect("italic has a colour");
    for status in [Role::Warn, Role::Caution, Role::Error] {
        let other = role_256(status).expect("a status colour");
        assert_ne!(
            italic, other,
            "italic must not share {status:?}'s colour index"
        );
    }
    // And it must not sit in the amber ramp at all: red and green high, blue low.
    let (r, g, b) = xterm_rgb(italic);
    assert!(
        b >= g,
        "italic must not be an amber: index {italic} is rgb({r},{g},{b})"
    );
}

#[test]
fn one_colour_means_code() {
    // Inline code used index 115 and a heading uses 78. Measured contrast between them: 1.07x, the
    // same colour to the eye and identical to a deuteranope. Code now takes one colour, inline or in
    // a block, which also spends one hue instead of two.
    let code = role_256(Role::MdCodeBlock).expect("code has a colour");
    let heading = role_256(Role::MdHeading).expect("a heading has a colour");
    assert_ne!(code, heading, "code and a heading are different colours");
    let (cr, cg, cb) = xterm_rgb(code);
    let (hr, hg, hb) = xterm_rgb(heading);
    // Code is blue-leaning and a heading is green-leaning, so the two differ in hue and not only in
    // brightness. Two greens of similar luminance are what the review caught.
    assert!(cb > cg, "code leans blue: rgb({cr},{cg},{cb})");
    assert!(hg > hb, "a heading leans green: rgb({hr},{hg},{hb})");
}

/// The rgb an xterm 256-colour index resolves to, for a contrast assertion.
fn xterm_rgb(index: u8) -> (u32, u32, u32) {
    const LEVELS: [u32; 6] = [0, 95, 135, 175, 215, 255];
    if index >= 232 {
        let grey = 8 + (u32::from(index) - 232) * 10;
        return (grey, grey, grey);
    }
    let i = u32::from(index) - 16;
    (
        LEVELS[(i / 36) as usize],
        LEVELS[((i / 6) % 6) as usize],
        LEVELS[(i % 6) as usize],
    )
}
