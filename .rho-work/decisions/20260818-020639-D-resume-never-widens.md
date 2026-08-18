# D-resume-never-widens — A resume must not widen a permission


**Question (T1b architect):** what stops a read-only session resuming under a wider
mode when the live config changed?

**Decision:** The session header record carries the resolved `approval` and `sandbox`
mode names. A resume reads them and refuses a mode more permissive than the header
names, unless the user passes `--allow-widen`. The `approval` order is `read-only`,
`ask`, `allow-all`. The `sandbox` order is `strict`, `confined`, `off`. See `SPEC-sessions`
section 8a and `ADR-session-format`.

**Reason:** a session created read-only must not silently gain write access on a
resume. The header is the only record that outlives the run, so the modes belong
there. The comparison uses stored mode names, which form a total order, so it does not
need the trait comparison that `SPEC-subagents` proved impossible.

**Rules out:** a resume that reads the modes from the live config. A silent widen with
no flag. Storing an `ApprovalPolicy` object in the record.
