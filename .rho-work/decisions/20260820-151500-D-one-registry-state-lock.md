# D-one-registry-state-lock — one mutex holds every index, and nothing holds it across an await

**Question (two second-pass reviewers, independently):** The registry gains a `queued` index
beside the live map and the finished ring. The move from queued to live was specified as
happening "in one lock". Which lock?

**The hole.** There was no such lock. `RegistryInner` holds a separate mutex per index today, so
a move between two of them is two operations. Three failures follow. A lookup can find an id in
both indexes, or in neither. A cancel can report failure while the child starts. And two
concurrent admissions can derive the same handle, because the numbering reads three indexes with
no snapshot. A first draft answered with a stated lock order, live then queued then finished.
That makes the races rarer and keeps every one of them possible.

**Decision:** One `Mutex<RegistryState>` holds the live handles, the queued entries, the finished
reports, and the handle bindings. Every index moves inside it. The lock is never held across an
await.

**What becomes true by construction.**

- The handout removes the queued entry and inserts the live handle in one critical section, so no
  id is ever in both indexes or in neither.
- `admit_child` derives and stores a handle in the same critical section as the registration, so
  two admissions of one agent name cannot pick one name.
- No lock-ordering rule is needed, so no ABBA deadlock can exist between these indexes.

**Why the await rule matters.** A permit wait can last minutes. `QueuedChild::started` acquires
its permits first and takes the lock afterwards. A lock held across an await would stop every
`agent_status` call in the process.

**The cost, stated.** One lock is coarser than three. Every operation here is a hash-map insert, a
remove, or a small scan, and the process-wide live cap is 32. So contention is bounded and small.
A finer design that is wrong is worse than a coarse design that is right.

**Rules out:** A mutex per index. A stated lock order as the answer to atomicity. Handle numbering
outside the registration lock. Holding the state lock across a permit wait.
