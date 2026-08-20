# D-one-redaction-home — Redaction has one home, because the copies had already drifted


The S14 developer reported that four pieces of code now existed in two or more crates.
The controller checked, and two of them were security code that had **already diverged**.

**The credential denylist** existed in `rho-tools/src/bash.rs` and again in
`rho-mcp/src/transport.rs`. The two lists were identical, which is exactly what decision
D-secret-in-core saw for `Secret` shortly before those copies diverged.

**The terminal sanitiser existed three times, with three different behaviours.**

- `rho-tui` replaced each unsafe character. Safe, but it left visible rubbish:
  `red\x1b[31mtext` rendered as `red\u{fffd}[31mtext`.
- `rho-mcp` parsed and dropped a whole escape sequence, so the same input rendered as
  `redtext`.
- `rho-tools` used a regex over the message.

So the same hostile output rendered three ways, depending on which path carried it.

**Decision.** A new crate, `rho-redact`, holds `looks_like_a_secret`, `sanitize_text`,
and `sanitize_line`. Every other crate calls it. The behaviour kept is the best of the
three: an escape sequence is dropped whole, and any other control character is replaced,
so nothing invisible survives and nothing visible is left behind.

**Reason, quoting D-secret-in-core because the argument is the same.** A leak needs only one weak
copy. So a value that guards a secret gets one definition and one test suite. A filter
that guards a terminal is the same kind of thing.

**Verification.** The controller broke the escape filter in `rho-redact` and confirmed
that all four crates fail: `rho-redact`, `rho-tui`, `rho-mcp`, and `rho-tools`. Before
the consolidation, breaking one copy left the others silent.

**Two duplicates stay, on purpose.** The bounded line reader and the stdio JSON-RPC
engine appear in `rho-plugin` and `rho-mcp`. They are close but not the same: one speaks
rho's protocol and one speaks MCP, and the framing differs. Merging them now would
invent an abstraction to fit two callers. They are recorded here so a third caller
triggers the extraction instead of a third copy.
