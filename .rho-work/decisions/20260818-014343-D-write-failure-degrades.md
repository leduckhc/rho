# D-write-failure-degrades — A write failure degrades a session to ephemeral, and never ends the run


**Question (T1 architect):** what happens when a session write fails mid-run?

**Decision:** `SessionLog::record` degrades the log to ephemeral with a warning, and the
run continues. It never ends the run.

**Reason:** defect 9 in `.rho-work/progress.md` shipped the opposite. One failure killed
a whole session, and 222 tests passed while the product was unusable. A lost log is a
degraded session, not a dead one.

**Rules out:** ending a run because a disk write failed. Silently losing the log with no
warning.
