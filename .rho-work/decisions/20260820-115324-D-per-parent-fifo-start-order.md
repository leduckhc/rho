# D-per-parent-fifo-start-order — A queued child starts in per-parent arrival order


**Question (architect):** in what order do queued children start?

**Decision:** Start order is first-in-first-out per parent. A child that queued earlier
under one parent starts before a later one under that parent. Order between two parents is
unspecified. See `SPEC-subagent-slots-handles-grace`.

**Reason:** a single fan-out cares that its own tasks start in the order it asked. Two
independent library callers share no clock, and a global order would need a central
scheduler. The compare-and-swap reservation avoids a central lock on purpose, so rho does
not add one. Result order is separate: `spawn_agents` returns results in request order
whatever the start order.

**Rules out:** a global start order across parents. A central scheduler lock. A fan-out
whose result order depends on which child started first.
