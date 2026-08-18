# D-cancel-wake-race — Fix the `CancelToken::cancelled` wake race


**Finding (S3 tester):** `cancelled()` checks the flag, and only then awaits
`notify.notified()`. `Notify::notify_waiters` stores no permit. So on a
multi-thread runtime a `cancel()` between the flag check and the registration is
lost, and the waiter hangs. The tester's test passes only because it runs on the
current-thread runtime, where the interleaving cannot happen.

This is a confirmed bug. The rules forbid leaving one unfixed.

**Decision:** S4 fixes it. Create the `Notified` future first. Then check the
flag. Then await. The `Notified` future registers on creation, so no wake is
lost.

```rust
pub async fn cancelled(&self) {
    let notified = self.inner.notify.notified();
    if self.is_cancelled() {
        return;
    }
    notified.await;
}
```

**Test:** add `cancel_token_cancelled_wakes_on_multi_thread_runtime`, marked
`#[tokio::test(flavor = "multi_thread", worker_threads = 2)]`, with a bounded
timeout so a lost wake fails the test instead of hanging the suite. S4 may add
this test, because it guards a bug the spec did not describe.
