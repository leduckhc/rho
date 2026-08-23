# D-permits-not-counters — a semaphore replaces the compare-and-swap counters

**Question (step-9 reviewer):** `QueuedChild::started` was specified as "the same
compare-and-swap loop `spawn_child` uses". It is not the same, because `spawn_child` refuses
when full and `started` must wait. So what wakes a waiter, and what happens to the loser of a
race?

**The hole.** A counter cannot be waited on. The draft added a notify beside it. Two waiters
wake, both retry, and one loses. The loser has no error to return, because `Dequeued` holds only
`Cancelled`, so it must sleep again. If the notify fires between its failed retry and its next
await, it stores no permit, so the wake is lost. The waiter then sleeps until the next child
happens to finish, and it never wakes when none does. Both named tests still pass: one waiter
starts, and the cap holds under a race. Liveness breaks, and safety looks fine.

**Decision:** The reservation is a `tokio::sync::Semaphore` permit. The registry holds one
semaphore of `max_live_total` permits. Each node holds one of `max_children_per_parent`.
`ChildSlot` holds both owned permits and releases them on drop.

- `spawn_child` calls `try_acquire_owned` and refuses exactly as it does today.
- `QueuedChild::started` calls `acquire_owned` on the per-parent semaphore, and selects on the
  child's cancel token.
- The acquire order is per-parent first, then process-wide.

**Two properties come free.** A permit release grants the next waiter directly, so a lost wakeup
is unrepresentable. And `tokio::sync::Semaphore` is first-in-first-out, so the start order in the
spec comes from the primitive rather than from a second structure that could disagree.

**Why the order cannot deadlock.** A per-parent permit is contended only by one parent's own
children. Each of those is running, and will finish, or queued behind this child in one line. A
running child never waits for our permit, so no cycle exists.

**Rules out:** A counter plus a notify. A busy retry loop. A `Dequeued` variant that means "you
lost a race", which would push a rho scheduling detail into a caller.

**Test that proves the liveness the old design lacked:**
`every_waiter_eventually_starts_when_slots_free_one_at_a_time`, with more waiters than slots.
