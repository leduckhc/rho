//! The cancellation token.
//!
//! `CancelToken` is a lightweight token. It uses an atomic flag and a `Notify`.
//! It adds no dependency. A clone shares the same state.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

/// A shared, cloneable cancellation signal.
#[derive(Clone, Default)]
pub struct CancelToken {
    inner: Arc<CancelInner>,
}

#[derive(Default)]
struct CancelInner {
    flag: AtomicBool,
    notify: Notify,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation. Idempotent.
    pub fn cancel(&self) {
        self.inner.flag.store(true, Ordering::SeqCst);
        self.inner.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.flag.load(Ordering::SeqCst)
    }

    /// Resolve when the token is cancelled. Resolve at once if already cancelled.
    pub async fn cancelled(&self) {
        // Register the waiter before the flag check. `Notified` registers on
        // creation. So a `cancel` that lands after this line still wakes the
        // waiter. `notify_waiters` stores no permit, so a later registration
        // would miss the wake. This order removes that race.
        let notified = self.inner.notify.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

#[cfg(test)]
mod tests {
    use super::CancelToken;
    use std::time::Duration;

    // This test guards D-009. It runs on a multi-thread runtime, so `cancel`
    // can land between the flag check and the waiter registration. The bounded
    // timeout fails the test on a lost wake, instead of hanging the suite.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_token_cancelled_wakes_on_multi_thread_runtime() {
        for _ in 0..1000 {
            let token = CancelToken::new();
            let waiter = token.clone();
            let handle = tokio::spawn(async move { waiter.cancelled().await });
            token.cancel();
            let woke = tokio::time::timeout(Duration::from_secs(5), handle).await;
            assert!(woke.is_ok(), "cancelled() must wake after cancel");
            woke.unwrap().unwrap();
        }
    }
}
