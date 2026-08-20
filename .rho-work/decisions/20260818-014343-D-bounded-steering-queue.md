# D-bounded-steering-queue — The steering queue is bounded, and a cancel keeps it


**Question (T1 architect):** how does rho hold a user message that arrives mid-turn, and
what does a cancel do to it?

**Decision:** A bounded `MessageQueue` holds it, capacity 32. A full queue returns a
typed `QueueError::Full` and drops no earlier message. The driver drains the queue at a
turn boundary, before the next provider request. A cancel keeps the queue. A frontend
clears it explicitly. See `SPEC-steering`.

**Reason:** an unbounded queue is a memory defect, and defect 7 already shipped one: an
unbounded `bash` reader turned 8 MB into 805 MB. Dropping a user message in silence is
the worse failure, so a cancel keeps the queue rather than discarding user input.

**Rules out:** an unbounded queue. Injecting a message into the middle of a provider
request. A cancel silently dropping queued user input.
