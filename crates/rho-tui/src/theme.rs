//! The theme, as a role table.
//!
//! A theme is a role table, never a colour list. A role maps to a 256-colour
//! value, a 16-colour fallback, and a no-colour modifier set. No role resolves to
//! a raw colour in the no-colour mode. Every role has all three mappings, and a
//! missing mapping is a defect, never a silent fallback. See
//! `SPEC-tui-experience` section 10 and `docs/tui-design.md` section 3.

/// A colour role. A theme is a role table, never a colour list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Text,
    Muted,
    Accent,
    Error,
    Warn,
    Caution,
    /// A markdown heading. Bold, and the accent colour, so a section stands out. It is its
    /// own role only because no existing role carries the bold weight with a colour.
    MdHeading,
    /// A line inside a code block. Its own colour, because no existing role reads as "not
    /// prose". A fence, a quote, and a bullet reuse `Muted` and `Accent`.
    MdCodeBlock,
    /// Inline bold. Bold also takes a colour, because a terminal may draw no distinct bold
    /// weight and the emphasis would then vanish. Used only inside body text: a heading keeps
    /// its own colour, so emphasis there is the modifier alone.
    MdBold,
    /// Inline italic. Coloured for the same reason, and more urgently: many terminals draw no
    /// italic at all.
    MdItalic,
    /// A submitted prompt. The only role that paints a background: the band marks where a turn
    /// began, and both Claude Code and pi mark it the same way. A background is loud, so it is
    /// spent once. See `D-a-submitted-prompt-sits-on-a-band`.
    UserBand,
    /// A quoted line and its bar. Quiet and leaning: a quote is someone else's voice. It is its
    /// own role again because no existing role carries dim with a lean.
    MdQuote,
}

impl Role {
    /// Every role, so a test proves each one has all three mappings.
    pub const ALL: [Role; 12] = [
        Role::Text,
        Role::Muted,
        Role::Accent,
        Role::Error,
        Role::Warn,
        Role::Caution,
        Role::MdHeading,
        Role::MdCodeBlock,
        Role::MdBold,
        Role::MdItalic,
        Role::MdQuote,
        Role::UserBand,
    ];
}

/// A 16-colour terminal colour, before any modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ansi16 {
    Default,
    Green,
    Red,
    Yellow,
    Cyan,
}

/// One resolved role style, colour plus modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoleStyle {
    pub color: Ansi16,
    pub dim: bool,
    pub bold: bool,
    /// A leaning weight. Added for the quote and the inline italic role: both mean "this is not
    /// the answer's own voice", and no colour says that on its own.
    pub italic: bool,
    pub reversed: bool,
}

impl RoleStyle {
    /// A plain style: the terminal default colour, no modifier.
    const fn plain() -> Self {
        Self {
            color: Ansi16::Default,
            dim: false,
            bold: false,
            italic: false,
            reversed: false,
        }
    }
}

/// The 256-colour index for a role. `None` means the terminal default foreground.
///
/// The match is exhaustive over `Role`, so a new role without a mapping fails to
/// compile rather than falling back silently.
pub fn role_256(role: Role) -> Option<u8> {
    match role {
        Role::Text => None,
        Role::Muted => Some(245),
        Role::Accent => Some(78),
        Role::Error => Some(203),
        Role::Warn => Some(179),
        Role::Caution => Some(173),
        // The markdown roles. A heading borrows the accent, so the interface keeps one
        // family. Code is a cool blue, which reads as "not prose" beside the warm accent.
        // A fence and a bullet stay quiet, because both are punctuation.
        Role::MdHeading => Some(78),
        // One colour means code, inline or in a block. Index 115 used to mark an inline span and
        // a design review measured it at 1.07x against the heading green 78: the same colour to the
        // eye, and identical to a deuteranope. Unifying on the blue 110 also spends one hue instead
        // of two. See `D-one-hue-means-code`.
        Role::MdCodeBlock => Some(110),
        // Bold reads brighter than body text. On a terminal whose default foreground is already
        // pure white the colour adds nothing and the weight carries it; measured at 1.16x against
        // #eeeeee and 1.82x against #c0c0c0.
        Role::MdBold => Some(231),
        // Italic is a lavender, deliberately outside the amber status family. Index 180 was measured
        // at 1.02x against warn 179 and 1.38x against caution 173, so emphasis inside an answer read
        // as a warning. 146 shifts the hue rather than the luminance. See `D-one-hue-means-code`.
        Role::MdItalic => Some(146),
        Role::MdQuote => Some(245),
        // The band keeps the body foreground: only the background changes, so the text reads the
        // same as the answer below it.
        Role::UserBand => None,
    }
}

/// The 256-colour **background** for a role, or `None` for the terminal's own.
///
/// A background is a fourth mapping and not a field on `RoleStyle`, because only this mode can
/// carry a quiet one. A 16-colour terminal has no subtle grey, and the no-colour mode has no
/// colour at all, so both mark a submitted prompt with a modifier instead.
///
/// **Exactly one role paints a background.** A background is loud, and spending it more than once
/// turns the screen into a patchwork that marks nothing. `only_the_user_band_paints_a_background`
/// holds that.
pub fn role_bg_256(role: Role) -> Option<u8> {
    match role {
        // Two steps above the usual terminal black: enough to read as a band, not enough to fight
        // the text on it.
        Role::UserBand => Some(236),
        Role::Text
        | Role::Muted
        | Role::Accent
        | Role::Error
        | Role::Warn
        | Role::Caution
        | Role::MdHeading
        | Role::MdCodeBlock
        | Role::MdBold
        | Role::MdItalic
        | Role::MdQuote => None,
    }
}

/// The 16-colour style for a role. `warn` and `caution` share yellow; the bold weight
/// keeps them apart. The match is exhaustive, so no role resolves by silent fallback.
pub fn role_16(role: Role) -> RoleStyle {
    match role {
        Role::Text => RoleStyle::plain(),
        Role::Muted => RoleStyle {
            color: Ansi16::Default,
            dim: true,
            ..RoleStyle::plain()
        },
        Role::Accent => RoleStyle {
            color: Ansi16::Green,
            ..RoleStyle::plain()
        },
        Role::Error => RoleStyle {
            color: Ansi16::Red,
            ..RoleStyle::plain()
        },
        Role::Warn => RoleStyle {
            color: Ansi16::Yellow,
            ..RoleStyle::plain()
        },
        Role::Caution => RoleStyle {
            color: Ansi16::Yellow,
            bold: true,
            ..RoleStyle::plain()
        },
        Role::MdHeading => RoleStyle {
            color: Ansi16::Green,
            bold: true,
            ..RoleStyle::plain()
        },
        Role::MdCodeBlock => RoleStyle {
            color: Ansi16::Cyan,
            ..RoleStyle::plain()
        },
        Role::MdBold => RoleStyle {
            bold: true,
            ..RoleStyle::plain()
        },
        // Not yellow. Yellow is `Warn`, and sixteen colours have no lavender, so the lean carries
        // the meaning here on its own.
        Role::MdItalic => RoleStyle {
            italic: true,
            ..RoleStyle::plain()
        },
        Role::MdQuote => RoleStyle {
            dim: true,
            italic: true,
            ..RoleStyle::plain()
        },
        // A submitted prompt is bold in **every** mode, not only as a fallback here. `style_for`
        // reads modifiers from this table whatever the colour depth, so the weight applies beside
        // the band as well as instead of it. That is intended: the words are the user's own, and a
        // 16-colour terminal has no subtle grey to band with.
        Role::UserBand => RoleStyle {
            bold: true,
            ..RoleStyle::plain()
        },
    }
}

/// The no-colour style for a role. Modifiers carry every meaning, so no role resolves
/// to a raw colour: every style keeps the terminal default colour. The match is
/// exhaustive, so a missing mapping fails rather than falling back silently.
pub fn role_none(role: Role) -> RoleStyle {
    match role {
        Role::Text => RoleStyle::plain(),
        Role::Muted => RoleStyle {
            dim: true,
            ..RoleStyle::plain()
        },
        Role::Accent | Role::Error | Role::Warn => RoleStyle {
            bold: true,
            ..RoleStyle::plain()
        },
        Role::Caution => RoleStyle {
            bold: true,
            reversed: true,
            ..RoleStyle::plain()
        },
        // No colour, so a modifier carries the meaning. A heading is bold. Code and a quote
        // are dim, which separates them from prose without a colour.
        Role::MdHeading => RoleStyle {
            bold: true,
            ..RoleStyle::plain()
        },
        Role::MdCodeBlock => RoleStyle {
            dim: true,
            ..RoleStyle::plain()
        },
        Role::MdBold => RoleStyle {
            bold: true,
            ..RoleStyle::plain()
        },
        // With no colour the lean is the whole signal, which is why the field exists.
        Role::MdItalic => RoleStyle {
            italic: true,
            ..RoleStyle::plain()
        },
        Role::MdQuote => RoleStyle {
            dim: true,
            italic: true,
            ..RoleStyle::plain()
        },
        // With no colour, reversing the row is the band.
        Role::UserBand => RoleStyle {
            reversed: true,
            ..RoleStyle::plain()
        },
    }
}
