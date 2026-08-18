# D-measured-codec-choice — A JSONL codec choice is measured, and the fast codec stays optional

**Question (controller, sprint 2):** which JSON library gives the best result for the
session log, and what does the faster library cost?

**Decision:** Measure the three candidates, then choose. `serde_json` is the default
codec. `sonic-rs` sits behind the `fast-json` cargo feature, which is off by default.
`simd-json` is rejected. The bench lives in `bench/jsonl-codec`, outside the workspace,
so the workspace carries neither extra crate. `docs/adr/20260818-014343-ADR-jsonl-codec.md` holds the
table, the command, and the platform.

**Reason:** The numbers decided it. `simd-json` is 1.9x slower than `serde_json` on the
small records rho actually writes. `sonic-rs` is only 7 percent faster on a typed
decode, but it is 2.6x faster on the untyped path that a provider uses for SSE. It costs
23 percent more binary, 81 more lines of dependency tree, and a slow fallback on a target
that is neither x86_64 nor aarch64. So the win is real for one path and the cost is real
for every build. A feature flag gives each user the trade they want.

**Rules out:** A claim that rho is fast because of a JSON crate. A codec-specific
attribute on a record type. A borrowed parse that ties a record to a read buffer. A
single measurement as proof, because the bench reports two corpora and both matter.
