use std::time::Duration;

// --- Limits (SPEC-subagents section 7) ---

/// The four subagent limits. A child that spawns a child fans out
/// geometrically, so one limit is not enough. See `SPEC-subagents` section 7.
#[derive(Clone, Copy, Debug)]
pub struct SubagentLimits {
    /// How deep the tree may go. A depth of 0 forbids spawning.
    pub max_depth: u32,
    /// How many children one parent may run at once.
    pub max_children_per_parent: usize,
    /// How many agents may be live in the whole process, at any depth. jcode's
    /// absolute cap, and it protects the machine rather than the run.
    pub max_live_total: usize,
    /// How long a child may run before it is cancelled.
    pub child_timeout: Duration,
    /// How many tool calls one child may make.
    ///
    /// A turn cap counts provider round trips, so it does not bound a child that
    /// makes forty tool calls inside one turn. This does.
    pub max_tool_calls: u32,
    /// How many children one parent may queue for a slot. A full line refuses.
    pub max_queued_per_parent: usize,
    /// How many children may wait in the whole process. A full process refuses.
    ///
    /// A per-parent cap alone does not bound the process. A session root holds no
    /// live-child slot, so `max_live_total` caps neither the number of roots nor the
    /// number of wait lines, and a host may run many sessions in one process. See
    /// `SPEC-subagent-slots-handles-grace` section 2.6.
    pub max_queued_total: usize,
    /// Turns of warning before a child's turn cap. Zero disables the warning.
    ///
    /// A child that runs out of turns has nobody to ask for more, so it is warned
    /// and asked to write its summary. A top-level session defaults to zero, because
    /// a user is there to react. See `SPEC-subagent-slots-handles-grace` section 4.
    pub grace_turns: u32,
}

/// The subagent default grace window, in turns.
pub const DEFAULT_SUBAGENT_GRACE_TURNS: u32 = 5;

impl SubagentLimits {
    /// The starting limits, stated here and not hidden. See decision D-no-four-argument-session-new.
    ///
    /// Depth 2, four children per parent, 32 live in total, and a ten minute
    /// child timeout.
    pub fn new() -> Self {
        Self {
            max_depth: 2,
            max_children_per_parent: 4,
            max_live_total: 32,
            child_timeout: Duration::from_secs(600),
            max_tool_calls: 64,
            max_queued_per_parent: 16,
            max_queued_total: 128,
            grace_turns: DEFAULT_SUBAGENT_GRACE_TURNS,
        }
    }
}

impl Default for SubagentLimits {
    fn default() -> Self {
        Self::new()
    }
}
