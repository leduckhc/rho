# AGENTS.md — rho

Rules for any agent or human who changes this repository.

## Product rules

- rho is the harness, unbundled. The core is a library. Frontends and providers
  are thin, optional crates.
- Speed and memory are features. Never claim a performance win without a
  measurement and the command that produced it.
- No crate in `crates/` may depend on `rho-tui`, `rho-acp`, or `rho-cli`.
- `rho-core` has no HTTP dependency and no terminal dependency.
- Keep the system prompt short. Do not write an operating manual into it.
- Keep the model prompt append-only. A stable prefix keeps the provider cache
  warm. Never edit an already-sent turn.

## Engineering rules

- TDD. A failing test lands before production logic. Red, green, refactor.
- Never edit a test to make an implementation pass. If the test is wrong, say so.
- SOLID. If you cannot explain why a change respects each of the five
  principles, it probably violates one.
- Never leave a verified bug unfixed. Fix a confirmed bug even outside the
  current diff. If the fix is unsafe or too large now, say so explicitly.
- No network access in tests. Use `wiremock` or a recorded fixture.
- No secret in a log, including at `trace` level. Redact by construction.
- Create crates with `cargo new`. Add dependencies with `cargo add`. Never
  hand-write a dependency line or invent a version number.

## Gate

All four must pass before you report work as done.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build -p rho-cli --no-default-features --features minimal
```

## Prose rules

Write prose in ASD-STE100 Simplified Technical English.

- Active voice. Simple tenses.
- One instruction per sentence.
- Sentences of 20 words or fewer.
- One word per meaning. No idioms.
- Applies to docs, comments, error messages, UI copy, and commit messages.
- Code identifiers, commands, and paths stay verbatim.

Commit messages follow Conventional Commits.

## Where things live

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
