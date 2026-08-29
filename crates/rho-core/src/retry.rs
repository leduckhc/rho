//! The retry policy for a provider request.
//!
//! See `SPEC-provider-interface` section 3 and decision D-secret-in-core.
//!
//! This type lives in `rho-core` on purpose. It once sat in the OpenRouter crate,
//! so the other two providers had no policy at all. A retry policy that retries a
//! 401 burns a user's rate limit on a wrong key, and it never succeeds. That rule
//! must be stated once, and tested once.

use crate::ProviderError;
use std::time::Duration;

/// How a provider retries a failed request.
///
/// The policy reads [`ProviderError::is_retryable`]. It retries `Transport`,
/// `Server`, and `RateLimited`. It never retries `Client`, `Decode`, or `Auth`,
/// because none of those succeed on a second attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    /// The total number of attempts, including the first one.
    pub max_attempts: u32,
    /// The delay before the second attempt, in milliseconds.
    pub base_delay_ms: u64,
    /// The ceiling on any single delay, in milliseconds.
    pub max_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay_ms: 500,
            max_delay_ms: 30_000,
        }
    }
}

impl RetryPolicy {
    /// A policy that never retries. Useful in a test, and for a caller who wants
    /// to handle failure itself.
    pub fn none() -> Self {
        Self {
            max_attempts: 1,
            base_delay_ms: 0,
            max_delay_ms: 0,
        }
    }

    /// True when the error may be retried under this policy at `attempt`.
    ///
    /// `attempt` is one-based. The first request is attempt 1.
    pub fn should_retry(&self, error: &ProviderError, attempt: u32) -> bool {
        error.is_retryable() && attempt < self.max_attempts
    }

    /// The delay before `attempt`, one-based, given an optional server hint.
    ///
    /// The growth is exponential with full jitter. Full jitter picks a delay
    /// anywhere in `[0, window)`. That spreads a thundering herd, which a fixed
    /// backoff does not. See `SPEC-provider-interface` section 3.
    ///
    /// A server hint wins and skips the jitter, because the server knows better
    /// than we do. The hint is still capped by `max_delay_ms`, so a hostile or
    /// mistaken header cannot stall a session for an hour.
    ///
    /// Returns `None` when `attempt` is at or past `max_attempts`, which means do
    /// not retry.
    pub fn backoff(&self, attempt: u32, retry_after_ms: Option<u64>) -> Option<Duration> {
        if attempt == 0 || attempt >= self.max_attempts {
            return None;
        }
        if let Some(hint) = retry_after_ms {
            return Some(Duration::from_millis(hint.min(self.max_delay_ms)));
        }
        // `attempt` is one-based, so the first retry uses the base delay.
        let exponent = attempt.saturating_sub(1).min(20);
        let window = self
            .base_delay_ms
            .saturating_mul(1u64 << exponent)
            .min(self.max_delay_ms);
        Some(Duration::from_millis(full_jitter(window)))
    }
}

/// Pick a value in `[0, window)`. Return 0 when the window is 0.
///
/// This uses a cheap time-seeded generator. A retry delay needs spread, not
/// cryptographic quality, so a random-number dependency is not worth its weight.
fn full_jitter(window: u64) -> u64 {
    if window == 0 {
        return 0;
    }
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    // A multiplicative mix, so consecutive nanosecond values do not correlate.
    let mixed = nanos
        .wrapping_mul(6_364_136_223_846_793_005)
        .rotate_left(17);
    mixed % window
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_policy_retries_a_rate_limit() {
        let policy = RetryPolicy::default();
        let error = ProviderError::RateLimited {
            retry_after_ms: None,
        };
        assert!(policy.should_retry(&error, 1));
    }

    #[test]
    fn retry_policy_retries_a_server_error() {
        let policy = RetryPolicy::default();
        assert!(policy.should_retry(&ProviderError::Server { status: 503 }, 1));
    }

    #[test]
    fn retry_policy_retries_a_transport_error() {
        let policy = RetryPolicy::default();
        assert!(policy.should_retry(&ProviderError::Transport("reset".into()), 1));
    }

    #[test]
    fn retry_policy_never_retries_an_unauthorised_request() {
        // The rule that matters most. A 401 means the key is wrong. A retry burns
        // the user's rate limit and still fails.
        let policy = RetryPolicy::default();
        let error = ProviderError::Client {
            status: 401,
            advice: "invalid api key",
        };
        assert!(!policy.should_retry(&error, 1));
    }

    #[test]
    fn retry_policy_never_retries_any_client_error() {
        let policy = RetryPolicy::default();
        for status in [400, 401, 403, 404, 422] {
            let error = ProviderError::Client {
                status,
                advice: "no",
            };
            assert!(!policy.should_retry(&error, 1), "{status} must not retry");
        }
    }

    #[test]
    fn retry_policy_never_retries_a_decode_or_auth_error() {
        let policy = RetryPolicy::default();
        assert!(!policy.should_retry(&ProviderError::Decode("bad".into()), 1));
        assert!(!policy.should_retry(&ProviderError::Auth("no creds".into()), 1));
    }

    #[test]
    fn retry_policy_stops_at_the_attempt_cap() {
        let policy = RetryPolicy::default();
        let error = ProviderError::Server { status: 500 };
        assert!(policy.should_retry(&error, policy.max_attempts - 1));
        assert!(!policy.should_retry(&error, policy.max_attempts));
    }

    #[test]
    fn retry_policy_none_never_retries_a_retryable_error() {
        let policy = RetryPolicy::none();
        assert!(!policy.should_retry(&ProviderError::Server { status: 500 }, 1));
        assert_eq!(policy.backoff(1, None), None);
    }

    #[test]
    fn backoff_returns_none_past_the_cap() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.backoff(policy.max_attempts, None), None);
        assert_eq!(policy.backoff(0, None), None);
    }

    #[test]
    fn backoff_stays_inside_the_window_and_grows() {
        let policy = RetryPolicy::default();
        // Full jitter picks a value in `[0, window)`, so assert the bound, never a
        // fixed value. A test that asserts one value would be flaky by design.
        for attempt in 1..policy.max_attempts {
            let window = policy
                .base_delay_ms
                .saturating_mul(1u64 << (attempt - 1))
                .min(policy.max_delay_ms);
            for _ in 0..64 {
                let delay = policy.backoff(attempt, None).expect("a delay");
                assert!(
                    (delay.as_millis() as u64) < window.max(1),
                    "attempt {attempt} delay {delay:?} left the window {window}"
                );
            }
        }
    }

    #[test]
    fn backoff_never_passes_the_ceiling() {
        let policy = RetryPolicy {
            max_attempts: 30,
            base_delay_ms: 1_000,
            max_delay_ms: 5_000,
        };
        for attempt in 1..policy.max_attempts {
            let delay = policy.backoff(attempt, None).expect("a delay");
            assert!(delay.as_millis() as u64 <= policy.max_delay_ms);
        }
    }

    #[test]
    fn backoff_uses_a_server_hint_without_jitter() {
        let policy = RetryPolicy::default();
        let delay = policy.backoff(1, Some(1_234)).expect("a delay");
        assert_eq!(delay, Duration::from_millis(1_234));
    }

    #[test]
    fn backoff_caps_a_hostile_server_hint() {
        // A mistaken or hostile `Retry-After` must not stall a session for an hour.
        let policy = RetryPolicy::default();
        let delay = policy.backoff(1, Some(3_600_000)).expect("a delay");
        assert_eq!(delay.as_millis() as u64, policy.max_delay_ms);
    }

    #[test]
    fn backoff_spreads_across_calls() {
        // Full jitter exists to spread a herd. If every caller picked the same
        // delay, the retry would rebuild the spike it is meant to avoid.
        let policy = RetryPolicy::default();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            seen.insert(policy.backoff(3, None).expect("a delay").as_millis());
        }
        assert!(seen.len() > 1, "the jitter produced one value only");
    }
}
