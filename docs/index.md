# rho documentation map

One row per document. Find the document that answers your question.

| Document | Who reads it | Why |
|----------|-------------|-----|
| `docs/index.md` (this file) | Everyone | Find the right document fast. |
| `docs/features.md` | Architect, developer, product owner | Every planned feature with ID, status, owning crate, and extension point. The primary contract for all later stages. |
| `docs/extending.md` | Extension author | The three tiers: core tools, capability loaders, and extensions. How to add a tool and a hook, and the planned extension catalogue. |
| `docs/architecture.md` | Architect, developer | Crate dependency graph, request/response data flow, session lifecycle. Read before writing any code. |
| `docs/comparison.md` | Product owner, architect | What rho takes from pi, jcode, and agentsdk.build, and what it deliberately drops. |
| `docs/non-goals.md` | Everyone | What rho will not do, and the reason. Read before proposing a new feature. |
| `docs/specs/` | Developer, tester | Per-feature implementation specs with trait signatures and test names. Written in stage S2. |
| `docs/adr/` | Architect, reviewer | Architecture decision records. One record per settled decision. Written in stages S2 onward. |
| `docs/benchmarks.md` | Product owner, devops | Measured binary size, resident memory per session, and time to first frame. Written in stage S11. |
| `docs/release-checklist.md` | Controller, devops | The repository is private until the first release. This file lists the steps that make it public. |
| `docs/verification/sprint-1.md` | QA, product owner | End-to-end smoke test results for sprint 1. Written in stage S9. |

## Repository layout

Moved here from `AGENTS.md`, which keeps the development flow and the binding rules.

| Path | Contents |
| --- | --- |
| `crates/` | All Rust crates. |
| `docs/` | Internal docs and the feature catalogue. |
| `docs/specs/` | Numbered specs. A spec defines the public API verbatim. |
| `docs/adr/` | Architecture decision records. |
| `web/` | The `getrho.dev` static site. |
| `bench/` | Footprint and start-up measurement scripts. |
| `workflow.yaml` | The sprint workflow, stage artifacts, and definition of done. |
| `.rho-work/` | Controller notes and the progress ledger. Not shipped. |
