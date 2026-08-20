# D-a-cancelled-waiter-says-so — a cancelled queued child never reports that it will start

**Found by driving it for real, not by a test.** AGENTS.md step 11, on live Bedrock, with
`--max-children-per-parent 1`. The model started two background children, cancelled the second
while it waited, and polled it at once. `agent_status` answered, verbatim:

```text
scout (id 2) is queued, at place 1 in its parent's line. It has not started, and it will start
when a sibling finishes. Cancel it with cancel_agent, or steer it now and it reads the message on
its first turn.
```

**Question:** What must `agent_status` say about a queued child that was cancelled a moment ago?

**Why the old answer was wrong.** A cancel wakes the waiting task, and that task removes the
entry when it drops. So there is a window where the entry is still in the queued index and its
token is already cancelled. In that window the text above told the model three false things: that
the child has a place in the line, that it **will** start, and that a steer would reach it. A
model that believes any of the three waits for work that will never arrive.

**Decision:** `AgentStatus::Queued` carries `cancelled: bool`, read from the entry's own token
under the state lock. `agent_status` states that the child gave up its place and will not start,
and it tells the parent to poll again for the final report.

**Why not remove the entry inside `cancel_descendant`.** Then the id would answer "not known in
this session tree" until the background task recorded its report, and a model that had just
cancelled the child would read that as a lost id. One writer removes a queued entry, and it is
`QueuedChild::drop`. Two removers would be two places that must agree.

**Why not a fourth variant.** `Cancelled` as a top-level state would need every reader to handle a
state that lasts microseconds, and the child still becomes `Finished` with
`AgentOutcome::Canceled`. One flag on the state that already exists says the same thing and adds
no case to the state machine.

**A new field on a variant is a source-breaking change, and that is the safe outcome.** The
compiler names every match that has to be reconciled. `AgentStatus` is never serialised, so no
older reader exists. See `SPEC-subagent-slots-handles-grace` section 2.3.

**Rules out:** Reporting a place in the line for a cancelled waiter. Offering a steer that cannot
be delivered. A second place that removes a queued entry. A fourth `AgentStatus` variant.
