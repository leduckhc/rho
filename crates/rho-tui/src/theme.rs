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
}

impl Role {
    /// Every role, so a test proves each one has all three mappings.
    pub const ALL: [Role; 6] = [
        Role::Text,
        Role::Muted,
        Role::Accent,
        Role::Error,
        Role::Warn,
        Role::Caution,
    ];
}

/// A 16-colour terminal colour, before any modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ansi16 {
    Default,
    Green,
    Red,
    Yellow,
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
    }
}
