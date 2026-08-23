use std::sync::Mutex;

use crate::subagent::error::SubagentError;

/// Tracks how many times a unit of work has died, keyed by a work key.
///
/// This is jcode's reclaim cap. When a caller re-delegates the same work and the
/// child dies again, the count grows. After [`MAX_CHILD_RETRIES`] the work is
/// reported failed rather than retried. See `SPEC-subagents` section 8.
pub struct RetryLedger {
    deaths: Mutex<std::collections::HashMap<String, u32>>,
    cap: u32,
}

impl RetryLedger {
    /// A ledger with the default retry cap.
    pub fn new() -> Self {
        Self {
            deaths: Mutex::new(std::collections::HashMap::new()),
            cap: MAX_CHILD_RETRIES,
        }
    }

    /// How many distinct work keys the ledger remembers.
    ///
    /// The map is bounded because its key holds the whole prompt, which a model
    /// writes. A parent that fails many distinct tasks would otherwise grow it
    /// without limit, and this project has already shipped one unbounded buffer. See
    /// decision D-bash-line-cap.
    const MAX_TRACKED_WORK: usize = 256;

    /// How many distinct work keys the ledger holds now.
    pub fn tracked(&self) -> usize {
        self.deaths
            .lock()
            .expect("the retry ledger lock is poisoned")
            .len()
    }

    /// Record one death for `key`.
    ///
    /// It returns the death count while the work may still retry. It returns a
    /// `RetryCapReached` error once the count reaches the cap, so the caller
    /// reports the work failed rather than retry a poisoned task forever.
    pub fn record_death(&self, key: &str) -> Result<u32, SubagentError> {
        let mut deaths = self
            .deaths
            .lock()
            .expect("the retry ledger lock is poisoned");
        // Forget the oldest tracking when the map is full. Losing a count is safe:
        // the work simply gets its retries again. Growing without a bound is not.
        if deaths.len() >= Self::MAX_TRACKED_WORK && !deaths.contains_key(key) {
            deaths.clear();
        }
        let count = deaths.entry(key.to_string()).or_insert(0);
        *count += 1;
        if *count >= self.cap {
            Err(SubagentError::RetryCapReached {
                deaths: *count,
                limit: self.cap,
            })
        } else {
            Ok(*count)
        }
    }
}

impl Default for RetryLedger {
    fn default() -> Self {
        Self::new()
    }
}

/// The most times a unit of work may die before it is reported failed.
///
/// This is jcode's reclaim cap. Without it a poisoned task loops until the
/// budget is gone. See `SPEC-subagents` section 8.
pub const MAX_CHILD_RETRIES: u32 = 3;
