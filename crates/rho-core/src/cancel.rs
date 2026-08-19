//! The cancellation token.
//!
//! `CancelToken` is a lightweight token. It uses an atomic flag and a `Notify`.
//! It adds no dependency. A clone shares the same state.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

/// A shared, cloneable cancellation signal.
///
/// A clone shares one state. A **child**, from [`CancelToken::child`], is a
/// separate signal that follows its parent one way: cancelling the parent cancels
/// the child, and cancelling the child leaves the parent alone.
#[derive(Clone, Default)]
pub struct CancelToken {
    inner: Arc<CancelInner>,
}

#[derive(Default)]
struct CancelInner {
    flag: AtomicBool,
    notify: Notify,
    /// The token this one follows, if any. One way, parent to child.
    parent: Option<CancelToken>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// A token that follows this one, in one direction only.
    ///
    /// Cancelling this token cancels the child. Cancelling the child does **not**
    /// cancel this one. A subagent needs exactly that: a parent that is cancelled
    /// must stop every descendant, and a child that hits its own timeout must not
    /// end its parent's run. Sharing one token gave the second behaviour, and a
    /// live run showed a child timeout killing the whole session.
    pub fn child(&self) -> Self {
        Self {
            inner: Arc::new(CancelInner {
                flag: AtomicBool::new(false),
                notify: Notify::new(),
                parent: Some(self.clone()),
            }),
        }
    }

    /// Request cancellation. Idempotent.
    ///
    /// It never touches the parent.
    pub fn cancel(&self) {
        self.inner.flag.store(true, Ordering::SeqCst);
        self.inner.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        if self.inner.flag.load(Ordering::SeqCst) {
            return true;
        }
        match &self.inner.parent {
            Some(parent) => parent.is_cancelled(),
            None => false,
        }
    }

    /// Resolve when the token is cancelled. Resolve at once if already cancelled.
    pub async fn cancelled(&self) {
        // Walk the ancestor chain first. The walk is iterative on purpose: an
        // `async fn` that awaits itself needs boxing, and the chain is short.
        let mut chain: Vec<&CancelInner> = Vec::new();
        let mut node = self;
        loop {
            chain.push(&node.inner);
            match &node.inner.parent {
                Some(parent) => node = parent,
                None => break,
            }
        }

        // Create every `Notified` before the flag check, for the reason below.
        //
        // Creating it takes a snapshot of the notify state. A `notify_waiters`
        // call after this line updates that state. Registration into the waiter
        // list happens on the first poll, not on creation. The first poll then
        // observes the snapshot difference and resolves. So a `cancel` that lands
        // after this line still wakes the waiter. The creation-time snapshot
        // removes the race. See decision D-cancel-wake-race.
        let waits: Vec<_> = chain
            .iter()
            .map(|inner| Box::pin(inner.notify.notified()))
            .collect();
        if self.is_cancelled() {
            return;
        }
        // Any ancestor cancelling wakes this token, because cancellation runs
        // parent to child.
        futures::future::select_all(waits).await;
    }
}

#[cfg(test)]
mod tests {
    use super::CancelToken;
    use std::time::Duration;

    // A child token propagates one way. A live run found the opposite: the child
    // shared the parent's token, so a child timeout cancelled the whole parent
    // session and the run ended silently. See docs/verification/subagents-bedrock.md.
    #[tokio::test]
    async fn a_child_token_cancelling_leaves_the_parent_live() {
        let parent = CancelToken::new();
        let child = parent.child();
        child.cancel();
        assert!(child.is_cancelled(), "the child must be cancelled");
        assert!(
            !parent.is_cancelled(),
            "a child must never cancel its parent"
        );
    }

    #[tokio::test]
    async fn cancelling_a_parent_cancels_every_descendant() {
        let parent = CancelToken::new();
        let child = parent.child();
        let grandchild = child.child();
        parent.cancel();
        assert!(child.is_cancelled(), "a child follows its parent");
        assert!(
            grandchild.is_cancelled(),
            "a grandchild follows its ancestor"
        );
    }

    #[tokio::test]
    async fn a_child_token_wakes_when_the_parent_cancels() {
        let parent = CancelToken::new();
        let child = parent.child();
        let waiter = tokio::spawn(async move { child.cancelled().await });
        parent.cancel();
        let woke = tokio::time::timeout(Duration::from_secs(5), waiter).await;
        assert!(woke.is_ok(), "a child must wake when its parent cancels");
        woke.unwrap().unwrap();
    }

    // This test guards D-cancel-wake-race. It runs on a multi-thread runtime, so `cancel`
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
