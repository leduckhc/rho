# D-plugin-trust-policy — The plugin host states a trust policy, and refuses a plugin in the session root


A security audit reported that `PluginHost::launch` did no path validation. The
controller had recorded it as open. This closes it.

**The risk is concrete.** If a plugin may live inside the session root, then a
repository hands executable code to the agent that reads it. A checked-in script
becomes a tool as soon as somebody points rho at that repository. Worse, the model can
write such a script itself, with `write` or `bash`, so a later launch runs code the
model authored.

**Decision.** `PluginHost::new` takes a `PluginPolicy`. The policy refuses:

- a path that does not resolve, is not a file, or is not executable;
- a plugin under `untrusted_root`, which a caller sets to the session root;
- a world-writable plugin, since another local user could replace the file first.

The check resolves the path before comparing, so `..` and a symlink cannot dodge the
root test.

**No `Default`, and no policy-free constructor.** A default would have to choose a
policy, and the only context-free choice is the permissive one. That is the shape
decision D-no-four-argument-session-new removed. `PluginPolicy::trust_any_path` exists for a caller that already
controls the path, and it is named so the call site admits what it does.

**Sequencing note.** The CLI does not wire plugins yet, so no production caller had to
change. The policy therefore lands before the caller exists, which is the right order.
`SPEC-hooks-and-plugins` section 5a records that the CLI must pass
`PluginPolicy::confined_to_outside(session_root)` when it does wire them.

**Verification.** Six tests cover the refusals and one covers ordinary use. The
controller disabled the policy check and confirmed all six fail, then restored it.
