# D-a-queued-child-lives-in-the-registry — a queued child is registered, and every lookup is scoped

**Question (step-9 reviewer, on the slot-queue draft):** The spec promises a queued child is
pollable, steerable, and cancellable by the same id a started child uses. Which structure holds
it?

**The hole.** None. `admit_child` returned the `QueuedChild` to the spawning task and registered
nothing. Every tool reaches a child through `AgentRegistry::descendant` or
`AgentRegistry::status`, and both read only the live map and the finished ring. So
`steer_agent`, `cancel_agent`, and `agent_status` would answer "no subagent with id N" for a
queued child. The promise was false, and the queue tests would not have caught it, because they
drive `QueuedChild` directly and never the tool path.

**Decision:** `RegistryInner` gains a third structure, `queued`, keyed by id. Each entry carries
the agent name, the depth, the **ancestor chain**, the cancel token, the message queue, and the
position. `status`, `cancel_descendant`, `resolve`, and the scoped steer all consult it.
`descendant` does not, because a `LiveAgent` addresses a running child.

**The ancestor chain is not optional.** A map keyed by id alone would let one tree reach another
tree's queued child. That is the escape a security review already found once for the live map.
So the new map repeats the finished ring's shape, where the chain is stored beside the entry
because the handle that carried it is gone. See decision D-a-caller-addresses-only-its-own.

**One id is never in two structures.** `admit_child` inserts the entry, and `started` removes it
in the same lock that hands out the slot.

**Rules out:** A queued child held only by its spawning task. A lookup keyed by id alone. A
promise of addressability proved by a type-level test that never touches a tool.

**Proof it is needed:** `crates/rho-tools/src/control.rs:98`, `:170`, and `:280` all resolve
through the scoped accessors, and `crates/rho-core/src/subagent/tree.rs:227` and `:331` read only
the live map and the finished ring.
