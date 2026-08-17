# rho documentation map

One row per document. Find the document that answers your question.

| Document | Who reads it | Why |
|----------|-------------|-----|
| `docs/index.md` (this file) | Everyone | Find the right document fast. |
| `docs/features.md` | Architect, developer, product owner | Every planned feature with ID, status, owning crate, and extension point. The primary contract for all later stages. |
| `docs/architecture.md` | Architect, developer | Crate dependency graph, request/response data flow, session lifecycle. Read before writing any code. |
| `docs/comparison.md` | Product owner, architect | What rho takes from pi, jcode, and agentsdk.build, and what it deliberately drops. |
| `docs/non-goals.md` | Everyone | What rho will not do, and the reason. Read before proposing a new feature. |
| `docs/specs/` | Developer, tester | Per-feature implementation specs with trait signatures and test names. Written in stage S2. |
| `docs/adr/` | Architect, reviewer | Architecture decision records. One record per settled decision. Written in stages S2 onward. |
| `docs/benchmarks.md` | Product owner, devops | Measured binary size, resident memory per session, and time to first frame. Written in stage S11. |
| `docs/verification/sprint-1.md` | QA, product owner | End-to-end smoke test results for sprint 1. Written in stage S9. |
