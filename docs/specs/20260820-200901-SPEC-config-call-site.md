# SPEC-config-call-site — the config files reach the product

Status: the call site is built and driven for real. See
`docs/verification/config-call-site.md` and `docs/verification/config-credentials.md`. Rules
4, 7, and 8 stay unbuilt while question U4 of `.rho-work/reasoning-task.md` is open. Item
I15, the provider credentials, is built. See section 7.

Owner crates: `rho-config` owns discovery and the merge. `rho-cli` owns the one call.

Decisions: `D-the-config-call-site-lands` (supersedes `D-the-layered-config-has-no-caller`),
`D-a-bad-reasoning-mode-is-refused`, `D-project-skill-needs-trust` (inconsistent, see the
risk section), `D-a-provider-names-its-own-credential`,
`D-an-untrusted-clone-supplies-no-credential`.

## 0. The problem

`rho-config` merges six layers and nobody calls it. `SPEC-config` section 2 states the merge
order, `rho-config` implements it, 100 tests pass, and no config file changes anything.

Four feature rows say `partial` for one reason: the call site. F-layered-config,
F-environment-variable-override, F-credential-resolution, and F-profile-support.

The defect that exposed it is small. `Config::reasoning` is parsed, validated, and never
read, so `tui-reasoning = "full"` in a file draws nothing.

## 1. The sides

| Side | Crate | What it owns |
| --- | --- | --- |
| Discovery | `rho-config` | which two paths hold a config file |
| The merge | `rho-config` | the six layers, the parse, the failure rule |
| The call | `rho-cli` | calling it once, and reporting the result |
| The consumers | `rho-cli`, `rho-tui` | reading `Config`, never a layer |

The contract kinds this change touches: the public API, the data model, the error taxonomy,
the configuration, and the behaviour rules. It touches no wire format. It touches no
persisted format, because a config file is authored by a user and `SPEC-config` already
froze its shape.

## 2. The contract

New public API in `rho-config`. Written as compilable Rust, before either side starts.

```rust
/// Where rho looks for its two config files.
///
/// A path is returned whether or not the file exists, because discovery is pure and
/// `Config::read_file` already answers `Ok(None)` for a file that is not there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigPaths {
    /// `$XDG_CONFIG_HOME/rho/config.toml`, else `$HOME/.config/rho/config.toml`.
    /// `None` when neither variable is set. Discovery does not fail, and the caller
    /// reports the absence, because a lost global file loses a hardened setting.
    pub global: Option<std::path::PathBuf>,
    /// `<bootstrap_root>/.rho/config.toml`.
    pub project: Option<std::path::PathBuf>,
}

impl ConfigPaths {
    /// Discover both paths. `env` supplies `XDG_CONFIG_HOME` and `HOME`, so a test never
    /// reads the real home directory.
    pub fn discover(env: &dyn EnvLookup, bootstrap_root: &std::path::Path) -> ConfigPaths;
}

/// Whether the user trusts the project file's powerful keys. `--trust-project` sets it.
///
/// The default is `Untrusted`, because a project file arrives with a clone. This reuses
/// the flag and the reason of `D-project-skill-needs-trust`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProjectTrust {
    #[default]
    Untrusted,
    Trusted,
}

impl Sources {
    /// Start from the discovered paths. Each later call adds one source, so a new source
    /// is a new method and never a longer argument list. See
    /// `D-no-four-argument-session-new`.
    pub fn from_paths(paths: ConfigPaths) -> Sources;
    pub fn with_env(self, env: Vec<(String, String)>) -> Sources;
    pub fn with_profile(self, profile: Option<String>) -> Sources;
    pub fn with_flags(self, flags: ConfigLayer) -> Sources;
    pub fn with_project_trust(self, trust: ProjectTrust) -> Sources;
}

/// A credential an untrusted project file asked for, in any form.
///
/// It is a variant and not a dropped value, because dropping it would hand the provider
/// an empty key and a 401. It fails when it is resolved, and the message names
/// `--trust-project`.
///
/// It was named `RefusedProjectCommand` and gated the `!command` form alone. Section 5
/// now gates the whole table, so the name no longer lies. See
/// `D-an-untrusted-clone-supplies-no-credential`.
pub enum CredentialSource {
    // ... the existing kinds stay ...
    RefusedProjectCredential { path: std::path::PathBuf },
}
```

The `Sources` fields become `pub(crate)`. They are `pub` today, so
`Sources { ..Default::default() }` bypasses any constructor rule written as prose. A rule
the compiler does not hold is a comment.

New public API in `rho-cli`, private to the binary but stated here because it is the call
site the whole spec is about.

```rust
/// The root that locates the project file: `--root`, then `RHO_SESSION_ROOT`, then the
/// working directory.
///
/// A `session-root` key inside a file sets the root for tools. It never moves the project
/// file that was already read, because that would be circular. When the two differ, rho
/// says so on stderr, because the project file of the new root is never read.
fn bootstrap_root(cli: &Cli, env: &[(String, String)]) -> anyhow::Result<std::path::PathBuf>;

/// Turn the parsed flags into layer 6. A flag absent from the command line must not write
/// a value, so every field stays `None` unless the user passed it.
///
/// This requires a change to `Cli`. `read_only`, `mouse`, and `no_skills` become
/// `Option<bool>`, and `sandbox` becomes `Option<SandboxArg>`. Today `sandbox` carries
/// `default_value_t = SandboxArg::Off`, so `clap` always yields `Off`, and layer 6 would
/// always beat a file that set `sandbox = "strict"`. That is a fail-open in the merge whose
/// whole purpose is to prevent one.
fn flag_layer(cli: &Cli) -> rho_config::ConfigLayer;

/// Load the configuration once for this process. Every later reader takes `&Config`.
fn load_config(cli: &Cli) -> anyhow::Result<rho_config::Config>;

/// Refuse a bad reasoning mode at its own source, before the merge.
///
/// `merge` keeps a winning value and drops where it came from, so a refusal raised after the
/// merge can only say "the merged configuration". The flag and the variable are checked here
/// so each refusal still names its own source. See `D-the-merge-cannot-name-a-values-source`.
fn validate_reasoning_sources(cli: &Cli, env: &[(String, String)]) -> anyhow::Result<()>;
```

`build_config` takes `&Config` and no longer takes `&Cli`, because every value it needs now
arrives through the merge. `build_session` takes both, because `--trust-project` stays a flag:
a file cannot grant itself trust.

### The error taxonomy

No new error type. `ConfigError` already names every case, and `rho-cli` maps it to
`anyhow::Error` for `fail`, which prints one line and returns a non-zero code.

| Case | Error | Which side reports it |
| --- | --- | --- |
| unreadable file | `ConfigError::Read`, with the path | `rho-config` |
| malformed TOML | `ConfigError::Parse`, with the path | `rho-config` |
| unknown key | `ConfigError::Parse`, by `deny_unknown_fields` | `rho-config` |
| *(the three rows above are the file-read path, so `Parse` has a real path to name)* | | |
| unknown profile | `ConfigError::UnknownProfile` | `rho-config` |
| bad enum value | `ConfigError::Value`, fail closed | `rho-config` |
| bad boolean | `ConfigError::Value`, fail closed | `rho-config` |
| absent credential | `ConfigError`, never an empty string | `rho-config` |
| untrusted project command | `ConfigError`, naming `--trust-project` | `rho-config` |
| no working directory | `anyhow`, names the cause | `rho-cli` |
| missing file | not an error, `Ok(None)` | `rho-config` |

One new `CredentialSource` variant, and one new error variant.

`ConfigError::Value { key, value, message }` was added on 20260821. `Parse` needs a path,
and four merged-layer parsers passed the literal "the merged configuration" as one. The run
then printed "cannot parse the config file the merged configuration", which two live runs
found. The merge has no file to name, so the error stops pretending. See
`D-a-merged-value-error-names-no-file`.

### What the contract forbids

- No caller builds `Sources` field by field. The fields are `pub(crate)`, and the builder
  above is the only way in, so a new source cannot appear in one caller in silence.
- No caller reads a `ConfigLayer`. A layer is an input to the merge, never an answer. The
  two current `ConfigLayer::from_env` calls in `rho-cli` are deleted.
- No `clap` `env` attribute. Layer 5 belongs to the merge. `--provider`, `--model`, and
  `--log` drop theirs, and `SPEC-config` section 2 already requires this.
- No flag with a defaulted value in layer 6. See `flag_layer` above.
- No second load. `load_config` runs once, and its result is passed by reference.
- No credential read through `std::env::var`. `provider.rs` stops using
  `unwrap_or_default()`, which turned an absent key into an empty string.
- No shell command from an untrusted project file. See section 5.

### The extension point

A new provider credential needs no edit to shared code, because `credentials` is a map from
a provider name to a `CredentialSource`.

The honest limits, corrected after review:

- A new **credential source kind** touches three places: the enum, `parse`, and
  `resolve_with_timeout`. The exhaustive match makes that safe, not single.
- A new **scalar key** touches `ConfigLayer`, `merge`, `from_env`, `Config`, and `load`.
  `merge` is a hand-kept parallel list, so a forgotten line drops a stronger layer's value
  with no error. That is the silent drop `AGENTS.md` names, so section 4 adds a completeness
  guard rather than a promise.
- The call site does not change for a new key, because it reads `Config` and never a key
  list. A typed config cannot accept an unknown key and keep `deny_unknown_fields`, and
  `deny_unknown_fields` is what turns a typo into an error.

## 3. Behaviour rules

1. The load happens once, before a session is built, in both `rho run` and the TUI.
2. The order is: discover the bootstrap root, discover the paths, read the files, apply the
   profile, apply the environment, apply the flags, then parse the typed values.
3. A missing file is normal. A present but broken file stops the run.
4. rho reports which files it loaded, once, on stderr. It also reports when **no** global
   path resolved, because an unset `HOME` would otherwise drop a hardened global file and
   fall back to weaker defaults in silence.
5. Loading twice with the same inputs gives the same `Config`.
6. A flag the user did not pass writes nothing. This covers a `bool` and an enum with a
   `clap` default, which is the harder half.
7. rho reports when the resolved `session_root` differs from the bootstrap root, because the
   project file of the new root is never read.
8. Without `ProjectTrust::Trusted`, the project layer loses `skill-paths` and `mcp-config`,
   and rho says so once on stderr. A dropped capability is announced, never silent.

## 4. Test cases

Named, with the assertion each one proves.

### Discovery, in `rho-config`

- `xdg_config_home_wins_over_home` — `global` is under `XDG_CONFIG_HOME`.
- `home_supplies_the_global_path` — with no XDG variable, `global` is
  `$HOME/.config/rho/config.toml`.
- `no_home_yields_no_global_path` — `global` is `None`, and discovery does not fail.
- `an_empty_home_value_yields_no_global_path` — an exported-but-empty variable counts as
  unset, so discovery never names `/rho/config.toml` at the filesystem root. Added during
  step 7: a break that deleted the empty check passed `no_home_yields_no_global_path`,
  because that test sets no variable at all and never reaches the empty case.
- `an_empty_xdg_value_falls_back_to_home` — the empty check must not throw away a usable
  `HOME` that sits beside an empty XDG variable.
- `the_project_path_sits_under_the_bootstrap_root` — `project` is
  `<root>/.rho/config.toml`.
- `discovery_names_a_path_that_does_not_exist` — a path is returned for an absent file.

### The call site, in `rho-cli`

- `a_config_file_alone_changes_the_reasoning_mode` — the R7 defect, and it fails today.
- `a_config_file_alone_changes_the_mouse_capture` — the same defect for `tui-mouse`, which
  `D-the-layered-config-has-no-caller` names beside it. Without this, mouse ships partial.
- `the_flag_beats_the_config_and_the_environment` — the full precedence chain. It replaces
  an earlier test that compared the flag with the environment variable only.
- `an_unset_flag_does_not_beat_a_file` — a `bool` flag the user never passed writes no
  value into layer 6.
- `the_sandbox_flag_default_does_not_beat_a_file` — the enum half of rule 6, and the
  fail-open the review found.
- `flag_layer_maps_each_passed_flag` — a passed `--provider`, `--model`, `--sandbox`, and
  `--reasoning` each land in the right field. A wrong mapping is otherwise silent.
- `every_scalar_key_merges_and_reaches_the_config` — the completeness guard for the
  hand-kept `merge` list, so a forgotten line fails a test instead of dropping a value.
  It sets every key in one file, then sweeps the merged layer for a single unset field.
  A new field on `ConfigLayer` fails it until the fixture and the merge both carry the
  field. Three deliberate breaks trip it: a dropped merge line, a merge that assigns the
  wrong field, and a fixture that misses a key. See `docs/verification/config-call-site.md`.
- `from_paths_maps_the_two_paths` and `the_builder_carries_env_profile_and_flags` — the one
  constructor the whole contract rests on. `from_paths_maps_the_two_paths` gives **both**
  files the same key, so a swap of the two slots reverses the winner and fails the test.
  An earlier version gave each file a different key, and a deliberate swap passed it,
  because both values still reached the config.
- `a_broken_project_file_stops_the_run` — a non-zero exit, and the path is in the message.
- `a_missing_config_file_is_not_an_error` — the run proceeds with defaults.
- `an_unknown_profile_is_an_error` — `--profile` names a block no file defines.
- `a_profile_key_beats_a_plain_file_key` — layer 4 over layer 3, through the call site.
- `the_load_reports_which_files_it_read` — planned. Rule 7 is unbuilt. One line on stderr,
  naming each path.
- `an_absent_global_path_is_reported` — planned. Rule 4 is unbuilt. It is the unset `HOME`
  case.
- `loading_twice_gives_the_same_config` — rule 5.
- `the_session_root_key_does_not_move_the_project_file` — no circular read.
- `a_global_session_root_does_not_change_which_project_file_is_read` — planned. It is the
  trap the review found, and it needs the stderr report of rule 7.
- `a_trusted_session_root_variable_moves_the_project_root` — `RHO_SESSION_ROOT` redirects the root when `--trust-project` is set.
- `an_untrusted_session_root_variable_does_not_move_the_project_root` — without `--trust-project`, the variable is ignored.
- `an_untrusted_session_root_variable_cannot_smuggle_a_project_config` — an attacker config at the redirected root does not reach the product.

Note: `bootstrap_root` honours `RHO_SESSION_ROOT` only under `--trust-project`. This differs from the `rho-config` rule for other environment keys. Those keys are gated only when a project file was actually read. `bootstrap_root` runs before config discovery. The signal does not exist yet, because `RHO_SESSION_ROOT` is what decides where to look. Requiring `--trust-project` is the safe fix available now. The better design is to honour the redirect and then treat the found config as untrusted. That needs its own probe and is recorded as a follow-up.
- `no_clap_env_attribute_remains` — a source guard, because the second precedence is the
  defect `SPEC-config` section 2 forbids.
- `a_missing_env_credential_is_an_error` — an absent credential stops the run. It replaces
  an `unwrap_or_default()` that read an absent key as an empty key.
- `an_absent_credential_is_an_error_not_an_empty_key` — the error names the entry and the
  variable, and no empty `Secret` reaches a provider.

Added while building it, each one for a reason the list above did not hold:

- `the_environment_beats_the_config_file` — layer 5 over layer 3 with no flag. Without it the
  full chain could pass while the environment was ignored.
- `an_unknown_mode_in_a_file_is_refused_and_names_the_key` — the third source of the S3 ruling.
- `the_model_variable_still_chooses_the_model` and
  `the_provider_variable_still_chooses_the_provider` — regression guards. Dropping the clap
  `env` attribute made both variables dead while all 874 tests still passed.
- `the_model_flag_beats_the_model_variable` — the flag half of the same pair.
- `the_root_flag_beats_the_session_root_variable` and
  `an_empty_session_root_variable_is_ignored` — the two branches of `bootstrap_root` that the
  named test above does not reach.
- `a_negated_read_only_flag_writes_nothing` and `an_empty_skill_list_writes_nothing` — rule 6
  for the two cases where an absent value could still send one.
- `a_project_file_reaches_the_product` — the gate of `bea3295` only matters once a project
  file is read at all.
- `an_untrusted_project_file_loses_skill_paths` — rule 8, through the call site rather than the
  merge alone, and it proves the flag restores the key.
- `a_config_file_can_deny_a_mutating_tool` and `the_read_only_flag_beats_an_allow_all_file` —
  the `approval` key reaching a real policy, and the U3 ruling end to end.
- `an_ask_approval_mode_is_refused_here` — `approval = "ask"` has no interactive gate in this
  path, so it is refused rather than downgraded in silence.
- `every_reasoning_mode_still_resolves` — all four names, after the resolver moved.

### The project trust gate, in `rho-config`

- `an_untrusted_project_command_credential_fails_on_resolve` — the message names
  `--trust-project`, and no command runs.
- `a_trusted_project_command_credential_resolves` — the flag restores the behaviour, so the
  gate is a gate and not a wall.
- `a_global_command_credential_needs_no_trust` — the user's own home directory is not a
  clone.
- `an_untrusted_project_file_loses_skill_paths_and_mcp_config` — rule 8, and it closes the
  bypass of `D-project-skill-needs-trust`.
- `an_untrusted_project_file_still_sets_the_display_keys` — the gate is narrow, and the
  owner's full-trust ruling still holds for every other key.

Added on 20260829, when the credential row widened to the whole table. One old line asserted
the opposite, that a project literal needed no trust, so it is deleted and recorded in
`bench/deleted-tests.txt`:

- `an_untrusted_project_env_credential_cannot_name_a_victim_variable` — the attack a review
  found. An untrusted `env:AWS_SECRET_ACCESS_KEY` refuses, and no secret is read.
- `an_untrusted_project_literal_credential_is_refused` — the second attack. An attacker key
  would send the victim's whole conversation to an account the attacker reads.
- `an_untrusted_project_interpolated_credential_is_refused` — the third form, so the gate
  covers the table and not a list of prefixes.
- `a_trusted_project_literal_credential_resolves` — the flag restores every form, so the
  wider gate is still a gate and not a wall.
- `a_global_literal_credential_needs_no_trust` — a home directory is not a clone, and the
  widening did not reach the user's own file.

## 5. The trust gate, and what the probe proved

The step 9 security review tested the full-trust ruling instead of rating it. It built a
scratch crate outside the repository, parsed `"!touch /tmp/rho-trust-probe"` with the real
`CredentialSource::parse`, called `resolve`, and the marker file appeared. So the execution
path ships in the tree today, and only this call site was missing.

It fires at startup, before the first model turn, and the model does nothing. The same file
chooses the `provider`, so the attacker also chooses which credential resolves. Exactly one
command runs, and it inherits `PATH` and `HOME`, which is enough to reach `curl` and `sh`.

The review also found a bypass the owner and I had both missed. A project file that sets
`skill-paths` loads attacker skills, and `D-project-skill-needs-trust` already refuses a
project skill by default. So full trust would have made the config file a way around a gate
this repository already ships and already reads.

**The owner took the narrow mitigation on that evidence.** The gate covers three keys, and
nothing else:

| Key, from a project file | Without `--trust-project` |
| --- | --- |
| a `credentials` value, in **any** form | becomes `RefusedProjectCredential`, and fails when resolved |
| `skill-paths` | dropped, and reported once on stderr |
| `mcp-config` | dropped, and reported once on stderr |

Every other key keeps the owner's full-trust ruling, including `approval` and `sandbox`.
Those two are still worth a later look, because the probe rated them High and
Medium-High, but they change no code path outside rho and they need no new contract. A
subagent limit is now the exception, and `SPEC-subagent-limits-are-a-floor` owns it.

**The credential row widened on 20260829.** It gated a value starting with `!` alone, and it
named the other three forms safe. That was true only while nothing resolved a credential.
Section 7 makes them resolve, so a security review of the amendment found two live attacks.
`openrouter = "env:AWS_SECRET_ACCESS_KEY"` sends the victim's own secret to openrouter.ai. A
literal attacker key sends the victim's whole conversation to an account the attacker reads.
A clone chooses `provider` too, so it chooses which credential name resolves. See
`D-an-untrusted-clone-supplies-no-credential`.

A global-file credential is not gated, in any form. A home directory is not a clone.

The cost to an honest user is one flag, once, when their own project file names a credential.
A global file needs no flag, and neither does the fallback variable of section 7.

## 6. Out of scope

- No new config key. This is the call site, not a key set.
- No interactive approval prompt. `SPEC-approval` owns that.
- No MCP server list in the config file. `SPEC-config` section 9 keeps it out.
- No per-key trust table. The gate in section 5 covers three keys, by one rule, for one
  reason. Every other key stays fully trusted.
- No change to the merge **order**. The six layers of `SPEC-config` section 2 stand. The
  merge does gain the project trust gate of section 5, so the earlier claim that
  `rho-config` needs no change was wrong.
- No trust store, and no remembered trust decision. `--trust-project` is a flag for one run,
  and a persisted trust file would be a new contract with its own spec.

## 7. I15: a provider resolves its credential

This section is the amendment that closed item I15. It comes after the out-of-scope list
because the earlier sections stay as they were written, and renumbering them would break
every reference to them. `.rho-work/i15-credential-expansion.md` is the requirement
breakdown, and `D-a-provider-names-its-own-credential` is the one decision it needed.

### What was still broken

Section 2 forbids a credential read through `std::env::var`, and `provider.rs` did five of
them. Four used `unwrap_or_default()`, which turns an absent key into an empty string, so a
user with no key read a provider 401 instead of a sentence naming what to set.

```
$ grep -rn "resolve_credential" crates --include=*.rs | grep -v rho-config
(nothing)
```

### The contract

One new method on `Config`. `resolve_credential` keeps its meaning, and this one adds the
fallback the user base needs.

```rust
impl Config {
    /// Resolve a named credential, or fall back to one environment variable.
    ///
    /// A `[credentials]` entry named `name` wins. With no such entry, rho reads
    /// `fallback_var` through the same `CredentialSource::Env` path, so the project trust
    /// gate, the `Secret` type, and the error taxonomy all still hold.
    ///
    /// An absent value is a `ConfigError::Credential` naming both the entry and the
    /// variable. An empty value is the same error, because an empty key reaches the
    /// provider and returns 401, which reads as a broken account rather than a missing
    /// key. It is never an empty `Secret`.
    ///
    /// The provider builder supplies `fallback_var`, so a new provider names its own
    /// variable and edits no shared code. See `D-a-provider-names-its-own-credential`.
    pub fn resolve_credential_or_env(
        &self,
        name: &str,
        fallback_var: &str,
        env: &dyn EnvLookup,
    ) -> Result<Secret, ConfigError>;
}
```

Changed API in `rho-cli`. The argument list gets shorter, not longer, because `base_url`
already lives in `Config`.

```rust
/// Build a provider by name, and resolve its credential through the merged configuration.
///
/// It reads no environment variable directly. `env` is the lookup `rho-config` uses, so a
/// test never touches the real process environment.
pub fn build_provider(
    name: &str,
    config: &rho_config::Config,
    env: &dyn rho_config::EnvLookup,
) -> Result<Arc<dyn Provider>, ProviderError>;

/// Whether this build can use a provider name, with no credential work.
///
/// A caller that only needs to know whether a name is usable must not run a credential
/// helper to find out. The JSONL frontend is that caller.
pub fn check_provider_name(name: &str) -> Result<(), ProviderError>;
```

One new `ProviderError` variant, because a credential failure is not a missing environment
variable and the two need different words.

```rust
pub enum ProviderError {
    // ... the existing variants stay ...
    /// A credential could not be resolved. `rho-config` names the reason, and the reason
    /// may be a refusal rather than an absence, so the message is passed through whole.
    #[error("{message}")]
    Credential { message: String },
}
```

### The credential name of each provider

| Provider | Credential name | Fallback variable | Resolves a credential |
| --- | --- | --- | --- |
| openrouter | `openrouter` | `OPENROUTER_API_KEY` | yes |
| azure | `azure` | `AZURE_OPENAI_API_KEY` | yes |
| bedrock | none | none | **no** |

**Bedrock resolves no credential, on purpose.** The AWS SDK owns its own credential chain:
environment variables, a profile, the SSO cache, and IMDS. `provider.rs` reads only
`AWS_REGION` for it, and a region is not a secret. Routing the region through
`resolve_credential` would wrap a public value in a `Secret`, which has no `Display`, and it
would break the SDK chain that `F-aws-bedrock-provider` states. So line 205's region read
stays a plain environment read.

### What the amendment forbids

- No `std::env::var` in `provider.rs`. Every read goes through `&dyn EnvLookup`, so a test
  never depends on the machine it runs on.
- No `unwrap_or_default()` on a credential. An absent value is an error with a name.
- No empty `Secret` reaching a provider.
- No `Secret` in a log, an error, or a panic. `Secret` has no `Display`, and its `Debug` is
  a fixed mask, so this holds by construction and not by a filter.
- No credential seed table inside `rho-config`. See `D-a-provider-names-its-own-credential`.
- No fallback after a `Credential` error. A `RefusedProjectCredential` must stay a refusal,
  so the fallback fires **only** when no entry carries that name. A `.or_else` on the
  resolve result would turn a refusal into an environment read, and the gate would then
  teach the user nothing. This is the U3(b) shape the decision rules out.
- No credential value inside an error message. A review found one leak beside this path: a
  `base-url` holding `https://user:password@host` was echoed whole into
  `ConfigError::BaseUrl`. That message now carries the url with its userinfo replaced.
- No credential helper's stderr on rho's stderr. `resolve_command` piped stdout and left
  stderr inherited, so a chatty helper could print a key. It is now null.

### Two rules a test can pass for the wrong reason

A security review named both, and each is written here because the test design is the whole
guard.

1. **The refusal test must set the fallback variable.** With `OPENROUTER_API_KEY` unset,
   correct code and the `.or_else` bug both fail, so the test passes and proves nothing.
   `a_refused_project_credential_does_not_fall_back` sets the variable to a value it then
   asserts is **not** returned.
2. **The empty-credential test must make the fallback empty too.** Otherwise an
   implementation that skips the empty check still fails, for the wrong reason.

### The non-secret provider settings

`AWS_REGION`, `AZURE_OPENAI_ENDPOINT`, and `AZURE_OPENAI_DEPLOYMENT` are not secrets, and
U1(a) of the requirement breakdown left them as environment reads. They stay environment
reads, and they now go through the same `&dyn EnvLookup`. So a test isolates them, and no new
config key joins the contract.

### Test cases for section 7

In `rho-config`:

- `an_absent_credential_is_an_error_not_an_empty_key` — the named entry is absent, the
  fallback variable is unset, and the error names both. No empty `Secret` is returned.
- `an_empty_credential_is_an_error` — an exported-but-empty variable is not a key. The entry
  and the fallback are both empty, so the test cannot pass for the wrong reason.
- `a_credentials_entry_beats_the_fallback_variable` — the file wins, so a config file really
  chooses the key. The entry value and the variable value differ, so a swap of the two
  arguments fails it.
- `the_fallback_variable_resolves_when_no_entry_names_it` — U2(a), so no existing user
  breaks.
- `a_refused_project_credential_does_not_fall_back` — the refusal survives **while the
  fallback variable is set**, and the message names `--trust-project`.
- `resolving_a_credential_twice_gives_the_same_answer` — I9. A build happens per process and
  a helper may run more than once.
- `a_credential_error_never_holds_the_resolved_value` — the pass-through message of
  `ProviderError::Credential` cannot leak a key.
- `a_userinfo_base_url_error_hides_the_password` — the leak beside this path.
- `a_credential_command_stderr_does_not_reach_the_parent` — a chatty helper cannot print a
  key onto rho's stderr.

In `rho-cli`:

- `the_openrouter_key_comes_from_the_config_file` — a `[credentials]` entry named
  `openrouter` reaches the provider.
- `the_azure_key_comes_from_the_config_file` — one provider is not every provider.
- `a_missing_openrouter_credential_names_what_to_set` — the message names
  `OPENROUTER_API_KEY`, and it is not a 401.
- `a_missing_azure_credential_names_what_to_set` — the same, per provider.
- `an_untrusted_project_command_credential_fails_the_provider_build` — the gate runs at the
  call site, and the message names `--trust-project`.
- `bedrock_needs_no_credential_entry` — the AWS chain still owns the credential, and an
  empty `[credentials]` table does not stop a Bedrock build.
- `bedrock_still_reads_its_region_from_the_environment` — the region is not a credential.
- `no_provider_builder_reads_the_process_environment` — a source guard, so a sixth
  `std::env::var` cannot come back.
- `check_provider_name_agrees_with_build_provider` — the two name lists cannot drift.
