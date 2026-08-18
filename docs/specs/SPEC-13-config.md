# SPEC-13 — Layered configuration

Status: draft for sprint 2.
Owning crate: `rho-config`.
Features: F-70, F-71, F-72, F-73, F-103.

## 1. What this crate does, and what it must never do

`rho-config` reads configuration from files, the environment, and the command line.
It merges those sources in one fixed order. It resolves a credential to a `Secret`.
It hands the result to the CLI, which builds a `rho_core::SessionConfig`.

The crate holds one rule above all others. **It fails closed.** A malformed file, an
unknown key, or an unreadable file returns a typed error. It never falls back to a
default that grants more access than the file asked for. Decision D-017 shipped the
opposite shape once. `ToolKind::Other` counted as non-mutating, so a read-only policy
approved a tool whose author forgot to declare a kind. This crate must not repeat that
family of defect.

`rho-config` depends on `rho-core` for `Secret` and `SandboxMode`. It depends on
`rho-redact` for the credential-name check. It links no HTTP client and no terminal.

## 2. The merge order

Six layers feed the merge. A later layer wins over an earlier one. The list runs from
the weakest to the strongest.

1. Built-in defaults.
2. The global file, `~/.config/rho/config.toml`.
3. The project file, `.rho/config.toml`, under the session root.
4. The selected profile, from either file.
5. The environment, every `RHO_*` variable.
6. The command-line flags.

The rule for each adjacent pair is the same. The stronger layer replaces a value that
it sets. It leaves a value that it does not set. So the winner for one key is the
strongest layer that names that key.

| Pair | Winner |
| --- | --- |
| defaults and global file | the global file |
| global file and project file | the project file |
| project file and profile | the profile |
| profile and environment | the environment |
| environment and flags | the flags |

A profile is not a seventh source. It is a named block inside a file. The merge applies
the profile after both files, so a profile value beats a plain file value. A flag still
beats a profile.

### The environment is one layer, and clap does not read it too

`crates/rho-cli/src/cli.rs` binds `RHO_PROVIDER`, `RHO_MODEL`, and `RHO_LOG` with clap
`env =`, so today those variables also arrive as flag values. That counts one variable in
two layers with two precedences, which contradicts the single merge order above.

**The single source of truth is the environment layer.** The CLI drops clap `env =` for
`RHO_PROVIDER`, `RHO_MODEL`, and `RHO_LOG`. Section 2 layer 5 reads every `RHO_*`
variable, and the flags layer reads only a real command-line flag. So a variable counts
in exactly one layer.

Migration: the CLI keeps the `--provider`, `--model`, and `--log` flags, and removes the
`env = ...` attribute from each. The CLI collects the `RHO_*` variables into `Sources.env`
instead, and `ConfigLayer::from_env` turns them into the environment layer. The user sees
the same behaviour, because a flag still beats a variable, but the precedence now lives in
one place.

### A flag is set only when the user passes it

Every flag in the flags layer is optional. The flag layer sets a key only when the user
passes the flag. `crates/rho-cli/src/cli.rs` today holds `--read-only` as a plain `bool`,
so it has no unset state. Folded into the top layer, a `false` from a flag the user never
passed silently overrides a stricter file value.

So `Cli.read_only` becomes an `Option<bool>`. The flag layer sets `approval` only when
`read_only` is `Some(true)`. When the user passes no `--read-only` flag, the flags layer
leaves `approval` unset, and a stricter file value survives.

## 3. The public API

```rust
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rho_core::{SandboxMode, Secret};
use serde::Deserialize;

/// A typed configuration error. Every failure is typed. Every failure is closed.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file exists but could not be read, for example a permission fault.
    #[error("cannot read the config file {path}: {message}")]
    Read { path: PathBuf, message: String },
    /// The file is not valid TOML, or it names an unknown key.
    #[error("cannot parse the config file {path}: {message}")]
    Parse { path: PathBuf, message: String },
    /// The user asked for a profile that no file defines.
    #[error("the profile \"{name}\" is not defined")]
    UnknownProfile { name: String },
    /// A credential source failed to resolve.
    #[error("cannot resolve the credential \"{name}\": {message}")]
    Credential { name: String, message: String },
}

/// One layer of configuration. Every field is optional. A layer states only what
/// it overrides. `serde(deny_unknown_fields)` makes an unknown key an error.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct ConfigLayer {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub session_root: Option<PathBuf>,
    pub session_file: Option<PathBuf>,
    pub ephemeral: Option<bool>,
    /// A sandbox name. It parses through `SandboxMode::from_str`.
    pub sandbox: Option<String>,
    /// An approval mode name. It parses through `ApprovalMode::from_str`.
    pub approval: Option<String>,
    pub skill_paths: Option<Vec<PathBuf>>,
    pub no_skills: Option<bool>,
    /// A path to the MCP server file. See section 6.
    pub mcp_config: Option<PathBuf>,
    pub subagents: Option<SubagentLimitsLayer>,
    /// Credential sources, by name. Each value is one string, parsed in section 5.
    pub credentials: Option<BTreeMap<String, String>>,
    /// Named profiles. A profile is a nested layer.
    #[serde(default)]
    pub profiles: BTreeMap<String, ConfigLayer>,
}

/// The subagent limits, as optional file fields. It mirrors `rho_core::SubagentLimits`,
/// which is not itself `Deserialize` and carries a `Duration`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct SubagentLimitsLayer {
    pub max_depth: Option<u32>,
    pub max_children_per_parent: Option<usize>,
    pub max_live_total: Option<usize>,
    pub child_timeout_secs: Option<u64>,
}

/// How the agent approves a tool call. It maps onto the built-in policies in
/// `rho-core`. This type is new in `rho-config`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalMode {
    /// Approve every tool call. It maps to `rho_core::AllowAllPolicy`.
    AllowAll,
    /// Deny every mutating tool call. It maps to `rho_core::ReadOnlyPolicy`.
    ReadOnly,
    /// Ask a frontend before a mutating tool call. It maps to the interactive
    /// `AskPolicy` in `SPEC-16`.
    Ask,
}

impl std::str::FromStr for ApprovalMode {
    type Err = String;
    fn from_str(text: &str) -> Result<Self, Self::Err>;
}

/// One credential source. A literal value is a `Secret` from the moment it is read.
#[derive(Clone, Debug)]
pub enum CredentialSource {
    /// A value written in the file. It is a `Secret` at once.
    Literal(Secret),
    /// The whole value of one environment variable, by name.
    Env(String),
    /// A template with `${NAME}` spans, filled from the environment.
    Interpolate(String),
    /// A command and its arguments. The trimmed standard output is the credential.
    /// `pass_env` is the only allowlist of variable names the child inherits.
    Command { argv: Vec<String>, pass_env: Vec<String> },
}

/// A source of environment values. A test passes a map. Production passes the real
/// environment. So a test never reads the real environment.
pub trait EnvLookup {
    fn get(&self, name: &str) -> Option<String>;
}

/// The real process environment.
pub struct SystemEnv;

impl EnvLookup for SystemEnv {
    fn get(&self, name: &str) -> Option<String>;
}

impl EnvLookup for BTreeMap<String, String> {
    fn get(&self, name: &str) -> Option<String>;
}

/// The sources that feed one merge. The strongest source is last.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    pub global_file: Option<PathBuf>,
    pub project_file: Option<PathBuf>,
    pub profile: Option<String>,
    /// The `RHO_*` variables, by name and value. The CLI collects them.
    pub env: Vec<(String, String)>,
    /// The command-line flags, already turned into a layer.
    pub flags: ConfigLayer,
}

/// The merged and resolved configuration for one run.
#[derive(Clone, Debug)]
pub struct Config {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub session_root: Option<PathBuf>,
    pub session_file: Option<PathBuf>,
    pub ephemeral: bool,
    pub sandbox: SandboxMode,
    /// The approval mode the user stated, or `None` when the user stated none.
    ///
    /// `None` is not a permissive default. It means the frontend resolves the mode
    /// from the table in `SPEC-16` section 4, which yields `ask` where a human or a
    /// client can answer, and `read-only` where nobody can answer. A `Config` that
    /// collapsed the unset case into one static value would make the `ask` default
    /// unreachable, and it would hide a user's explicit `read-only` behind the same
    /// value. That is the `--read-only: bool` fault in a second place.
    pub approval: Option<ApprovalMode>,
    pub skill_paths: Vec<PathBuf>,
    pub discover_skills: bool,
    pub mcp_config: Option<PathBuf>,
    pub subagents: rho_core::SubagentLimits,
    /// Credential sources, by name. A value resolves through `resolve_credential`.
    pub credentials: BTreeMap<String, CredentialSource>,
}

impl ConfigLayer {
    /// Merge `over` onto `self`. A value in `over` wins. `self` fills a gap.
    pub fn merge(self, over: ConfigLayer) -> ConfigLayer;

    /// Build a layer from the `RHO_*` variables.
    pub fn from_env(vars: &[(String, String)]) -> ConfigLayer;
}

impl CredentialSource {
    /// Parse one credential value from the config file. See section 5.
    pub fn parse(raw: &str) -> CredentialSource;

    /// Resolve to a `Secret`. Never log the result. The child of a command source
    /// inherits only the `pass_env` names.
    pub fn resolve(&self, name: &str, env: &dyn EnvLookup) -> Result<Secret, ConfigError>;
}

impl Config {
    /// The built-in defaults. The weakest layer.
    pub fn defaults() -> ConfigLayer;

    /// Read one layer from a TOML file. A missing file is `Ok(None)`. An unreadable
    /// file, a malformed file, or an unknown key is an `Err`.
    pub fn read_file(path: &Path) -> Result<Option<ConfigLayer>, ConfigError>;

    /// Load, merge, and resolve. This is the one entry point.
    pub fn load(sources: &Sources) -> Result<Config, ConfigError>;

    /// Resolve one named credential to a `Secret`.
    pub fn resolve_credential(
        &self,
        name: &str,
        env: &dyn EnvLookup,
    ) -> Result<Secret, ConfigError>;
}
```

## 4. The key set for sprint 2

Every key below maps to a real field in the tree, or the row says it is new.

| Key | Type | Maps to |
| --- | --- | --- |
| `provider` | string | `Cli.provider` and `RHO_PROVIDER`. New config key. |
| `model` | string | `Cli.model` and `RHO_MODEL`. New config key. |
| `session-root` | path | `SessionConfig.session_root` and `Cli.root`. |
| `session-file` | path | The session file in `SPEC-14`. New config key. |
| `ephemeral` | bool | Ephemeral mode in `SPEC-14`. New config key. |
| `sandbox` | string | `SessionConfig.sandbox`, a `SandboxMode`, and `Cli.sandbox`. |
| `approval` | string | `Cli.read_only`, mapped to a built-in policy. New name. |
| `skill-paths` | path list | `SkillConfig.user_dirs` and `Cli.skills`. |
| `no-skills` | bool | `Cli.no_skills`. |
| `mcp-config` | path | `Cli.mcp_config` and `~/.rho/mcp.json`. |
| `subagents` | table | `rho_core::SubagentLimits`, through `SubagentLimitsLayer`. |
| `credentials` | table | New. The credential sources in section 5. |
| `profiles` | table | New. The named profiles for F-73. |

The `sandbox` value parses through `SandboxMode::from_str`, which exists today. The
`approval` value parses through the new `ApprovalMode::from_str`. Both fail closed on
an unknown name, and the message names the valid set.

**The defaults for the two security keys are stated, not left implicit.** `Config::defaults`
leaves `approval` unset, because the resolved default is not one static value. It comes
from the mode resolution in `SPEC-16`: rho defaults to `ask` where a human or a client
can answer, and to `read-only` where nobody can answer. A config `approval` value narrows
that resolved default, and widens it only when the user states it. The `sandbox` default
is `off`, which matches `SandboxMode::default` and the CLI flag default, per decision
D-031. A sandbox that breaks a build is worse than none, so `off` is the stated default
rather than a hidden one.

An inline MCP server table is out of scope. It would pull `rho-mcp` into `rho-config`,
which adds a runtime dependency to a config crate. So `mcp-config` names the existing
server file instead. See section 9.

## 5. Credential resolution

A credential never sits in `rho-config` as a plain string longer than one parse step.
`CredentialSource::parse` reads the file value. It picks a source by a prefix.

| File value | Source |
| --- | --- |
| `sk-live-abc` | `Literal`. The value is a `Secret` at once. |
| `env:OPENROUTER_API_KEY` | `Env`. The whole value of one variable. |
| `Bearer ${TOKEN}` | `Interpolate`. A `${NAME}` span is filled from the environment. |
| `!op read op://vault/item/field` | `Command`. The output is the credential. |

A `Literal` is a `Secret` from the moment `parse` reads it. So no plain credential
string outlives the parse. The `Debug` of `Secret` prints `Secret(***)`, and `Secret`
has no `Display`. So no formatter prints a credential.

### The shell command source

A command source runs a program and reads its standard output. The trimmed output is
the credential. `rho-config` wraps it in a `Secret` before it returns.

`crates/rho-tools/src/bash.rs` strips credential-named variables from a child. A
credential command must be consistent with that, and stricter. It runs closer to the
key, so it earns a tighter rule.

- The child inherits `PATH`, `HOME`, and the names in `pass_env`. Nothing else.
- The child inherits no variable that `rho_redact::looks_like_a_secret` flags, unless
  `pass_env` names it. So `op` reads its own session token only when the file lists it.
- The child inherits no credential that `rho-config` itself resolved.
- The child gets a null standard input.
- The child has a timeout. A hung helper fails the resolution.

`bash` uses a denylist, because a shell needs a wide and open-ended set of variables. A
credential helper needs a tiny set, so an allowlist is the safer trade here. The
allowlist is the whole difference, and it is on purpose.

### The redaction rule

A resolved credential is a `Secret`. `rho-redact` is the one home for redaction, per
decision D-026. No code in `rho-config` writes a credential to `tracing`, not even at
`trace` level. Redaction is by construction. The type carries the mask, so no filter
must remember to run. This satisfies F-103.

## 6. The failure rule

`rho-config` fails closed on every bad input.

- A malformed file returns `ConfigError::Parse`. The run stops. It does not use a
  default in place of the file.
- An unknown key returns `ConfigError::Parse`. `serde(deny_unknown_fields)` is the
  mechanism. A typo in a security key must not pass unseen.
- An unreadable file returns `ConfigError::Read`. A file that the user meant to apply,
  but that the process cannot read, is a hard error, not an empty layer.
- A missing file is not an error. `read_file` returns `Ok(None)`. So a user with no
  project file still runs.

**A parse failure never falls back to a more permissive default.** This is the exact
defect that decision D-017 fixed for `ToolKind::Other`. There, an untyped value read as
the safe-looking option and opened a boundary. A config that read a broken `approval`
key as `allow-all` would be the same defect in a new place. So a broken value stops the
run, and the message names the file and the key.

## 7. Test cases

Merge order:
- `merge_prefers_the_project_file_over_the_global_file` — a key set in both resolves to
  the project value.
- `merge_prefers_a_profile_over_a_plain_file_value` — a profile value beats a file value.
- `merge_prefers_the_environment_over_a_file` — `RHO_MODEL` beats a file `model`.
- `merge_prefers_a_flag_over_the_environment` — a `--model` flag beats `RHO_MODEL`.
- `merge_keeps_a_lower_value_the_stronger_layer_omits` — a gap in the flags keeps the
  file value.

Files:
- `a_missing_file_is_ok_none` — `read_file` on an absent path returns `Ok(None)`.
- `a_malformed_file_is_a_parse_error` — invalid TOML returns `ConfigError::Parse`.
- `an_unknown_key_is_a_parse_error` — an unknown key returns `ConfigError::Parse`.
- `an_unreadable_file_is_a_read_error` — a file with no read permission returns
  `ConfigError::Read`.
- `a_broken_approval_key_stops_the_run` — a bad `approval` value is an error, and the
  run never falls back to `allow-all`.
- `a_broken_sandbox_key_stops_the_run` — a bad `sandbox` value is an error, and the run
  never falls back to a weaker mode. The security key gets the same guard as `approval`.

Profiles:
- `a_named_profile_overrides_the_base_keys` — the profile's keys win.
- `an_unknown_profile_name_is_an_error` — `ConfigError::UnknownProfile`.

Credentials:
- `a_literal_credential_resolves_to_its_value` — `Literal` resolves to the written value.
- `an_env_credential_reads_the_named_variable` — `Env` reads the variable.
- `a_missing_env_credential_is_an_error` — an absent variable is a `Credential` error.
- `an_interpolated_credential_fills_a_span` — `${NAME}` is replaced by the variable.
- `a_command_credential_reads_the_command_output` — a stub command supplies the value.
- `a_command_child_inherits_only_the_allowlist` — a secret-named variable is absent
  unless `pass_env` names it.

Redaction, the security core:
- `a_resolved_credential_never_appears_in_a_formatted_value` — the `Debug` of a `Config`
  that holds a literal credential prints `Secret(***)` and never the value.
- `a_resolved_credential_never_reaches_a_log` — a resolution at `trace` level writes no
  credential to the subscriber.

Defaults, layers, and parsing:
- `config_defaults_sets_the_stated_defaults` — `Config::defaults` leaves `approval`
  unset and sets `sandbox` to `off`, per section 4.
- `from_env_builds_a_layer_from_rho_variables` — `ConfigLayer::from_env` maps
  `RHO_MODEL` and `RHO_PROVIDER` into a layer, and reads no variable twice.
- `load_merges_and_resolves_end_to_end` — `Config::load` reads the files, applies the
  profile, the environment, and the flags, and resolves the credentials in one call.
- `approval_mode_parses_each_name_and_fails_closed` — `ApprovalMode::from_str` parses
  `read-only`, `ask`, and `allow-all`, and returns an error that names the valid set on
  an unknown name.
- `a_sandbox_key_parses_through_sandbox_mode` — a `sandbox` value parses through
  `SandboxMode::from_str`, so the config and the core share one parser.
- `system_env_reads_the_real_environment` — `SystemEnv` reads a variable the test set on
  the real process, so the production `EnvLookup` is proved.
- `the_environment_is_read_in_one_layer_only` — `RHO_MODEL` set with no `--model` flag
  reaches the config through the environment layer, and a `--model` flag beats it. The
  variable is not double-counted as a flag. This pins the precedence for the section 2
  fix.
- `an_unset_read_only_flag_does_not_override_a_file_approval` — a run with a file
  `approval` of `read-only` and no `--read-only` flag keeps `read-only`, because the
  flag layer sets `approval` only when the user passes the flag.

Every test uses `tempfile` for a file. Every test passes an in-memory `EnvLookup`, with
one exception: `system_env_reads_the_real_environment` sets a unique variable it owns and
reads it back, so its result does not change per machine. No test reads the real
`~/.config/rho` or the real `~/.rho`.

## 8. Out of scope for sprint 2

- A custom `CredentialResolver` closure for a third-party source. F-72 lists it as a
  later extension point.
- A `ConfigSchema` for a third-party key set. F-70 lists it as planned.
- A nested profile inside a profile. A profile is one flat layer.
- An inline MCP server table. See section 9.
- Reloading the config while a session runs. The config is read once at start-up.
- A model registry loaded from the file. F-14 owns that, and it is planned.

## 9. A note on MCP servers

The MCP server list lives in its own file today, `~/.rho/mcp.json`, read by
`rho-cli::extensions::read_mcp_config`. The `--mcp-config` flag points to it. So the
`mcp-config` key names that file, and `rho-config` does not parse the servers itself.

An inline server table in `config.toml` would need the `McpServerConfig` type from
`rho-mcp`. That would add `rho-mcp` as a dependency of `rho-config`. A config crate
must stay small, so this stays out. A future change may move the server list into the
main config once the type can live without its transport crate.
