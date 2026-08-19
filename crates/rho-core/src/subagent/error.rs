use crate::sandbox::SandboxMode;

// --- Refusals (SPEC-subagents section 7: refusing must teach) ---

/// A refusal to spawn or run a child. Every variant names the limit, its value,
/// and what to do. See `SPEC-subagents` section 7.
#[derive(Clone, Debug, thiserror::Error)]
pub enum SubagentError {
    #[error("a task needs a goal. Say what the child must achieve, not only which agent to run.")]
    EmptyGoal,
    #[error(
        "the depth limit is {limit} and this would be depth {attempted}. \
         Do the work here. A subagent started from the rho command line holds no \
         spawn tool, so it cannot delegate further."
    )]
    DepthExceeded { limit: u32, attempted: u32 },
    #[error(
        "the per-parent child limit is {limit} and this parent already runs {current}. \
         Wait for a child to finish, or ask the user to raise --max-children-per-parent."
    )]
    TooManyChildren { limit: usize, current: usize },
    #[error(
        "the process-wide agent limit is {limit} and {current} agents are live. \
         Wait for an agent to finish, or ask the user to raise --max-live-agents."
    )]
    TooManyLiveAgents { limit: usize, current: usize },
    #[error(
        "a child may not weaken the sandbox. The parent mode is {parent} and the child \
         asked for {requested}. Ask for {parent} or a stricter mode."
    )]
    WeakerSandbox {
        parent: SandboxMode,
        requested: SandboxMode,
    },
    #[error(
        "a cycle was found in the parent chain, so the spawn is refused rather than looped. \
         This needs a bug fix, not a retry."
    )]
    CycleDetected,
    #[error(
        "the work failed {deaths} times, at the retry limit of {limit}. \
         Report the failure to the user rather than retry the same work."
    )]
    RetryCapReached { deaths: u32, limit: u32 },
}
