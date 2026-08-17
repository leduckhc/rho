//! Tests for the cancellation token.
//!
//! The async tests synchronise with a channel. No test sleeps.

use rho_core::CancelToken;
use tokio::sync::oneshot;

#[test]
fn cancel_token_starts_uncancelled() {
    let token = CancelToken::new();
    assert!(!token.is_cancelled());
}

#[test]
fn cancel_token_cancel_sets_flag() {
    let token = CancelToken::new();
    token.cancel();
    assert!(token.is_cancelled());
}

#[tokio::test]
async fn cancel_token_cancelled_resolves_after_cancel() {
    let token = CancelToken::new();
    let waiter = token.clone();
    let (started_tx, started_rx) = oneshot::channel();

    // The current-thread runtime runs the waiter until its first real await.
    // So the waiter registers inside `cancelled()` before `cancel` fires. This
    // removes the race and needs no sleep.
    let handle = tokio::spawn(async move {
        started_tx.send(()).unwrap();
        waiter.cancelled().await;
    });

    started_rx.await.unwrap();
    token.cancel();
    handle.await.unwrap();
    assert!(token.is_cancelled());
}

#[tokio::test]
async fn cancel_token_cancelled_returns_immediately_when_already_cancelled() {
    let token = CancelToken::new();
    token.cancel();
    // This must resolve at once. It does not block.
    token.cancelled().await;
    assert!(token.is_cancelled());
}
