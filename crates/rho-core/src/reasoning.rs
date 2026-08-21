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

/// How hard the model should think.
///
/// This is the user's word, and never a wire value. A provider maps it: an
/// OpenAI-compatible host takes a word, and an Anthropic-style host takes a token budget.
///
/// `Option<ReasoningEffort>` carries "unset". `None` means the provider's own default, so
/// rho sends no field at all and the host keeps its behaviour. See
/// `SPEC-reasoning-across-providers` section 9.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    /// Ask the model not to think. A host with a disable switch gets it. A host without one
    /// gets no field, because rho must not invent a value.
    Off,
    Low,
    Medium,
    High,
    XHigh,
}

/// The lowest budget Anthropic accepts. A smaller value is a 400 for the whole turn.
pub const MIN_THINKING_BUDGET: u32 = 1024;

impl ReasoningEffort {
    /// The level name, as the config file, the flag, and the variable accept it.
    pub fn as_str(self) -> &'static str {
        match self {
            ReasoningEffort::Off => "off",
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
            ReasoningEffort::XHigh => "xhigh",
        }
    }

    /// The Anthropic-style token budget for this level. `None` for `Off`.
    ///
    /// The ladder starts at `MIN_THINKING_BUDGET`. The numbers are a starting point, and
    /// section 7 of the spec says a measurement gets its own bench.
    pub fn budget_tokens(self) -> Option<u32> {
        match self {
            ReasoningEffort::Off => None,
            ReasoningEffort::Low => Some(MIN_THINKING_BUDGET),
            ReasoningEffort::Medium => Some(4096),
            ReasoningEffort::High => Some(16_384),
            ReasoningEffort::XHigh => Some(32_768),
        }
    }
}

impl FromStr for ReasoningEffort {
    type Err = String;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "off" => Ok(ReasoningEffort::Off),
            "low" => Ok(ReasoningEffort::Low),
            "medium" => Ok(ReasoningEffort::Medium),
            "high" => Ok(ReasoningEffort::High),
            "xhigh" => Ok(ReasoningEffort::XHigh),
            other => Err(format!(
                "unknown reasoning effort \"{other}\": the valid levels are \
                 off, low, medium, high, and xhigh"
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

#[cfg(test)]
mod effort_tests {
    use super::*;

    // The effort level. See SPEC-reasoning-across-providers section 9.

    #[test]
    fn every_effort_name_round_trips() {
        for effort in [
            ReasoningEffort::Off,
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::XHigh,
        ] {
            assert_eq!(ReasoningEffort::from_str(effort.as_str()), Ok(effort));
        }
    }

    #[test]
    fn an_unknown_effort_name_is_an_error() {
        let error = ReasoningEffort::from_str("hard").unwrap_err();
        // The message names the valid levels, so a user can fix it without the docs.
        assert!(
            error.contains("xhigh"),
            "the error names the levels: {error}"
        );
    }

    #[test]
    fn the_budget_ladder_only_grows() {
        let ladder = [
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::XHigh,
        ];
        let budgets: Vec<u32> = ladder
            .iter()
            .map(|effort| effort.budget_tokens().expect("a level has a budget"))
            .collect();
        for pair in budgets.windows(2) {
            assert!(pair[1] > pair[0], "the ladder only grows: {budgets:?}");
        }
        // Anthropic refuses a budget under 1024, so the bottom rung must clear it.
        assert!(budgets[0] >= 1024, "the lowest budget is at least 1024");
    }

    #[test]
    fn off_has_no_budget() {
        assert_eq!(ReasoningEffort::Off.budget_tokens(), None);
    }
}
