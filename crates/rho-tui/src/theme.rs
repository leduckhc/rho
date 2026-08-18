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

/// The 256-colour index for a role. `None` means the terminal default foreground.
pub fn role_256(_role: Role) -> Option<u8> {
    todo!("role_256 is unimplemented in the red stage")
}

/// The 16-colour style for a role.
pub fn role_16(_role: Role) -> RoleStyle {
    todo!("role_16 is unimplemented in the red stage")
}

/// The no-colour style for a role. Modifiers carry every meaning.
pub fn role_none(_role: Role) -> RoleStyle {
    todo!("role_none is unimplemented in the red stage")
}
