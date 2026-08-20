# D-session-context-accessor — `Session` gets a read-only context accessor


**Question (S3 tester):** `SPEC-core-runtime` gives `Session` no way to read its `Context`.
So `agent_loop_appends_assistant_and_tool_messages` cannot assert on the context.
The tester asserted the equivalent ordering through the event stream instead.

**Decision:** Add a read-only accessor to `Session`. Name it `messages`. It
returns a borrowed slice, or a cheap snapshot if a lock forces that. Add it to
`SPEC-core-runtime`.

**Reason:** the append-only rule is a core invariant. An invariant that a test
cannot observe is an invariant that will break in silence. A read-only accessor
does not weaken encapsulation, because it grants no mutation. Keep the
event-stream assertion as well. Two views of one invariant are better than one.

**Constraint:** read only. No public API may mutate the context from outside a
turn.
