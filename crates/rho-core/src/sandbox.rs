//! The OS-level confinement mode for the `bash` tool.
//!
//! The type lives here, in the core, because `SessionConfig` carries it. The
//! planner that turns a mode into a real command lives in `rho-tools`, next to
//! the `bash` tool. See `SPEC-bash-sandbox`.

use std::fmt;
use std::str::FromStr;

/// The OS-level confinement mode for the `bash` tool. See `SPEC-bash-sandbox`.
///
/// `Off` is the default, and the `Default` derive makes that explicit. A sandbox
/// that breaks a build is worse than none, so the default does not change under a
/// user. See decision D-no-four-argument-session-new on stated defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SandboxMode {
    /// No confinement. Today's behaviour, and the default.
    #[default]
    Off,
    /// Writes limited to the session root and the scratch directory. Reads
    /// stay allowed, because a coding agent must read a toolchain.
    Confined,
    /// `Confined`, plus no network.
    Strict,
}

impl SandboxMode {
    /// The lowercase name used on the command line and in messages.
    pub fn as_str(&self) -> &'static str {
        match self {
            SandboxMode::Off => "off",
            SandboxMode::Confined => "confined",
            SandboxMode::Strict => "strict",
        }
    }

    /// True when the mode confines writes to the session root and scratch.
    pub fn confines_writes(&self) -> bool {
        matches!(self, SandboxMode::Confined | SandboxMode::Strict)
    }

    /// True when the mode denies network access.
    pub fn denies_network(&self) -> bool {
        matches!(self, SandboxMode::Strict)
    }
}

impl fmt::Display for SandboxMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SandboxMode {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "off" => Ok(SandboxMode::Off),
            "confined" => Ok(SandboxMode::Confined),
            "strict" => Ok(SandboxMode::Strict),
            other => Err(format!(
                "unknown sandbox mode \"{other}\". Use off, confined, or strict."
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_mode_default_is_off() {
        // The default must be stated and must not change under a user. See D-no-four-argument-session-new.
        assert_eq!(SandboxMode::default(), SandboxMode::Off);
    }

    #[test]
    fn sandbox_mode_parses_each_name() {
        assert_eq!("off".parse(), Ok(SandboxMode::Off));
        assert_eq!("confined".parse(), Ok(SandboxMode::Confined));
        assert_eq!("strict".parse(), Ok(SandboxMode::Strict));
    }

    #[test]
    fn sandbox_mode_rejects_an_unknown_name() {
        let error = SandboxMode::from_str("loose").unwrap_err();
        assert!(error.contains("loose"), "{error}");
    }

    #[test]
    fn confined_and_strict_confine_writes() {
        assert!(!SandboxMode::Off.confines_writes());
        assert!(SandboxMode::Confined.confines_writes());
        assert!(SandboxMode::Strict.confines_writes());
    }

    #[test]
    fn only_strict_denies_the_network() {
        assert!(!SandboxMode::Off.denies_network());
        assert!(!SandboxMode::Confined.denies_network());
        assert!(SandboxMode::Strict.denies_network());
    }
}
