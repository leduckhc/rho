# D-trust-is-provenance-not-a-field-list — a profile walked around the gate

**Question:** an untrusted project file loses `skill-paths`, `mcp-config`, and a `!command`
credential. A live probe put `skill-paths` inside `[profiles.work]` and the attacker's skill
reached the model with no `--trust-project`. See `docs/verification/profile-trust-bypass.md`.
How should the gate work?

**Decision: trust follows the value, not the field.** A value from an untrusted source is
untrusted wherever it sits, including inside a profile, and at any depth a later format adds.

The current gate is a denylist over one struct:

```rust
layer.skill_paths = None;
layer.mcp_config = None;
```

It reads as a rule and behaves as a coincidence. It holds only for the shape the author had in
mind, and `profiles` was a second shape in the same file.

## The rule

An untrusted project layer is **filtered before it joins the merge**, and the filter walks the
whole layer:

- Every powerful key is cleared at the top level and inside every profile the layer defines.
- A `!command` credential is refused at every depth, and the refusal names the path.
- A profile a project file defines may still set display keys, because those grant nothing.

The powerful set is one list in one place, and the filter is recursive, so a new nesting level
inherits the rule instead of defeating it.

## Two rules that make the next defect louder

**The powerful set is named where the field is declared, not in `load`.** A key that grants a
capability says so beside its own definition. A reviewer reading `ConfigLayer` sees which keys
are gated, and a new field that forgets to say is caught by the test below.

**The compiler holds the rule.** `strip_powerful_keys` destructures `ConfigLayer`
exhaustively, with no `..`, so a new field fails the build until somebody classifies it. That
came from an architecture review, which insisted on it before merge and was right. My first
version was a remembered list that missed `session_root`, and my second was a test that
claimed to enumerate every field and asserted four hard-coded names. Two reviews called it
decorative. A rule the compiler holds is the only one nobody can forget.

**A test asserts the set is complete.** Every field of `ConfigLayer` is either in the powerful
set or in the harmless set, and a field in neither fails the test. That is the same shape as
`every_scalar_key_merges_and_reaches_the_config`, which caught the merge list drifting, and it
is the only way a hand-kept security list stays honest.

## The environment is the wider door

`RHO_*` variables enter at a layer with no gate. A `.devcontainer/devcontainer.json`, a CI
`env:` block, and a `.envrc` all arrive with the clone, so the environment is attacker-influenced
in exactly the case the project-file gate exists for.

**Decision:** a powerful key from the environment needs the same trust as one from the project
file, when the project is untrusted. So `RHO_SKILL_PATHS`, `RHO_MCP_CONFIG`, and the new
`RHO_BASE_URL` are dropped from an untrusted project, and rho says which and why.

That is a behaviour change for anyone setting those variables in an untrusted checkout, and it
is the right one: the alternative is a gate on the front door beside an open window.

## Rules out

**Gating the profile only when the user names it.** The value is untrusted whether or not a
profile is selected. Filtering at selection time leaves the value in the merge for anything
else to read.

**Refusing a project file that defines any profile.** A profile is a useful, harmless feature
for display keys, and a blunt refusal would teach users to pass `--trust-project` reflexively,
which destroys the gate's meaning.

**Trusting a profile from a global file that a project file overrode.** `profiles.extend`
replaces on a name collision, so a project file can shadow a same-named global profile. After
this decision the shadowing value is filtered, so the collision is no longer a privilege path.
The shadowing itself stays, and rho reports it.

**Keeping the field-list gate and adding profiles to it.** That fixes today's shape and waits
for the next one. The probe found this because the rule was a list; a rule about provenance has
no list to forget.

## What this does not settle

An untrusted project file can still set `sandbox` and `approval`, and a project value beats a
global one. So a hostile checkout may weaken a sandbox a user set in their own global config,
unless the user passes the flag. That is a separate question with its own probe to run, and it
is not folded in here, because widening a security mode is a different rule from loading a
capability. It is recorded so it is not lost.
