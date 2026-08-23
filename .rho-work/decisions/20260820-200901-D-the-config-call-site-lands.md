# D-the-config-call-site-lands — rho-cli reads both config files, and a project file is trusted

Date: 20260820

Supersedes `D-the-layered-config-has-no-caller`, which deferred this work.

## The question

`rho-config` merges six layers, refuses a bad value, and resolves a credential. No binary
called it. `grep -rn "rho_config::" crates/*/src` found two calls, both
`ConfigLayer::from_env` inside `rho-cli`, and both for one interface switch.

So `~/.config/rho/config.toml` changed nothing, `.rho/config.toml` changed nothing, and
`Config::reasoning` was parsed and thrown away. Four feature rows read `partial` because of
it. `D-the-layered-config-has-no-caller` asked for three things first: a spec, a review of
the layer order for `sandbox` and `approval`, and a live run for each layer.

## The decision

**The call site lands now.** The owner ruled it on 20260820, while finishing
`SPEC-reasoning-across-providers`. `rho-cli` discovers both files, calls `Config::load`
once per process, and reads every key. The reasoning display mode is one key of many, and
it stops being a special case.

Three sub-rulings, all from the owner on the same day:

1. **An unknown value is an error at every source.** See
   `D-a-bad-reasoning-mode-is-refused`.
2. **A project file is trusted, with one narrow gate.** `.rho/config.toml` may set every
   key, including `approval`, `sandbox`, and `session-root`. Three keys need
   `--trust-project`: a `credentials` value that runs a command, `skill-paths`, and
   `mcp-config`. See the evidence section, and `SPEC-config-call-site` section 5.
3. **The work gets its own spec.** `SPEC-config-call-site`, in this branch.

## The reason

The crate was right and the call site was missing. That is the seventh time this project
found tested code no caller reaches, and the count is the argument: a merge nobody calls is
not a feature, it is a well-tested library with no user.

Ruling 2 keeps one merge order, with no per-key exception table. The gate is one rule for
three keys, not a table, and it reuses a flag and a decision that already exist.

## The evidence that changed ruling 2

The owner first ruled full trust, with no gate. I recorded the risk and sent the contract to
the step 9 security review, with one instruction: test the severity, do not rate it.

**It proved remote code execution.** In a scratch crate outside the repository, the real
`CredentialSource::parse` turned `"!touch /tmp/rho-trust-probe"` into a command, `resolve`
spawned it, and the marker file appeared on disk. The mechanism already ships. It fires at
startup, before the first model turn, because the same file also picks the provider whose
credential resolves.

The review then found a bypass that neither the owner nor I had weighed. A project file that
sets `skill-paths` loads attacker skills, and `D-project-skill-needs-trust` already refuses a
project skill by default, for the reason that a skill can instruct the model and can carry
scripts. Full trust would have made a config file a way around a gate this repository already
ships and already reads at `extensions.rs:49`. `mcp-config` is the same shape, because it
launches server processes.

On that evidence the owner took the narrow mitigation. No code existed yet, so the change
cost one spec edit. `AGENTS.md` step 9 asks for this: a severity is a hypothesis, and a live
probe is the test.

The two keys the review rated next, `approval` and `sandbox`, stay fully trusted by the
owner's ruling. They are named here so a later reader reopens them with evidence rather than
with an opinion.

## What this rules out

- **No second precedence.** `clap` must stop reading the environment for `--provider`,
  `--model`, and `--log`. `SPEC-config` section 2 forbids it, and the merge owns layer 5.
- **No per-key trust table.** The gate is one rule for three keys: a command runs only from
  a file the user trusts. Every other key follows ruling 2.
- **No defaulted flag in the top layer.** `--sandbox` carries a `clap` default today, so
  layer 6 would always beat a file's `sandbox = "strict"`. The contract review caught it, so
  the flag fields become `Option`.
- **No silent credential.** `provider.rs` uses `std::env::var(...).unwrap_or_default()`,
  which turns an absent key into an empty string and a provider 401. A credential resolves
  through `Config::resolve_credential`, and an absent one is a named error.
- **No load per call.** The config loads once, in one place. Two loads under two working
  directories would read two different project files.
- **No TUI-only wiring.** `rho run` reads the same config. A key that works in one mode is
  the defect this repairs.
- **No circular root.** The project file is found under the bootstrap root, which is
  `--root`, then `RHO_SESSION_ROOT`, then the working directory. A `session-root` key in a
  file sets the root for tools, and never moves the file that was already read.
