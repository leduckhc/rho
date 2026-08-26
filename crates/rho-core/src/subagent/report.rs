use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::usage::Usage;

/// The most characters a child summary may carry back to the parent.
///
/// The summary is the only thing the parent's context receives, so it must stay
/// small. A longer final answer is truncated to this many characters. See
/// `SPEC-subagents` section 6.
pub const MAX_SUMMARY_CHARS: usize = 8_000;

// --- The result contract (SPEC-subagents section 6) ---

/// What a child returns to its parent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentReport {
    pub agent: String,
    pub outcome: AgentOutcome,
    /// The child's final answer, capped. This is what the model sees.
    pub summary: String,
    /// Every turn's usage, summed. Feeds the budget governor and the status
    /// line.
    pub usage: Usage,
    pub turns: u32,
    /// rho's verified verdict on the task. Empty when the task declared no checks.
    ///
    /// This is the truth. `claims` is what the child said. An old record with no
    /// field reads as an empty, passing report, which is correct: a task with no
    /// declared check has nothing to fail.
    #[serde(default)]
    pub gate: crate::GateReport,
    /// The child's own, unverified claims. Never a substitute for `gate`.
    #[serde(default)]
    pub claims: crate::ChildClaims,
    /// Where the full transcript was written, for a human. Never sent to the
    /// model.
    pub transcript: Option<PathBuf>,
}

/// How a child's run ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentOutcome {
    Done,
    /// The child hit its own turn cap. The summary holds what it had.
    OutOfTurns,
    /// The child was cancelled, with its parent or alone.
    Canceled,
    /// The child failed. The parent continues.
    Failed {
        reason: String,
    },
    /// The child stopped, but rho's gate failed one or more checks. It holds the
    /// failed labels.
    ///
    /// It is never `Done`, so a reader that trusts only the outcome still sees a
    /// failure. Added by `SPEC-agent-tasks`. See decision
    /// D-a-child-does-not-grade-itself.
    Rejected {
        failed: Vec<String>,
    },
}

impl AgentOutcome {
    /// Whether this outcome is a failure the parent must notice.
    ///
    /// The only place that decides. Four sites used to answer this question in their
    /// own words, so a new variant meant editing four matches and the newest one
    /// defaulted to success. Ask the type instead.
    pub fn is_failure(&self) -> bool {
        !matches!(self, Self::Done)
    }

    /// The wire name, matching this enum's own serde spelling.
    ///
    /// A transcript groups by it, so a second spelling would split one outcome into
    /// two buckets.
    pub fn wire_name(&self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::OutOfTurns => "out_of_turns",
            Self::Canceled => "canceled",
            Self::Failed { .. } => "failed",
            Self::Rejected { .. } => "rejected",
        }
    }

    /// A short phrase for a human or a model, including the reason when there is one.
    ///
    /// **A phrase ends with no full stop.** A caller puts this inside a sentence of its
    /// own, and a reason is often a whole sentence itself. So `agent_status` printed
    /// "raise --queue-wait-secs.. 0 turn(s)" with two stops, and a live poll showed it.
    pub fn label(&self) -> String {
        match self {
            Self::Done => "done".to_string(),
            Self::OutOfTurns => "out of turns".to_string(),
            Self::Canceled => "cancelled".to_string(),
            Self::Failed { reason } => {
                format!("failed: {}", reason.trim_end().trim_end_matches('.'))
            }
            Self::Rejected { failed } => format!("rejected: {}", failed.join(", ")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_label_is_a_phrase_and_not_a_sentence() {
        // A live poll printed "raise --queue-wait-secs.. 0 turn(s)". The caller ends the
        // sentence, so the phrase must not end it too. A reason is often a whole
        // sentence, and every subagent refusal is.
        let outcome = AgentOutcome::Failed {
            reason: "the child waited 0 seconds for a slot. Ask for more.".to_string(),
        };
        let label = outcome.label();
        assert!(
            !label.ends_with('.'),
            "a phrase must not end a sentence: {label}"
        );
        assert!(
            label.contains("Ask for more"),
            "and it keeps every word of the reason: {label}"
        );
        // The stop inside the reason stays, because only the end is a caller's business.
        assert!(label.contains("slot. Ask"), "{label}");
    }
}
