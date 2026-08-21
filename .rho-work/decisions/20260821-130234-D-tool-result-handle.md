# D-tool-result-handle — a capped tool result keeps a handle the model can read


**Question (controller):** `D-cap-a-large-tool-result` caps an oversize tool result and
spills the payload to a sidecar file. Nothing lets the model read that sidecar. Does rho
give the model a way back to the evidence?

**Decision:** Yes. A result over a threshold returns three things to the model: a bounded
head preview, the retained byte count, and a session-scoped handle. A new `read_tool_result`
tool reads a byte range from that handle, or searches it for a literal string.

The handle is session-scoped. It resolves only inside the session that made it. An exact
match is required, and a near miss is an error that names the reason.

**Reason:** rho already caps the record, so the tail is already on disk. Today that tail is
unreachable, so the model must re-run the command to see it. A re-run costs a second
process, a second wait, and a second output cap. fx solves the same problem with a
preview, a byte count, and a handle. The cap and the read-back are two halves of one
feature, and rho shipped only the first half.

**Why the model, not the harness, decides:** the harness cannot know which part of a
100,000-line log matters. A summarizer would have to guess, and a wrong guess destroys the
evidence. A byte range and a literal query let the model ask for exactly what it needs.

**Rules out:** sending the whole result and trusting the context window. Dropping the tail
with no note, which `D-cap-a-large-tool-result` already forbids. A handle that outlives its
session, because a stale handle would read another session's evidence. A summarizing model
call on the truncated tail, because that spends tokens on text nobody asked for.

**Supersedes nothing.** It completes `D-cap-a-large-tool-result`, and that decision stands.
