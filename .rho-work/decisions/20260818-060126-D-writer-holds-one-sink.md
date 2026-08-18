# D-writer-holds-one-sink — The writer holds one sink, and a test injects a failing sink

**Question (controller, T5 review):** the writer reopened the session file per record,
because three degrade tests removed the session directory. Is that the design, or is it the
test?

**Decision:** The writer holds one open sink for the life of the session, and it writes one
record as one write. `SessionWriter::with_sink` takes the sink, so a test injects a sink
that fails on demand. The three degrade tests now use that seam instead of removing a
directory. rho does not `fsync` per record.

**Reason:** Measured, not argued. A reopen costs 17567 ns per record, a held sink costs
1034 ns, and an `fsync` per record costs 3076785 ns. So a reopen is 17 times slower than a
held sink, in a project that exists because a session must be cheap. The test could only
pass with a reopen, because an open descriptor on Unix keeps writing to an unlinked inode.
`AGENTS.md` says a test that forces a defect into the design is the reviewer's call, and
this was that case. The developer flagged it rather than hiding it, which is what the rule
asks for. A real append through the finished writer now costs 1762 ns per record, which
includes the encode and the id.

**Rules out:** A reopen per record. An `fsync` per record, which costs 3000 times the
write. A degrade test that depends on filesystem behaviour rather than on a stated seam.
