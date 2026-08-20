# D-append-only-jsonl — The session file is append-only JSONL with a per-record parent pointer


**Question (T1 architect):** what shape does the session file take, so a resume and a
branch both work and a third party can read it?

**Decision:** Append-only JSONL. One record per line. Each record carries an id, a
parent id, and a timestamp. A branch names an earlier record as its parent. See
`ADR-session-format` and `SPEC-sessions`.

**Reason:** the append path is one write, so a crash never corrupts a written record. A
branch is a tree walk, not a file copy. A plain reader reads the file, which keeps the
F-append-only-session-log promise real.

**Rules out:** a single rewritten JSON document, because it pays an O(n) write per
message and a crash corrupts the whole file. An embedded database, because it adds a C
dependency and a file lock that contends across many sessions in one process.
