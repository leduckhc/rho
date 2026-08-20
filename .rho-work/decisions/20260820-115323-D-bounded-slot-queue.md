# D-bounded-slot-queue — The slot wait line is bounded


**Question (architect):** what bounds the queue of children that wait for a slot?

**Decision:** `SubagentLimits` gains `max_queued_per_parent`, default 16, flag
`--max-queued-per-parent`. A full wait line refuses at once with
`SubagentError::QueueFull { limit }`. The refusal names the limit and what to do. See
`SPEC-subagent-slots-handles-grace`.

**Reason:** an unbounded queue is a memory defect, and this project shipped one: an
unbounded `bash` reader turned 8 MB into 805 MB. A queued child holds a cancel token and a
message queue, so an unbounded wait line grows without limit. A bound keeps the wait line
small.

**Rules out:** an unbounded wait line. A fan-out of a thousand tasks queueing a thousand
children. A silent drop of a queued child.
