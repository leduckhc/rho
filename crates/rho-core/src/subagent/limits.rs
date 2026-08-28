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
    /// How long one child may wait for a slot. Then rho refuses it.
    ///
    /// It bounds one blocking spawn, which the wait line depth used to multiply: the old
    /// worst case was `ceil(max_queued_per_parent / max_children_per_parent)` times
    /// `child_timeout`, about forty minutes at the defaults. Zero refuses any child that
    /// has to wait, and no value turns the deadline off. See
    /// `SPEC-subagent-slots-handles-grace` section 2.8 and decision D-a-waiter-has-a-deadline.
    pub queue_wait: Duration,
    /// The largest steering message a child's queue accepts, in bytes.
    ///
    /// A child queue is written to by a model, and 160 of them may exist at the shipped
    /// defaults. So this is smaller than `MAX_STEER_MESSAGE_BYTES`, which a person types
    /// into. See `SPEC-steering` section 4 and decision
    /// D-a-steering-message-is-bounded-by-bytes.
    pub max_steer_message_bytes: usize,
    /// Turns of warning before a child's turn cap. Zero disables the warning.
    ///
    /// A child that runs out of turns has nobody to ask for more, so it is warned
    /// and asked to write its summary. A top-level session defaults to zero, because
    /// a user is there to react. See `SPEC-subagent-slots-handles-grace` section 4.
    pub grace_turns: u32,
}

/// The subagent default grace window, in turns.
pub const DEFAULT_SUBAGENT_GRACE_TURNS: u32 = 5;

/// How long a queued child waits for a slot by default.
///
/// It is one `child_timeout`, so a waiter gets one whole sibling run of patience and no
/// more. See decision D-a-waiter-has-a-deadline.
pub const DEFAULT_QUEUE_WAIT: Duration = Duration::from_secs(600);

/// The largest steering message a child's queue accepts by default, in bytes.
///
/// 16 KiB is a long instruction and a short document. A model writes these, and 160 child
/// queues may exist, so the product is 80 MiB rather than the 320 MiB a session-sized cap
/// would allow. See decision D-a-steering-message-is-bounded-by-bytes.
pub const DEFAULT_AGENT_STEER_MESSAGE_BYTES: usize = 16 * 1024;

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
            queue_wait: DEFAULT_QUEUE_WAIT,
            max_steer_message_bytes: DEFAULT_AGENT_STEER_MESSAGE_BYTES,
            grace_turns: DEFAULT_SUBAGENT_GRACE_TURNS,
        }
    }
}

impl Default for SubagentLimits {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_child_byte_cap_is_smaller_than_the_session_one() {
        // Two different numbers, on purpose. One value for both would either refuse a
        // paste from a user, or raise the process ceiling to 320 MiB. See decision
        // D-a-steering-message-is-bounded-by-bytes.
        let limits = SubagentLimits::new();
        assert_eq!(limits.max_steer_message_bytes, 16 * 1024);
        assert_eq!(
            limits.max_steer_message_bytes,
            DEFAULT_AGENT_STEER_MESSAGE_BYTES
        );
        assert_eq!(crate::MAX_STEER_MESSAGE_BYTES, 64 * 1024);
        assert!(
            limits.max_steer_message_bytes < crate::MAX_STEER_MESSAGE_BYTES,
            "a child queue is written to by a model, and there may be 160 of them"
        );
        // The product is the bound this cap exists to state: 160 queues, 32 messages
        // each, and 16 KiB a message is 80 MiB.
        let queues = limits.max_queued_total + limits.max_live_total;
        let ceiling = queues * crate::STEER_QUEUE_CAPACITY * limits.max_steer_message_bytes;
        assert_eq!(ceiling, 80 * 1024 * 1024, "the stated ceiling is 80 MiB");
    }

    #[test]
    fn the_queue_wait_deadline_is_one_child_timeout() {
        // A waiter gets one whole sibling run of patience, and no more. See decision
        // D-a-waiter-has-a-deadline.
        let limits = SubagentLimits::new();
        assert_eq!(limits.queue_wait, DEFAULT_QUEUE_WAIT);
        assert_eq!(limits.queue_wait, limits.child_timeout);
    }
}
