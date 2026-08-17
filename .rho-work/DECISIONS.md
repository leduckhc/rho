# Controller decisions

Decisions the controller made during the sprint. These are binding. They answer
questions that subagents raised. Do not re-open them.

## D-001 — Session file format is rho's own, not pi's

**Question (S1 documenter):** should the session log be compatible with pi's
session format, so `makit` can switch tools without a session migration?

**Decision:** No. rho defines its own append-only JSONL session format, with a
version field in the first record.

**Reason:** pi's format encodes pi's own message and content model. If rho
adopts it, pi's decisions leak into `rho-core`, and the core stops being free to
normalise providers its own way. That breaks the reason the project exists.

**Mitigation:** a one-way converter is a `planned` feature in a separate crate,
`rho-session-import-pi`. It is out of scope for sprint 1. Record it in
`docs/features.md`.

## D-002 — The headless frontend is ACP, and it is the real ACP

**Question (S1 documenter):** should public docs say "ACP" or "RPC"?

**Decision:** Say **ACP**, and mean the real Agent Client Protocol. `rho-acp`
implements the same protocol that `pi-acp` and Zed speak. Do not invent a
private RPC dialect for sprint 1.

**Reason:** `makit` already drives `pi` over ACP. If `rho-acp` speaks real ACP,
`makit` can use rho as a drop-in session backend with no change in `makit`. That
is the shortest path to the owner's actual goal, which is 50 cheap concurrent
sessions.

**Reference:** the ACP protocol documents and JSON schema are already on disk at
`~/Work/Vibe/acp-docs/`. Use `~/Work/Vibe/acp-docs/schema/` as the source of
truth. Do not guess a method name.

**Scope:** `rho-acp` is `planned` for delivery after the TUI in sprint 1. Its
spec is still written in S2, because the spec constrains `rho-core`'s event
model.

## D-003 — `docs/benchmarks.md` is created by S11 and nothing may pre-empt it

Any document that cites a rho performance number must write `to be measured` and
link to `docs/benchmarks.md`. The website must not publish a rho number until
S11 fills that file with a real measurement and the command that produced it.
