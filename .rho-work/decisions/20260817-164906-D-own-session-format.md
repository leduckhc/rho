# D-own-session-format — Session file format is rho's own, not pi's


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
