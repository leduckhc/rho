# SPEC-config-call-site — the config files reach the product

Status: the call site is built and driven for real. See
`docs/verification/config-call-site.md`. Rules 4, 7, and 8 stay unbuilt while question U4
of `.rho-work/reasoning-task.md` is open, and item I15, the provider credentials, is next.

Owner crates: `rho-config` owns discovery and the merge. `rho-cli` owns the one call.

Decisions: `D-the-config-call-site-lands` (supersedes `D-the-layered-config-has-no-caller`),
`D-a-bad-reasoning-mode-is-refused`, `D-project-skill-needs-trust` (inconsistent, see the
risk section).

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

/// A credential the project file asked to run as a command, without trust.
///
/// It is a variant and not a dropped value, because dropping it would hand the provider
/// an empty key and a 401. It fails when it is resolved, and the message names
/// `--trust-project`.
pub enum CredentialSource {
    // ... the existing kinds stay ...
    RefusedProjectCommand { path: std::path::PathBuf },
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
| unknown profile | `ConfigError::UnknownProfile` | `rho-config` |
| bad enum value | `ConfigError::Parse`, fail closed | `rho-config` |
| bad boolean | `ConfigError::Parse`, fail closed | `rho-config` |
| absent credential | `ConfigError`, never an empty string | `rho-config` |
| untrusted project command | `ConfigError`, naming `--trust-project` | `rho-config` |
| no working directory | `anyhow`, names the cause | `rho-cli` |
| missing file | not an error, `Ok(None)` | `rho-config` |

One new `CredentialSource` variant, and no new error type.

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
- `the_flag_beats_the_config_and_the_environment` — the full precedence chain, which
  replaces `the_flag_wins_over_the_env_var`.
- `an_unset_flag_does_not_beat_a_file` — a `bool` flag the user never passed writes no
  value into layer 6.
- `the_sandbox_flag_default_does_not_beat_a_file` — the enum half of rule 6, and the
  fail-open the review found.
- `flag_layer_maps_each_passed_flag` — a passed `--provider`, `--model`, `--sandbox`, and
  `--reasoning` each land in the right field. A wrong mapping is otherwise silent.
- `every_scalar_key_merges_and_reaches_the_config` — the completeness guard for the
  hand-kept `merge` list, so a forgotten line fails a test instead of dropping a value.
- `from_paths_maps_the_two_paths` and `the_builder_carries_env_profile_and_flags` — the one
  constructor the whole contract rests on. `from_paths_maps_the_two_paths` gives **both**
  files the same key, so a swap of the two slots reverses the winner and fails the test.
  An earlier version gave each file a different key, and a deliberate swap passed it,
  because both values still reached the config.
- `a_broken_project_file_stops_the_run` — a non-zero exit, and the path is in the message.
- `a_missing_config_file_is_not_an_error` — the run proceeds with defaults.
- `an_unknown_profile_is_an_error` — `--profile` names a block no file defines.
- `a_profile_key_beats_a_plain_file_key` — layer 4 over layer 3, through the call site.
- `the_load_reports_which_files_it_read` — one line on stderr, naming each path.
- `an_absent_global_path_is_reported` — rule 4, the unset `HOME` case.
- `loading_twice_gives_the_same_config` — rule 5.
- `the_session_root_key_does_not_move_the_project_file` — no circular read.
- `a_global_session_root_does_not_change_which_project_file_is_read` — the trap the review
  found, with the stderr report of rule 7.
- `the_bootstrap_root_reads_the_session_root_variable` — the `RHO_SESSION_ROOT` branch.
- `no_clap_env_attribute_remains` — a source guard, because the second precedence is the
  defect `SPEC-config` section 2 forbids.
- `an_absent_credential_is_an_error_not_an_empty_key` — replaces `unwrap_or_default()`.

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
- `a_project_literal_credential_needs_no_trust` — only a command is gated.

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
| a `credentials` value starting with `!` | becomes `RefusedProjectCommand`, and fails when resolved |
| `skill-paths` | dropped, and reported once on stderr |
| `mcp-config` | dropped, and reported once on stderr |

Every other key keeps the owner's full-trust ruling, including `approval` and `sandbox`.
Those two are still worth a later look, because the probe rated them High and
Medium-High, but they change no code path outside rho and they need no new contract.

A literal credential, an `env:` credential, and a global-file command are not gated. A home
directory is not a clone.

The cost to an honest user is one flag, once, when their own project file uses a
shell-command credential. A global file needs no flag.

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
