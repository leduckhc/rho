# D-caps-that-cannot-wait-refuse — Depth and the cycle guard still refuse at once


**Question (architect):** which subagent caps queue, and which still refuse?

**Decision:** `max_depth` and the cycle guard refuse at once. They never queue. The
concurrency caps queue instead. See `SPEC-subagent-slots-handles-grace` and
D-queue-over-refuse.

**Reason:** waiting frees a concurrency slot, so a concurrency cap can queue. Waiting adds
no depth and breaks no cycle, so a queue there would wait forever. A refusal that teaches
is the honest answer for a cap that waiting cannot fix. The refusal names the limit and
its value.

**Rules out:** queueing a spawn that depth or a cycle forbids. A queued child that can
never start. A refusal that names a flag that cannot help.
