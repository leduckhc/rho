# D-budget-governor — A fleet-wide budget governor, and it is the safety valve for subagents


An architect review ranked this second by value over cost, and the reasoning holds.

Two competitors meter usage per process, because both run one process per session. So
neither can cap a fleet without an outside coordinator that watches many processes and
cannot stop one mid-turn. rho holds many sessions in one address space, so a fleet-wide cap
is a shared counter checked in the hook chain that already runs around every call.

**The lead is not that rho has a cost meter.** Every harness has one. The lead is that one
limit can bind fifty sessions and every subagent beneath them, and stop the next call
rather than report the overspend afterwards.

**It is also a safety requirement, not only a feature.** `SPEC-subagents` makes a subagent cost
about 247 KB, so the cheap thing to create is not the cheap thing to run. Density without a
budget is a way to spend money in silence.

**Decision.** `SPEC-budget-governor` specifies it, with three scopes: session, tree, and fleet.
Implementation waits for `SPEC-subagents`, because a governor must cap a tree and the tree does not
exist yet.

**Two honest limits are written into the spec rather than discovered later.** The first
version reads usage from the event stream, so it is one turn late until the model request
and response hooks land as F-model-request-and-response-hooks. And a money cap binds only where a provider reports a
charge: Azure and Bedrock report none, so the governor must warn at configuration time
instead of silently failing to enforce. A cap that quietly does nothing is worse than no cap.
