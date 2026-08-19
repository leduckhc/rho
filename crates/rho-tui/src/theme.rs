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
    /// An inline code span. Measured against pi, which gives it RGB 138,190,183, and jcode,
    /// which adds a background. rho takes the foreground only, because `RoleStyle` has no
    /// background field and adding one is a contract change. See `SPEC-tui-markdown` 3a item 6.
    MdCode,
}

impl Role {
    /// Every role, so a test proves each one has all three mappings.
    pub const ALL: [Role; 11] = [
        Role::Text,
        Role::Muted,
        Role::Accent,
        Role::Error,
        Role::Warn,
        Role::Caution,
        Role::MdHeading,
        Role::MdCodeBlock,
        Role::MdCode,
        Role::MdBold,
        Role::MdItalic,
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
    pub reversed: bool,
}

impl RoleStyle {
    /// A plain style: the terminal default colour, no modifier.
    const fn plain() -> Self {
        Self {
            color: Ansi16::Default,
            dim: false,
            bold: false,
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
        Role::MdCodeBlock => Some(110),
        Role::MdCode => Some(115),
        // Bold reads brighter than body text. Italic reads warmer, so the two never blur.
        Role::MdBold => Some(231),
        Role::MdItalic => Some(180),
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
        Role::MdCodeBlock | Role::MdCode => RoleStyle {
            color: Ansi16::Cyan,
            ..RoleStyle::plain()
        },
        Role::MdBold => RoleStyle {
            bold: true,
            ..RoleStyle::plain()
        },
        Role::MdItalic => RoleStyle {
            color: Ansi16::Yellow,
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
        Role::MdCodeBlock | Role::MdCode => RoleStyle {
            dim: true,
            ..RoleStyle::plain()
        },
        Role::MdBold => RoleStyle {
            bold: true,
            ..RoleStyle::plain()
        },
        Role::MdItalic => RoleStyle {
            reversed: true,
            ..RoleStyle::plain()
        },
    }
}
