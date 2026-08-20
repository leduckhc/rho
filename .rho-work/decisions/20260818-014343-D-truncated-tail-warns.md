# D-truncated-tail-warns — Resume drops a truncated last line and warns, and keeps every whole record


**Question (T1 architect):** what does resume do when a crash cut the last line in half?

**Decision:** The reader decodes every whole line. It drops only a last line that fails
to decode. It keeps every whole record before it. It sets `truncated_tail`, and the
resume path logs one warning.

**Reason:** a crash mid-write is normal for a long session. A half-written tail must
never fail a resume, and must never discard a good record.

**Rules out:** failing a resume on a partial tail. Discarding records before a bad tail.
