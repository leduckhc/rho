# D-recorder-consumes-events — The session log is an event-stream consumer, not a field in Session


**Question (T1 architect):** where does the session log live, so it needs no change to
the existing `Session`?

**Decision:** A `SessionRecorder` folds the `AgentEvents` stream into records. It holds
a `SessionLog`. `Session` and `SessionConfig` do not change. See `SPEC-sessions` section 5.

**Reason:** every frontend already consumes the event stream, per F-event-stream and `ADR-event-model`. A
`SessionWriter` holds an open file handle, so it cannot live in a `Clone` `SessionConfig`.
The recorder keeps the format and the runtime separate, so either can change alone.

**Rules out:** embedding a writer in `SessionConfig`. Adding a fourth argument to the
session constructor, which decision D-no-four-argument-session-new deleted for a security reason.
