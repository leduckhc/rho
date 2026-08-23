# D-the-process-wide-cap-still-refuses — only the cap one parent controls may queue

**Question (step-9 reviewer):** The draft queued on both concurrency caps. `max_live_total` is
process-wide, across every tree in the process. What bounds a wait on it?

**Nothing bounds it.** A queued child spends none of its `child_timeout` while it waits, by
design. So tree A's fan-out can block inside `join_all` until tree B frees a slot, and tree B may
run every child for its full ten minutes. The old refusal, `TooManyLiveAgents`, always made
progress. The queue could hang one session on another session's behaviour, across a trust
boundary that this project defends everywhere else.

**Decision:** Only `max_children_per_parent` queues. `max_live_total` refuses at once, exactly as
it does today, and its message still names the limit and the flag.

**Reason:** A parent controls its own children. Waiting for one of them to finish is a wait on
work the caller started. Waiting for another tree is a wait on a stranger. The first is a
scheduling delay. The second is an availability failure dressed as a delay.

**The alternative, and why not.** A bounded wait plus a new `Dequeued::TimedOut` would work, and
it adds a timeout, a second failure mode, and a number nobody can choose well. The refusal is the
smaller contract, and a caller that wants to wait can retry.

**Rules out:** Any cross-tree wait. A queue that makes one session's throughput depend on
another's. A silent unbounded delay inside a fan-out.
