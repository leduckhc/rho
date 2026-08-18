# D-cancel-keeps-the-session-open — Cancel reuses the existing CancelToken and keeps the session open


**Question (T1 architect):** does cancel need a new mechanism, and what does it write?

**Decision:** Cancel reuses `CancelToken` in `crates/rho-core/src/cancel.rs`. It stops
the turn and keeps the session open and usable. It writes the assistant message so far,
a synthetic error result for any unmatched tool call, and one `Stop` record with
`Canceled`. So a cancelled turn leaves no half-written tool pairing.

**Reason:** cancel is not close. A second cancellation mechanism would duplicate a type
the tree already tests. A half-written tool pairing on disk would break a later resume.

**Rules out:** a second cancellation type. A cancel that ends the session. A `ToolCall`
on disk with no matching `ToolResult`.
