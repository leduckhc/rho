# ADR-001 — Plugin mechanism

Status: accepted. Sprint 1.
Deciders: the architect, per the settled decision in `BRIEF.md` section 4.2 and
`docs/features.md` F-40 to F-43.

## Context

rho must let a third party add tools, hooks, and behaviour without forking. The
project's reason to exist is resource efficiency: 50 concurrent sessions on one
machine, each far cheaper than pi's ~76.5 MB per session (measured August 2026,
jcode.sh). Any extension mechanism must not undo that gain.

Three mechanisms were on the table:
1. In-tree Rust traits, compiled into the binary.
2. Out-of-process subprocesses over stdio JSON-RPC, any language.
3. WASM modules loaded by an embedded engine.

## Decision

Ship two tiers in sprint 1:
- Tier 1: in-tree Rust traits (`Provider`, `Tool`, `Hook`). Zero process
  overhead. See `SPEC-02`, `SPEC-03`, `SPEC-04`.
- Tier 2: out-of-process plugins over stdio JSON-RPC. Any language. Crash
  isolated. See `SPEC-04`.

Do not ship WASM in sprint 1.

## Alternatives and trade-offs

### Tier 1: in-tree Rust traits

- Pro: zero runtime cost. A trait object is one pointer. No process, no
  serialisation on the hot path.
- Pro: full type safety and the smallest possible footprint.
- Con: an extension must be compiled into the binary. A third party rebuilds rho
  with their crate as a dependency.
- Verdict: the right default for a performance-first harness.

### Tier 2: stdio JSON-RPC subprocess

- Pro: any language. No rebuild of rho.
- Pro: crash isolation. A plugin fault does not take down the session.
- Pro: cheap when idle. A plugin that is not called costs one idle process, and a
  plugin can be launched lazily.
- Con: per-call serialisation and a process boundary. This is acceptable because
  tool calls are not on the token hot path.
- Verdict: the right escape hatch for language freedom and isolation.

### Tier 3: WASM — deferred

- Pro: sandboxed, in-process, language-flexible through a compile target.
- Con: binary-size cost. A general WASM engine such as Wasmtime adds several
  megabytes to the release binary. `wasmtime` with Cranelift pulls a large
  dependency tree and raises compile time and binary size well beyond the
  footprint budget in `ADR-002`.
- Con: engine memory cost. Each module instance reserves a linear-memory arena.
  A naive default reserves a large guard region per instance. With 50 sessions
  and several modules each, the resident cost fights the whole reason for the
  project.
- Con: the host-guest ABI for streaming tool output and cancellation is real work
  to design well. It duplicates what the stdio protocol already gives us.
- Con: no user demand yet that the stdio tier does not already meet.
- Verdict: WASM waits. The stdio tier covers language freedom and isolation now,
  at a known and small cost. WASM is reconsidered only when a measured need
  appears, and only with a per-instance memory budget and a binary-size budget
  set first.

## Consequences

- A third party writes a Tier-1 extension in Rust for zero overhead, or a Tier-2
  plugin in any language for isolation and no rebuild.
- The plugin schema cache (F-43) keeps the prompt prefix stable when a plugin
  connects late. See `SPEC-04` section 5.
- The core carries no WASM dependency. The footprint budget in `ADR-002` holds.
- If WASM returns, it is a third tier behind a cargo feature, off by default, so
  a build that does not use it pays nothing.
