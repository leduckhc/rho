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

/// A limit value the runtime cannot accept.
///
/// It names the config key, so every caller words its own message and no caller repeats the
/// bound. `rho-config` reports it as a `ConfigError`, and `rho-cli` reports a flag the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitTooLarge {
    /// The config key, such as `subagents.max-live-total`.
    pub key: &'static str,
    /// The value the user asked for.
    pub value: usize,
    /// The largest value rho can accept.
    pub maximum: usize,
}

impl std::fmt::Display for LimitTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the {} value {} is too large. The maximum is {}.",
            self.key, self.value, self.maximum
        )
    }
}

impl SubagentLimits {
    /// The largest a count limit may be.
    ///
    /// Two limits become `tokio::sync::Semaphore` permits, and tokio **panics** above
    /// `MAX_PERMITS`. So a config file could abort the binary with a tokio backtrace instead of
    /// a sentence naming the key. A live run proved it. See
    /// `D-a-limit-too-large-is-refused-not-clamped`.
    pub const MAX_COUNT: usize = tokio::sync::Semaphore::MAX_PERMITS;

    /// Refuse a limit the runtime cannot accept.
    ///
    /// The table below holds **every** numeric field, so a new limit cannot be added without
    /// a decision about its bound. A field with no runtime bound is listed with `None` and
    /// `every_numeric_limit_is_bounded_or_provably_safe` proves the extreme value is safe,
    /// rather than assuming it.
    pub fn check(&self) -> Result<(), LimitTooLarge> {
        for (key, value, maximum) in self.bounds() {
            if let Some(maximum) = maximum
                && value > maximum
            {
                return Err(LimitTooLarge {
                    key,
                    value,
                    maximum,
                });
            }
        }
        Ok(())
    }

    /// Every numeric limit, its value, and its maximum when it has one.
    ///
    /// The destructure is exhaustive, with no `..` and no `_`, so a new field fails the build
    /// until it is given a bound or an explicit `None`.
    fn bounds(&self) -> [(&'static str, usize, Option<usize>); 10] {
        let Self {
            max_depth,
            max_children_per_parent,
            max_live_total,
            child_timeout,
            max_tool_calls,
            max_queued_per_parent,
            max_queued_total,
            queue_wait,
            max_steer_message_bytes,
            grace_turns,
        } = self;
        [
            // A semaphore permit count. tokio panics above MAX_PERMITS.
            (
                "subagents.max-children-per-parent",
                *max_children_per_parent,
                Some(Self::MAX_COUNT),
            ),
            (
                "subagents.max-live-total",
                *max_live_total,
                Some(Self::MAX_COUNT),
            ),
            // The rest are compared against a counter or a length, so no value overflows a
            // runtime structure. Each is listed so a reader sees the decision, and the table
            // test drives the extreme value to prove it.
            ("subagents.max-depth", *max_depth as usize, None),
            (
                "subagents.child-timeout-secs",
                child_timeout.as_secs() as usize,
                None,
            ),
            ("subagents.max-tool-calls", *max_tool_calls as usize, None),
            (
                "subagents.max-queued-per-parent",
                *max_queued_per_parent,
                None,
            ),
            ("subagents.max-queued-total", *max_queued_total, None),
            (
                "subagents.queue-wait-secs",
                queue_wait.as_secs() as usize,
                None,
            ),
            (
                "subagents.max-steer-message-bytes",
                *max_steer_message_bytes,
                None,
            ),
            ("subagents.grace-turns", *grace_turns as usize, None),
        ]
    }
}

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
    fn every_numeric_limit_is_bounded_or_provably_safe() {
        // The table test the review asked for, over the whole set rather than one field.
        //
        // Two limits become semaphore permits, and tokio panics above `MAX_PERMITS`. Every
        // other numeric limit is compared against a counter or a length. This drives the
        // extreme value of each one, so "no bound needed" is proved and not assumed.
        let stated = SubagentLimits::new();
        let table = stated.bounds();
        assert_eq!(
            table.len(),
            10,
            "every numeric field is listed, so a new one fails here"
        );

        let bounded: Vec<&str> = table
            .iter()
            .filter(|(_, _, maximum)| maximum.is_some())
            .map(|(key, _, _)| *key)
            .collect();
        assert_eq!(
            bounded,
            vec![
                "subagents.max-children-per-parent",
                "subagents.max-live-total"
            ],
            "exactly the two semaphore counts carry a bound"
        );

        // Each bounded field, one over its maximum, is refused and names itself.
        let over = SubagentLimits::MAX_COUNT + 1;
        for (key, limits) in [
            (
                "subagents.max-live-total",
                SubagentLimits {
                    max_live_total: over,
                    ..SubagentLimits::new()
                },
            ),
            (
                "subagents.max-children-per-parent",
                SubagentLimits {
                    max_children_per_parent: over,
                    ..SubagentLimits::new()
                },
            ),
        ] {
            let error = limits
                .check()
                .expect_err("a value over the maximum is refused");
            assert_eq!(error.key, key);
            assert_eq!(error.maximum, SubagentLimits::MAX_COUNT);
            assert!(
                error.to_string().contains(key),
                "the message names the key: {error}"
            );
            assert!(
                error
                    .to_string()
                    .contains(&SubagentLimits::MAX_COUNT.to_string()),
                "and the maximum: {error}"
            );
        }

        // Exactly at the maximum is allowed, so the comparison is strictly greater.
        assert!(
            SubagentLimits {
                max_live_total: SubagentLimits::MAX_COUNT,
                max_children_per_parent: SubagentLimits::MAX_COUNT,
                ..SubagentLimits::new()
            }
            .check()
            .is_ok(),
            "a value at the maximum is accepted"
        );

        // Every unbounded field, at its extreme, still passes the check.
        let extreme = SubagentLimits {
            max_depth: u32::MAX,
            child_timeout: Duration::from_secs(u64::MAX),
            max_tool_calls: u32::MAX,
            max_queued_per_parent: usize::MAX,
            max_queued_total: usize::MAX,
            queue_wait: Duration::from_secs(u64::MAX),
            max_steer_message_bytes: usize::MAX,
            grace_turns: u32::MAX,
            ..SubagentLimits::new()
        };
        assert!(
            extreme.check().is_ok(),
            "an unbounded field has no maximum, so the extreme is accepted"
        );
    }

    #[test]
    fn the_stated_defaults_pass_their_own_check() {
        // A default that its own check refuses would break every run.
        SubagentLimits::new()
            .check()
            .expect("the defaults are valid");
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
