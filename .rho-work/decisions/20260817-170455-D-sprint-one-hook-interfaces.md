# D-sprint-one-hook-interfaces — The Hook trait and the approval gate are sprint-1 interfaces


**Question (S2 architect):** `docs/features.md` marks F-hook-trait-tier-1 (Hook) and F-tool-approval-gate
(approval gate) as `planned`. But the S3 definition of done tests hook order,
and `20260817-170455-SPEC-acp.md` needs an async approval path for the ACP
`session/request_permission` request.

**Decision:** The **traits** ship in sprint 1. Flip F-tool-approval-gate and F-hook-trait-tier-1 to `sprint-1`.
The rich behaviour stays `planned`.

- Sprint 1 delivers: the `Hook` trait, its ordering guarantee, and one or two
  real hook points. Plus the approval callback type and its wiring in the tool
  dispatch path.
- Sprint 1 does not deliver: a full hook point at every lifecycle stage, a
  policy language, or an approval user interface beyond a TUI prompt.

**Reason:** an interface added later changes every caller. An interface added now
costs almost nothing. `rho-acp` cannot conform to ACP without an approval path,
so the type must exist in the core from the start.
