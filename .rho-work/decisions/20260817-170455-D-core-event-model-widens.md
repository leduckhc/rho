# D-core-event-model-widens — The architect may widen the core event model to fit ACP


The S2 architect added `ToolKind`, `Tool::kind()`, `ToolSpec.kind`, and
`AgentStopReason` on `AgentEnd`, mapped one to one onto the ACP `StopReason`.
This is approved. The remaining ACP concepts, which are plan, locations,
structured diff, and cost, stay on the `rho-acp` side and do not enter
`rho-core`.
