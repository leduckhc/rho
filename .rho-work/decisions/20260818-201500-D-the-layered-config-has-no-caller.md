# D-the-layered-config-has-no-caller — the config crate is unreachable, and this stage does not fix it

**Question (controller, while wiring `tui-mouse`):** `rho-cli` should read the resolved
config. Where does it do that?

**Decision:** Nowhere, and this stage does not repair it. `grep -rn rho_config crates/*/src`
finds nothing. `rho-cli` declares the dependency in its manifest and calls none of it. So
no binary reads `~/.config/rho/config.toml`, no binary reads `.rho/config.toml`, and no
`RHO_*` variable reaches the session.

The interface switch reaches the product by the two paths that do exist: the `--mouse` flag
and the `RHO_TUI_MOUSE` variable, resolved by `resolve_mouse` in `rho-cli`. The config file
key `tui-mouse` parses and merges, and nothing reads the file yet.

Four feature rows drop to `partial` today: F-layered-config,
F-environment-variable-override, F-credential-resolution, and F-profile-support.

**Reason:** The wiring is not small, and it is not safe to rush. The config resolves the
session root, the approval mode, the sandbox mode, and every credential. Each one is a
security boundary, and `SPEC-config` and `SPEC-approval` both state rules about which layer
may widen a permission. A quick call inserted into `build_config` during a TUI stage would
decide those questions by accident.

This is the sixth time this project has found tested code that no caller reaches. The
others were `confine`, `ToolKind::Other`, the `Composer`, the approval panel, and the row
durations. The pattern is always the same: the unit tests pass, the feature table claims the
row, and no key or no call site connects it.

**Rules out:** Claiming the config file works. A partial wiring that reads the model and
skips the security keys. Fixing it inside this spec. Deleting the crate, because the crate
is right and the call site is missing.

**What it needs:** Its own spec, its own review of the layer order for `sandbox` and
`approval`, and a live run for each layer. The `rho-config` tests already state the merge
order, so the work is the call site and its proof, not the logic.
