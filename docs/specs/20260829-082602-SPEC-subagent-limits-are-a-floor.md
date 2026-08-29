# SPEC-subagent-limits-are-a-floor — the `[subagents]` table reaches the agent

Status: delivered. Driven for real: `docs/verification/config-credentials-and-limits.md`.

Owner crates: `rho-config` owns the merge, the ceiling, and the narrowing. `rho-cli` owns
the one call that turns a `Config` into `SubagentLimits`.

Decisions: `D-a-project-file-only-lowers-a-limit`, and `D-your-settings-are-a-floor` which
it applies. `D-the-layered-config-has-no-caller` parked this work, and this spec is it.
`D-cli-depth-is-zero` fixes the depth the CLI can honour.

## 0. The problem

`rho-config` builds `Config.subagents` and no binary reads it.

```
$ grep -rn "\.subagents" crates --include=*.rs | grep -v rho-config
(nothing)
```

So `subagent_limits` in `crates/rho-cli/src/cli.rs` reads flags only, and a `[subagents]`
block in a config file is silently inert. `docs/guide/configuration.md` and
`docs/guide/subagents.md` both say so, in a warning box each.

A second defect sits beside it. `ConfigLayer::merge` replaces `subagents` wholesale, so a
project table that names one limit erases every limit the global file set. That is the same
shape as the credentials defect C4 already fixed.

## 1. The sides

| Side | Crate | What it owns |
| --- | --- | --- |
| The merge | `rho-config` | joining two `[subagents]` tables, field by field |
| The ceiling | `rho-config` | which layers may raise a cap, and which may only lower one |
| The call | `rho-cli` | turning `Config.subagents` plus the flags into `SubagentLimits` |
| The notice | `rho-cli` | telling the user which limit rho refused to raise |

The contract kinds this change touches: the public API, the data model, the configuration,
and the behaviour rules. It touches no wire format and no persisted format, because a config
file's shape is already frozen by `SPEC-config` and this spec adds no key.

## 2. The contract

New and changed API in `rho-config`. Written as compilable Rust, before either side starts.

```rust
/// The subagent limits, as optional file fields.
///
/// It mirrors `rho_core::SubagentLimits`, which is not `Deserialize` and carries a
/// `Duration`.
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct SubagentLimitsLayer {
    pub max_depth: Option<u32>,
    pub max_children_per_parent: Option<usize>,
    pub max_live_total: Option<usize>,
    pub child_timeout_secs: Option<u64>,
}

impl SubagentLimitsLayer {
    /// Merge `over` onto `self`, field by field.
    ///
    /// A field set in `over` wins. A field absent from `over` keeps the value in `self`, so
    /// a table that names one limit does not erase the others. A whole-table `.or()` did
    /// erase them, which is the second defect of section 0.
    ///
    /// Both this and `narrow_to` open with an exhaustive `let Self { .. }` destructure, with
    /// no `..` and no `_`. So a fifth limit field fails the build until both are updated.
    /// A review named the alternative a blocker: two hand-kept parallel field lists let a
    /// forgotten arm drop a limit in `merge`, or leave a limit that never narrows. That is
    /// the exact shape of defect C4. `strip_powerful_keys_to_depth` already uses this
    /// technique, and the reason is the same.
    pub(crate) fn merge(self, over: SubagentLimitsLayer) -> SubagentLimitsLayer;

    /// Lower each limit to `ceiling`, and name each limit that asked for more.
    ///
    /// A limit is a bound, so the stricter value is the smaller one. A value at or below
    /// the ceiling is kept untouched. A value above it becomes the ceiling. An unset field
    /// stays unset, because an unset field states nothing and must not pin the ceiling into
    /// the layer.
    ///
    /// The returned names are config key names, such as `subagents.max-live-total`, because
    /// the user reads a key name and not a Rust field name.
    pub(crate) fn narrow_to(&mut self, ceiling: &rho_core::SubagentLimits) -> Vec<&'static str>;
}

impl ConfigLayer {
    /// Lower every subagent limit in this layer, and in every nested profile, to `ceiling`.
    ///
    /// The recursion is the one `strip_powerful_keys` uses, and it carries the same
    /// `MAX_PROFILE_DEPTH` bound, for the same reason: a project profile once carried a
    /// powerful key past a gate that only read the top layer. See
    /// `docs/verification/profile-trust-bypass.md`.
    pub(crate) fn narrow_limits(
        &mut self,
        ceiling: &rho_core::SubagentLimits,
    ) -> Vec<&'static str>;
}
```

One new field on `Config`. It is a report, not a value, so no consumer has to read it.

```rust
pub struct Config {
    // ... every existing field stays ...
    /// Every subagent limit a project file asked to raise, with the file that asked.
    ///
    /// A project file may lower a cap and never raise one. A silent refusal to obey a file
    /// is its own confusion, so the caller names each one. See
    /// `D-a-project-file-only-lowers-a-limit`.
    pub lowered_limits: Vec<String>,
}
```

Changed API in `rho-cli`, private to the binary, and stated here because it is the call site.

```rust
/// The subagent limits for this run: the merged configuration, then the flags.
///
/// `loaded.subagents` already holds the built-in defaults, the global file, and the
/// narrowed project file. A flag beats all of them, because a flag is layer 6.
///
/// `max_depth` is the one limit the CLI bounds itself. `rho-cli` captures the parent tool
/// set before `spawn_agent` joins it, so a grandchild cannot exist and a depth above 1 is a
/// promise the CLI cannot keep. A depth **below** 1 is honoured, because it is stricter, and
/// `max-depth = 0` forbids spawning. See `D-cli-depth-is-zero`.
///
/// **`loaded.subagents.queue_wait` is not authoritative, and this function must not read
/// it.** `build_subagents` fills every field from `SubagentLimits::default()`, so
/// `queue_wait` is always 600 seconds and can never say "the user set nothing". The
/// deadline follows the **resolved** `child_timeout`, so a config `child-timeout-secs = 30`
/// gives a waiter 30 seconds of patience and not ten minutes. A review named the naive
/// shape: `.unwrap_or(loaded.subagents.queue_wait)` passes every per-limit test and breaks
/// behaviour rule 6 in silence.
fn subagent_limits(cli: &Cli, loaded: &rho_config::Config) -> rho_core::SubagentLimits;
```

### The error taxonomy

No new error. Nothing here fails the run.

| Case | Result | Which side reports it |
| --- | --- | --- |
| a project file lowers a cap | the file value applies | nobody, it is normal |
| a project file raises a cap | the ceiling applies, and rho names the limit | `rho-cli`, one notice |
| a global file raises a cap | the file value applies | nobody, a home directory is not a clone |
| a flag raises a cap | the flag value applies | nobody, the user typed it |
| an unknown key in `[subagents]` | `ConfigError::Parse`, by `deny_unknown_fields` | `rho-config` |
| a value that is not an integer | `ConfigError::Parse`, by `toml` | `rho-config` |

### What the contract forbids

- No caller reads `Config.subagents` and then re-reads a file. `Config` is the answer.
- No new config key. The table has four keys, and this spec adds none.
- No trust branch on the narrowing. `--trust-project` loads a capability, and a limit is not
  a capability. See `D-a-project-file-only-lowers-a-limit`.
- No whole-table replacement in `merge`. A layer that names one limit states one limit.
- No silent clamp. Every limit rho refused to raise is named in a notice.
- No limit read from the environment. `ConfigLayer::from_env` maps no `[subagents]` key
  today, and this spec adds none.

### The extension point

A caller that is not `rho-cli` sets `rho_core::SubagentLimits` itself, exactly as
`F-four-subagent-limits` already says. The config path is one such caller, and it adds no
new trait and no new hook.

A fifth limit key arrives as one field on `SubagentLimitsLayer`, plus one arm in `merge` and
one in `narrow_to`. `narrow_to` returns the names it lowered, so a forgotten arm shows up as
a limit that never narrows, and the per-limit tests of section 4 are what catch it.

## 3. Behaviour rules

1. The ceiling is the built-in defaults merged with the global file. Nothing else raises it.
2. The project file, and every profile the project file defines, is lowered to the ceiling.
3. A flag beats the merged configuration, for every limit.
4. An unset field in a layer states nothing, so it never overwrites a set field.
5. `rho-cli` takes the smaller of the merged `max_depth` and 1.
6. `queue_wait` follows the resolved `child_timeout` when `--queue-wait-secs` is unset, so a
   config `child-timeout-secs` moves the deadline with it. `Config.subagents.queue_wait` is
   never read, because it cannot say "unset".
7. rho names every limit it refused to raise, once, in a startup notice.
8. Loading twice with the same inputs gives the same limits.

## 3a. What a review found, and what stays open

A contract review before any code found three holes. Two are fixed above. The third is real,
and it is bigger than this spec.

**A project profile replaces a global profile of the same name.** `ConfigLayer::merge` ends
with `self.profiles.extend(over.profiles)`. So a clone that writes `[profiles.prod]` replaces
the user's own `[profiles.prod]` whole, and every key the user set in it is lost. For a limit
that is safe, because a project profile is narrowed to the ceiling first, so nothing widens.
For `approval` and `sandbox` it is not safe: a user whose own `prod` profile sets
`approval = "read-only"` loses it in a hostile clone, and the run then approves every write.

**This spec does not fix it, and says so rather than hiding it.** The fix changes what a
profile of the same name means, which is a `SPEC-config` question with its own review, and it
would touch every profile test. `a_project_profile_shadowing_a_global_profile_name_cannot_raise_a_limit`
pins the limits half here, so the narrowing cannot regress while the wider question waits.

## 4. Test cases

Named, with the assertion each one proves. One test per limit, not one for the set, because
a per-field bug hides inside a set-shaped assertion.

### The merge and the ceiling, in `rho-config`

- `a_project_subagents_table_does_not_erase_a_global_limit` — a global `max-live-total` and
  a project `max-children-per-parent` both survive. The whole-table `.or()` dropped one.
- `a_config_file_lowers_the_depth_limit` — `max-depth = 0` reaches `Config.subagents`.
- `a_config_file_lowers_the_children_limit` — `max-children-per-parent = 2` reaches it.
- `a_config_file_lowers_the_live_limit` — `max-live-total = 3` reaches it.
- `a_config_file_lowers_the_child_timeout` — `child-timeout-secs = 30` reaches it.
- `a_project_file_cannot_raise_the_depth_limit` — the ceiling stands, and the key is named.
- `a_project_file_cannot_raise_the_children_limit` — the same, per limit.
- `a_project_file_cannot_raise_the_live_limit` — the same, per limit.
- `a_project_file_cannot_raise_the_child_timeout` — the same, per limit.
- `a_global_file_may_raise_a_limit` — a home directory is not a clone.
- `a_project_file_may_lower_a_limit_the_global_file_raised` — the two rules compose, so a
  ceiling above the default still bounds the project file.
- `a_project_profile_cannot_raise_a_limit` — the recursion, and the bypass that a live probe
  found for the powerful keys.
- `a_project_profile_shadowing_a_global_profile_name_cannot_raise_a_limit` — the likely
  vector, because the user types `--profile prod` from habit. See section 3a.
- `a_global_profile_may_raise_a_limit` — the ceiling excludes the applied profile, so the
  user's own profile is still their own choice.
- `trust_does_not_let_a_project_file_raise_a_limit` — `--trust-project` loads a capability
  and does not lift this rule.
- `an_unset_limit_field_stays_unset_after_narrowing` — narrowing must not pin the ceiling
  into a layer. A pinned field would name a limit the user never set in the notice, and it
  would beat a later layer that states nothing.
- `a_refused_raise_is_named_in_the_config` — `lowered_limits` holds the key and the file.
- `loading_twice_gives_the_same_limits` — rule 8.

### The call site, in `rho-cli`

- `a_config_file_alone_lowers_the_children_limit` — through `subagent_limits`, per limit.
- `a_config_file_alone_lowers_the_live_limit` — per limit.
- `a_config_file_alone_lowers_the_child_timeout` — per limit.
- `a_config_file_alone_forbids_spawning_with_a_zero_depth` — `max-depth = 0` survives the
  CLI's own bound, because 0 is stricter than 1.
- `a_config_depth_above_one_is_clamped_to_one` — the CLI cannot make a grandchild, so it
  never promises one.
- `the_children_flag_beats_the_config_file` — per limit.
- `the_live_agents_flag_beats_the_config_file` — per limit.
- `the_child_timeout_flag_beats_the_config_file` — per limit.
- `a_config_child_timeout_moves_the_queue_wait` — rule 6, so a shorter child does not leave
  a waiter with ten minutes of patience. The timeout comes from a **config file** and not a
  flag, because that is the path the naive implementation breaks.
- `a_flag_only_limit_still_comes_from_the_flag` — the six limits with no config key are
  untouched by this change.
- `a_refused_raise_reaches_a_notice` — rule 7, through `wiring_notices`.

## 5. Out of scope

- **No new config key.** `grace-turns`, `max-tool-calls`, `max-queued-per-parent`,
  `max-queued-total`, `queue-wait-secs`, and `max-steer-message-bytes` have flags and no
  key. Six new keys is a new contract with its own review.
- **No environment variable for a limit.** `RHO_SUBAGENTS_*` does not exist, and adding a
  nested table to `from_env` is its own design.
- **No sandbox or approval floor.** `D-your-settings-are-a-floor` also covers those two, and
  each needs the live probe that decision names. This spec covers the limits only.
- **No depth above one in `rho-cli`.** `D-cli-depth-is-zero` owns that, and lifting it needs
  a spawn tool the child may hold.
- **No trust store.** `--trust-project` is a flag for one run.
