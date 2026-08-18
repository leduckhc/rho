# D-cap-a-large-tool-result — A large tool result is capped in the record, not stored verbatim


**Question (T1 architect):** what stops a ten-megabyte tool result from bloating the
session file?

**Decision:** The writer caps one record at `MAX_RECORD_BYTES`. For an oversize tool
result it stores the head plus a note that states the full byte count. The full payload
spills to a sidecar file. See `SPEC-sessions` section 3.

**Reason:** a codec is fast only when a record is small. A verbatim ten-megabyte line
would slow every read of the file and waste disk.

**Rules out:** storing an unbounded tool result inline. Dropping the tail with no note.
