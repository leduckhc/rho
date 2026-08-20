# D-grace-turn-warning — A child is warned before its turn cap


**Question (architect):** how does a child learn it is about to run out of turns?

**Decision:** rho warns the child a fixed number of turns before the cap. The default is 5
grace turns. The warning is delivered through the steering queue, at a turn boundary,
before the next request. Exactly one warning is delivered. The warning applies to the turn
cap only, not the tool-call budget. The warning is not a turn. See
`SPEC-subagent-slots-handles-grace`.

**Reason:** pi warns a child before the hard limit, so the child can write its summary. A
child that hits the cap with no warning returns whatever summary existed. The steering
queue already delivers a user turn through `Context::append`, so the sent prefix stays
byte-identical and the provider cache stays warm.

**Rules out:** a hard turn cap with no warning. A second warning that repeats. A warning
that drops a user message when the queue is full. A warning that lies about the turns that
remain. A grace warning for the tool-call budget, which has no safe turn boundary.
