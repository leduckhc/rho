//! How rho draws a model's reasoning.
//!
//! rho keeps the reasoning text for every turn, whatever the display mode. So a user who
//! switches to `full` mid-session still sees the earlier reasoning. The memory cost is
//! small and bounded per turn, and losing the text would be worse. See
//! `SPEC-reasoning-across-providers` section 5.

use std::str::FromStr;

/// How rho draws reasoning. jcode has `Off`, `Full`, and `Current`. rho keeps `Off` and
/// `Full`, splits `Current` into `Live`, and adds a `Summary` default.
///
/// `Summary` is the default, and that differs from jcode, whose default is `Off`. rho
/// shows the one-row summary because a turn that thought for nine seconds and says so is
/// honest, while a turn that hides the nine seconds looks stalled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReasoningDisplay {
    /// Draw nothing at all, not even the summary row. This hides reasoning entirely.
    Off,
    /// Draw a one-row summary only, for example `∴ thought for 2.4s`.
    #[default]
    Summary,
    /// Draw the summary row and the reasoning text, dimmed.
    Full,
    /// Draw the reasoning text while it streams, then collapse it to the summary row once
    /// the model commits an answer or a tool call.
    Live,
}

impl ReasoningDisplay {
    /// The mode name, as the config file and the CLI accept it.
    pub fn as_str(self) -> &'static str {
        match self {
            ReasoningDisplay::Off => "off",
            ReasoningDisplay::Summary => "summary",
            ReasoningDisplay::Full => "full",
            ReasoningDisplay::Live => "live",
        }
    }
}

impl FromStr for ReasoningDisplay {
    type Err = String;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "off" => Ok(ReasoningDisplay::Off),
            "summary" => Ok(ReasoningDisplay::Summary),
            "full" => Ok(ReasoningDisplay::Full),
            "live" => Ok(ReasoningDisplay::Live),
            other => Err(format!(
                "unknown reasoning mode \"{other}\": the valid names are \
                 off, summary, full, and live"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_mode_is_summary() {
        assert_eq!(ReasoningDisplay::default(), ReasoningDisplay::Summary);
    }

    #[test]
    fn every_name_round_trips() {
        for mode in [
            ReasoningDisplay::Off,
            ReasoningDisplay::Summary,
            ReasoningDisplay::Full,
            ReasoningDisplay::Live,
        ] {
            assert_eq!(ReasoningDisplay::from_str(mode.as_str()), Ok(mode));
        }
    }

    #[test]
    fn an_unknown_name_is_an_error() {
        assert!(ReasoningDisplay::from_str("loud").is_err());
    }
}
