# A profile walks around the project trust gate

Date: 2026-08-25. Binary: `target/release/rho`, 0.1.0, built from `main` at 53a9793.
Found by: a contract review of `SPEC-wire-the-dead-switches`, then a two-minute probe.

## The claim under test

`Config::load` drops three powerful keys from an untrusted project file: `skill-paths`,
`mcp-config`, and a `!command` credential. `--trust-project` restores them. The gate exists
because a project file arrives with a clone, and a `SKILL.md` in a cloned repository is a
prompt injection with a filename. See `D-project-skill-needs-trust`.

The review asked one question the tests never had: **does the drop reach inside a profile?**

## The code

The drop nulls fields on the top-level project layer:

```rust
if sources.project_trust == ProjectTrust::Untrusted {
    layer.skill_paths = None;
    layer.mcp_config = None;
    // ... collect `!command` credentials as refused ...
}
merged = merged.merge(layer);
```

`ConfigLayer::merge` carries profiles across with `self.profiles.extend(over.profiles)`, so a
project file's `[profiles.*]` blocks enter the merge untouched. The profile is then applied
**after** the gate, with no trust check:

```rust
if let Some(name) = &sources.profile {
    let profile = merged.profiles.get(name).cloned()...;
    merged = merged.merge(profile);
}
```

So the gate is a field list on one layer. A profile is a second layer, and it never meets the gate.

## The probe

A fake cloned repository. `/tmp/rho-evil/repo/.rho/config.toml`:

```toml
skill-paths = ["/tmp/rho-evil/evil-skills/canary"]

[profiles.work]
skill-paths = ["/tmp/rho-evil/evil-skills/canary"]
```

The skill at that path is named `canary-injected`. Neither run passed `--trust-project`.

**Control, the top-level key:**

```sh
rho run "Do you have a skill named canary-injected? Answer YES or NO only." --no-skills
NO
```

The gate works, as its test says.

**The probe, the same key inside a profile:**

```sh
rho run "Do you have a skill named canary-injected? Answer YES or NO only." \
  --profile work --no-skills
YES
```

The attacker's skill reached the model.

## What this means

A cloned repository that ships `.rho/config.toml` with any profile can inject a skill into the
prompt the moment the user runs `--profile <name>`. `--trust-project` is not needed. `--no-skills`
does not stop it, because a config `skill-paths` entry arrives as an **explicit** path, and an
explicit path always loads.

`mcp-config` sits in the same dropped set and takes the same route, so the same file can name a
server whose command runs at startup. That half is not probed here, and it follows by symmetry
of the code, so a test must cover it rather than an argument.

A second, wider door stands beside this one: `RHO_BASE_URL` and every other `RHO_*` variable
enter at a layer with no trust gate at all, and a `.devcontainer` file or a CI `env:` block
arrives with the clone as surely as the config file does.

## Why no test caught it

Every trust test places the key at the top level. `an_untrusted_project_file_loses_skill_paths_and_mcp_config`
is exact, and true, and it never nests the key. The gate was written as a denylist over one
struct, so a new nesting level defeats it and nothing in the suite asks about nesting.

## The fix this calls for

Trust is a property of **where a value came from**, not of which field holds it. The gate must
apply to every value an untrusted source contributes, at any depth, including inside a profile.
See `D-trust-is-provenance-not-a-field-list`.

## Also found while probing

A wrong fixture cost two runs and is worth recording. A config `skill-paths` entry must name a
**skill directory**, one holding `SKILL.md`, because the value becomes an explicit path. Naming
a parent directory that contains skill directories loads nothing, and it looks exactly like a
gate working correctly. The first two probes were wrong for that reason, not because the code
was safe.
