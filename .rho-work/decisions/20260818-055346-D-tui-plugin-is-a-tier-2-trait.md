# D-tui-plugin-is-a-tier-2-trait — The terminal view surface is a Tier-2 in-tree trait


**Question.** A third party wants to extend the terminal interface. rho has a three-tier
extension model. Which tier holds a terminal view, and is a new tier needed?

**Decision.** A terminal view is a **Tier-2 extension**: a compiled Rust trait,
`TranscriptView`, in `rho-tui`. A third party implements it and registers the value with
`TuiExtensions::register_view`. This matches how `rho_core::Tool` and `rho_core::Hook`
extend rho: a trait, not a plugin format, and no registry to join. See
`docs/extending.md` and `SPEC-tui-plugins`.

**Reason.** A render runs many times in a turn. A Tier-1 stdio subprocess, from
`SPEC-hooks-and-plugins` and `ADR-plugin-mechanism`, does a JSON-RPC round trip for each
call. That would do IO on the draw path and would break the frame budget, so it fails the
`SPEC-tui` rule that a render does no IO. An in-tree trait is one pointer call and pays no
process cost. `ADR-plugin-mechanism` already names the in-tree trait as the right default
for a performance-first harness.

**What it rules out.**

- A fourth tier. The three tiers stay as they are.
- A per-frame subprocess render. A subprocess is the right home for a tool, not for a draw
  that repeats every frame.
- A plugin format or a manifest for a view. A view is a crate that returns a trait object,
  behind a cargo feature, like every other Tier-2 extension.
