# ADR-002 — Async runtime and footprint

Status: accepted. Sprint 1.
Deciders: the architect. Held to account by stage S11.

## Context

The project exists to cut per-session cost. The owner runs 50 concurrent agent
sessions on one machine. Today each session costs him over 100 MB. rho must be
far cheaper than pi (~76.5 MB resident per extra session, ~596 ms to first input,
measured August 2026 per jcode.sh). Speed and memory are product features, not
optimisations. See F-110, F-111, F-112, F-113.

Every choice here is defensible on footprint and start-up time.

## Decision

### Async runtime: tokio, feature-selected

- Use `tokio`. It is the ecosystem standard and both `reqwest` and the AWS SDK
  need it.
- Select tokio features per crate. Do not enable the `full` feature in any
  non-test target. `rho-core` uses `rt`, `rt-multi-thread`, `macros`, `sync`,
  `time`, `io-util`, `process`. Provider crates use only `rt`, `macros`, `time`,
  `sync`. Test targets may use `full`.
- The `rho-cli` binary uses `rt-multi-thread`, `macros`, and `signal`.

### Thread count

- A session does mostly IO. It does not need one worker thread per core.
- `rho-cli` builds the runtime with a small fixed worker count, default 2, set
  with `worker_threads(2)`. A machine that runs 50 sessions does not want 50
  times the core count in threads. Each thread costs a stack.
- The value is a config key so a heavy single-session run can raise it.

### Allocation strategy

- Use the system allocator in sprint 1. Do not link jemalloc or mimalloc. They
  raise binary size and resident memory for a small throughput gain that this
  workload does not need.
- Avoid per-token heap churn. The stream reducer appends to an existing `String`.
  The event channel is bounded.

### What we refuse to link

- No jemalloc or mimalloc.
- No WASM engine. See `ADR-001`.
- No `openssl`. Every HTTP client uses `rustls`, already set in the provider
  `Cargo.toml` files.
- No `tokio` `full` feature outside tests.
- No provider crate in the core. Providers are optional cargo features on
  `rho-cli` (F-113), so a minimal build links one provider only.

### Release profile

Already set in the workspace `Cargo.toml`: `lto = "fat"`, `codegen-units = 1`,
`panic = "abort"`, `strip = "symbols"`. `panic = "abort"` removes unwind tables
and shrinks the binary. The code must therefore not rely on unwinding; a panic
aborts the process.

## Target budget

These are targets, not measurements. Stage S11 measures the real numbers and
records them, with the command, in `docs/benchmarks.md` (decision D-003). This
ADR states no measured rho number.

| Budget | Target | Command that measures it (S11) |
| --- | --- | --- |
| Extra resident memory per added session | under 15 MB | `bench/footprint.sh` reads PSS from `/proc/<pid>/smaps_rollup` on Linux, or `footprint` on macOS, for N sessions and divides the delta |
| Time to first frame (TUI) | under 100 ms | `bench/footprint.sh` times from process start to the first `ratatui` draw |
| Time to first input | under 100 ms | `bench/footprint.sh` times from process start to the input editor ready |
| Release binary size, minimal build | under 15 MB | `cargo build --release -p rho-cli --no-default-features --features minimal` then `ls -l` and `size` |
| Release binary size, all features | under 30 MB | `cargo build --release -p rho-cli --all-features` then `ls -l` |

The targets beat pi comfortably. They are goals for S11 to verify, not claims.

## Consequences

- The core stays lean and links no HTTP or TLS. A library user pays only for the
  crates they add.
- A minimal `rho` build is one provider, no plugins, no extra frontend. It is the
  smallest and fastest build.
- The fixed small worker count keeps thread stacks bounded at 50 sessions.
- If a target is missed in S11, the fix is a code or feature change, not a change
  to this ADR's intent. S11 records the measured number and the command either
  way.
