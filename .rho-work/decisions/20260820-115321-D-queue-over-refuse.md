# D-queue-over-refuse — A concurrency cap queues a child instead of refusing it


**Question (architect):** what happens when a spawn passes a concurrency cap?

**Decision:** A concurrency cap now queues the child. The spawn returns a live id at once,
and the child starts when a slot frees. The two caps that queue are
`max_children_per_parent` and `max_live_total`. The model addresses a queued child by its
id, so it can poll, steer, or cancel it. See `SPEC-subagent-slots-handles-grace`.

**Reason:** pi's `spawn()` returns an id at once and queues over its concurrency limit.
The work is admissible, so a refusal wastes it. A queued id keeps the work alive and out
of the parent's context. The old refusal taught the model to wait or to raise a flag, and
rho can do the waiting itself.

**Rules out:** a tool call that blocks the model on a full cap with no id. Silently
dropping work over a cap. Turning a fixable wait into a permanent refusal.
