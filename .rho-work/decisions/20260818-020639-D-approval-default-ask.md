# D-approval-default-ask — The approval default is Ask where answerable, read-only where not


**Question (T1b architect):** what approval mode does rho use when the user states
none, and how is it resolved per frontend?

**Decision:** The owner chose an interactive Ask policy. rho defaults to `ask`
wherever a human or a client can answer, and to `read-only` where nobody can answer.
Allow-all is never a default. The resolution table in `SPEC-approval` section 4 binds each
case: an interactive TUI, `rho run` with a terminal, and `rho acp` with a capable
client resolve to Ask; `rho run` with no terminal and `rho acp` with an incapable
client resolve to read-only; a subagent child composes the parent policy.

**Reason:** allow-all as a default is the fail-open shape that decisions D-todo-in-a-green-stage and
D-plugin-does-not-classify-itself removed elsewhere. A human or a capable client can approve a mutating call, so
Ask is safe and useful there. Nobody can approve a headless call, so read-only is the
only safe default there.

**Rules out:** allow-all as any resolved default. A single global default that
ignores the frontend. Asking a client that never declared the capability.
