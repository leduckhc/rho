# D-acp-cancelled-spelling — The ACP cancelled stop reason needs an explicit serde rename


The S2 reviewer found a real interoperability defect. The controller confirmed it
against `~/Work/Vibe/acp-docs/schema/v1/schema.json`, where `StopReason` is
`["end_turn","max_tokens","max_turn_requests","refusal","cancelled"]`.

ACP spells the value `cancelled`, with two letters `l`. The Rust variant is
`AgentStopReason::Canceled`, with one `l`. So `serde(rename_all = "snake_case")`
alone emits `canceled`, which no ACP client accepts.

**Decision:** keep the Rust variant name `Canceled`, and carry
`#[serde(rename = "cancelled")]` on it. A named test guards the attribute:
`agent_stop_reason_canceled_serialises_as_cancelled`.

**Reason:** the Rust name follows Rust convention. The wire name follows the
protocol. An attribute plus a test is cheaper than a name that reads wrong in
either place.
