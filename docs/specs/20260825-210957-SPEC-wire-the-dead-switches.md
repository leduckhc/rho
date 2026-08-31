# SPEC-wire-the-dead-switches — six switches that reach nothing, and the guard for the class

Status: delivered, except the separator guard and the dead-surface gate entry. See section 4a.
Driven for real; see `docs/verification/wiring-sprint.md`.
Owning crates: `rho-config` and `rho-cli` (the keys), `rho-tui` (motion and the help table),
`rho-mcp` and `rho-cli` (the schema cache), `bench` (the guard).
Features: F-provider-base-url, F-agent-discovery-switch, F-reduced-motion, F-mcp-schema-cache,
F-dead-surface-guard.

Decisions this spec implements: D-trust-is-provenance-not-a-field-list,
D-a-provider-base-url-is-a-config-key,
D-skills-and-agents-are-two-switches, D-motion-answers-to-one-switch,
D-the-dead-surface-allowlist-names-its-lane. It acts on D-dead-surface-is-a-defect-class.

## 1. The problem

Writing rho's user documentation found fifteen defects. Five share one shape: **a capability
exists in code and reaches no user**, because one call is missing. A test suite cannot see it,
since the code under test works. Only a person driving the product finds it.

| Capability | The missing call | What a user sees |
| --- | --- | --- |
| MCP tools | `McpSchemaCache::save` has no caller | Every run says the tools arrive next session. They never do. |
| Reduced motion | `motion_enabled` is never consulted | The animation cannot be stopped. An accessibility gap. |
| Local model hosts | `base_url` is settable and unreachable | No way to use Ollama or vLLM, though the client speaks their format. |
| Subagents with `--no-skills` | One field governs two loaders | `spawn_agent` disappears, in silence. |
| Task progress | The task row ignores its `progress` field | A long build shows no percentage. |

This spec closes four of the five, and adds a guard for the sub-class it can see: an uncalled
capability. The guard would have caught three of the five. It cannot see the task-progress field
that the renderer ignores, nor the `--no-skills` call bound to the wrong boolean, because
neither is an uncalled function. Those two need a different guard, and this spec fixes one of
them by hand. The task-progress row was out of scope here, because it
needed a row-rendering decision this spec did not make. That decision and the row are
delivered now. See `SPEC-the-task-row-draws-its-progress`.

## 1a. What the review found, and why item zero is new

The contract went to a security review before any code, and the review found a **live defect in
shipped rho** that this spec would otherwise have built upon.

The untrusted-project gate drops `skill-paths`, `mcp-config`, and a `!command` credential from a
project file. It nulls fields on the top-level layer only, and `merge` carries `profiles` across
untouched, and the profile is applied after the gate. So the same key inside `[profiles.work]`
reaches the merge with no trust check.

A probe proved it: an attacker skill named in a project profile reached the model with no
`--trust-project`, and `--no-skills` did not stop it, because a config `skill-paths` entry
arrives as an explicit path. See `docs/verification/profile-trust-bypass.md`.

Adding `base-url` to that gate would have inherited the hole and pointed the credential at a
host of the attacker's choosing. So the gate is rebuilt first, on provenance, and every later
item in this spec depends on it.

Three more corrections the review forced:

**The motion premise was inverted.** `state.animate` is never assigned, so the sweep never
draws today. Wiring it turns motion on for the first time. See the corrected decision.

**The MCP cache write cannot live in `extensions::load`.** That function returns before any
handshake, and the pool discards the tool list in `spawn_connect`'s `Ok` arm. The write belongs
there, it must be read-modify-write with an atomic rename, and the persisted key must be hashed,
because a fingerprint embeds server `env` values and would put a token on disk in clear text.

**The guard catches three of the five defects, not five.** It finds an uncalled function. It
cannot see a field the renderer never reads, nor a live call bound to the wrong boolean. This
spec says so instead of overclaiming.

## 2. The contract

### `rho-config`: two new keys

```rust
pub struct ConfigLayer {
    // ... existing fields ...
    /// The provider endpoint. Unset means the provider's own default.
    pub base_url: Option<String>,
    /// False stops the terminal's sweep animation.
    pub tui_motion: Option<bool>,
    /// True stops the agent-definition search.
    pub no_agents: Option<bool>,
}
```

TOML names are `base-url`, `tui-motion`, and `no-agents`. Environment variables are
`RHO_BASE_URL`, `RHO_TUI_MOTION`, and `RHO_NO_AGENTS`. `RHO_REDUCE_MOTION=1` also stops the
sweep, because the code already documents that name.

`Config` gains the resolved values:

```rust
pub struct Config {
    // ... existing fields ...
    pub base_url: Option<String>,
    pub tui_motion: bool,
    pub discover_agents: bool,
}
```

`tui_motion` defaults to true. `discover_agents` defaults to true, and it is **independent of
`discover_skills`**.

### `rho-config`: the base url is refused unless it is safe

```rust
/// A base url must be `https`, or a loopback host. A plain-`http` remote host would put
/// the credential on the wire in clear text.
fn check_base_url(value: &str) -> Result<(), ConfigError>;
```

An untrusted project file loses `base-url`, beside `skill-paths`, `mcp-config`, and a
`!command` credential.

### `rho-cli`: three new flags

```
--base-url <URL>     The provider endpoint. Use it for a local model host.
--no-agents          Do not search for agent definitions.
--no-motion          Stop the sweep animation.
```

`--no-skills` stops the skill search only. It no longer touches agent discovery.

### `rho-tui`: the renderer asks before it animates

```rust
/// The conditions under which the sweep animates. Every field is a condition a caller
/// really supplies.
pub struct MotionInputs {
    pub tui_motion: bool,
    pub stdout_is_terminal: bool,
}

pub fn motion_enabled(inputs: MotionInputs) -> bool;
```

`TuiState` carries one resolved flag, `animate: bool`. `footer_line` consults it before it
sweeps. The original five-field design lost three fields that callers never set.
`RHO_REDUCE_MOTION=1` sets `tui_motion = false` in `rho-config` and reaches `MotionInputs`
through the normal config merge.

### `rho-mcp`: the cache is written

`McpPool` gained four public items: `set_cache_path`, `drain_connects`, `take_cache_notices`,
and the free function `record_tools`. The write runs in `spawn_connect`'s `Ok` arm inside
`spawn_blocking`, not in `extensions::load`. `drain_connects(timeout)` awaits the background
tasks so a short `rho run` does not kill a pending write. `take_cache_notices` returns any
write failures as strings the CLI shows to the user. No notice promises a session that cannot
differ.

### `rho-config`: new error types

`ConfigError::BaseUrl { value, reason: BaseUrlRejection }` replaces the old stringly-typed
`ConfigError::Value` for all five base-url refusals. `BaseUrlRejection` is a public enum with
five variants and an `is_safety_block()` predicate.

### `rho-cli`: new provider error

`ProviderError::IncompatibleBaseUrl { name, url }` replaces the reused `MissingConfig` for a
base-url that does not match the chosen provider.

### `bench/check-dead-surface.py`: the guard

It reports every `pub fn` in `crates/*/src` with no caller outside its own module and its
tests. `bench/allowed-uncalled.txt` holds one line per exemption:
`<path>::<function>  <reason>`. An entry with no reason fails. An entry naming a `SPEC-` slug
that does not resolve fails. An entry whose function now has a caller fails as unnecessary.

### What the contract forbids

- No key changes a security-relevant value from an untrusted project file.
- No base url sends a credential over plain `http` to a remote host.
- `motion_enabled` is the only path to the sweep decision. The renderer holds no second rule.
- The guard never counts a test as a caller.
- No new switch defaults to changing today's behaviour.

## 3. Behaviour rules

1. With no `base-url`, the endpoint is unchanged.
2. A `base-url` that is neither `https` nor loopback stops the run and names the reason.
3. A project file's `base-url` is dropped unless `--trust-project` is passed.
4. When `base-url` is set, a startup notice names the host the credential goes to.
5. `--no-skills` leaves agent discovery on. `--no-agents` leaves skills on.
6. When agent discovery is off, rho names the switch so the missing `spawn_agent` tool does not
   read as a broken feature. No count of skipped definitions is reported: `discover_agents`
   returns an empty set before it scans, so counting would require the discovery the switch
   prevents.
7. `--no-motion`, `tui-motion = false`, or `RHO_REDUCE_MOTION=1` each stop the sweep. The
   footer still names the state in words.
8. A successful MCP handshake writes the schema cache. A later session advertises those tools.
9. The refusal that names `--allow-widen` no longer names a flag that does not exist.
10. Every key in the binding table uses one separator style.

## 4. Test cases

### The base url

- `a_custom_base_url_uses_the_openai_chat_path` — the provider builds with the custom host and uses the OpenAI chat path, not the OpenRouter path.
- `the_default_base_url_keeps_the_openrouter_path` — the OpenRouter endpoint is unchanged when no custom url is set.
- `an_https_base_url_is_allowed` — an `https` url passes the safety check.
- `a_loopback_http_base_url_is_allowed` — `http://localhost:11434` passes, because the traffic never leaves the machine.
- `a_plain_http_remote_base_url_is_refused` — a plain-http remote url is refused. The error text contains `base-url`.
- `a_userinfo_host_cannot_pose_as_loopback` — `http://user@localhost` is refused as a safety block.
- `a_look_alike_host_cannot_pose_as_loopback` — a hostname that resembles `localhost` is refused.
- `an_encoded_loopback_address_is_read_by_the_parser` — the parser reads a percent-encoded loopback address.
- `a_trailing_dot_localhost_is_refused` — `localhost.` is refused.
- `the_unspecified_address_is_refused` — `0.0.0.0` is refused.
- `a_url_with_no_scheme_is_refused` — a url with no scheme is refused.
- `a_base_url_with_a_query_or_fragment_is_refused` — a query string or fragment is refused.
- `a_base_url_refusal_tells_a_safety_block_from_a_typo` — `is_safety_block()` is true for transport and userinfo refusals and false for others.
- `an_https_base_url_with_userinfo_is_refused_as_a_safety_block` — `https://user:pass@host` is refused as a safety block.
- `every_field_is_classified_as_powerful_or_harmless` — every `ConfigLayer` field is reached by the trust classifier.
- `a_trusted_project_keeps_a_powerful_environment_variable` — `--trust-project` restores a dropped environment key.
- `setting_a_base_url_names_the_host_in_a_notice` — the startup notice names the host the credential goes to.
- `a_loopback_host_bypasses_every_proxy` — a loopback base url sends its request with no proxy.
- `a_remote_host_still_honours_a_proxy` — a remote base url respects the environment proxy variable.

### Skills and agents are two switches

- `no_skills_leaves_agent_discovery_on` — skills off leaves agent discovery enabled.
- `no_agents_leaves_skill_discovery_on` — agents off leaves skill discovery enabled.
- `no_agents_stops_the_definition_search` — planned. No `spawn_agent` tool is registered.
- `agent_discovery_off_is_reported` — the notice names the switch. No count is reported, because `discover_agents` returns before it scans.
- `build_session_passes_the_agent_discovery_switch_from_the_config` — the config value reaches the session builder.
- `no_switch_means_no_wiring_notice` — a default config produces no wiring notice.

### Motion

- `motion_is_on_by_default_once_it_is_wired` — a default `App` animates.
- `the_motion_switch_stops_the_sweep` — `tui-motion = false` renders the word plain.
- `a_non_terminal_stdout_stops_the_sweep` — a non-terminal stdout renders the word plain.
- `the_renderer_draws_the_word_either_way` — the word is always present, regardless of motion state.
- `the_motion_flag_reaches_the_renderer` — `--no-motion` reaches the renderer and stops the sweep.
- `with_motion_reaches_the_renderer_through_the_builder` — `App::with_motion` is exercised through the real builder.
- `with_motion_false_is_a_still_frame_through_the_builder` — `with_motion(false)` produces a still frame.
- `with_motion_true_animates_through_the_builder` — `with_motion(true)` produces an animated frame.
- `reduce_motion_wins_over_the_motion_key` — `RHO_REDUCE_MOTION=1` sets `tui_motion` to false and wins over `RHO_TUI_MOTION`, regardless of order.

### The MCP cache

- `a_recorded_tool_list_is_read_back` — `record_tools` writes the list, and `tools_for` returns it without a new handshake.
- `a_second_server_does_not_clobber_the_first` — two servers each keep their own entry.
- `the_cache_file_holds_no_server_secret` — the persisted key is a hash; no raw `env` value appears in the file.
- `a_handshake_through_the_pool_writes_the_cache` — a full pool handshake writes the cache file on disk.
- `eight_concurrent_writers_keep_every_entry` — eight parallel writes all survive.
- `a_failed_cache_write_surfaces_a_notice` — a write failure produces a notice the CLI can show.
- `both_run_paths_drain_the_mcp_connects_before_exit` — both the run path and the headless path drain connects.
- `the_drain_awaits_connects_and_surfaces_cache_notices` — `drain_connects` awaits the background write; `take_cache_notices` returns any failure.
- `a_failed_handshake_writes_no_cache` — planned. The write sits in the `Ok` arm. A broken server leaves no false promise.
- `the_notice_does_not_promise_a_session_that_cannot_differ` — planned. A notice names only what the code can do.

### The guard

The guards are Python scripts. Their behaviour cannot be a Rust test name, and the gate would
read a name here as a missing test. They are proved by running them and by breaking them on
purpose. The runs are recorded in `docs/verification/wiring-sprint.md`:

- `check-flag-names.py` reports zero, and reports one when the old `--allow-widen` message goes back.
- `check-dead-surface.py` reports its ledger. It refuses an entry with no reason. It refuses an entry naming a spec that does not resolve.

### The two smaller defects

`check-flag-names.py` replaces the first of these two. Section 4a records why the separator guard is not in this sprint.

## 4a. What landed, and what did not

Delivered: the trust fix, the MCP cache write, the base url with its refusals and its
notice, the two switches, motion, and the flag-name guard. The provider client construction
no longer fails open: OpenRouter's `unwrap_or_else` is gone and Azure now also uses the
guarded builder. Four wiremock tests pin the redirect refusal and the loopback proxy bypass,
two per provider crate. The CLI gained 125 tests; it had none before.

`bench/check-dead-surface.py` is delivered as a tool and **not** added to the ship gate.
The guard had three bugs before this round: a doc-comment mention counted as a caller,
`examples/` counted as production, and the definition subtraction cancelled a real same-file
call for generic functions. All three are fixed. Run `python3 bench/check-dead-surface.py`
to see the current ledger. Do not write a count here: it moved through 35, 48, 52, 54, 55,
and 46 during this round alone, and it will move again as new public items are added.
The allowlist now holds twelve entries, each with a spec-anchored reason.
`D-the-dead-surface-allowlist-names-its-lane` forbids entries without an honest reason.
The triage is the next job. The guard joins the ship gate when the ledger is honest.

**Live holes this sprint did not close.** A review found the ledger silent about them.

`sandbox`, `approval`, `no-skills`, `no-agents`, and `subagents.*` are not in the powerful
set, and a project value beats a global one. An untrusted cloned repository can set any of
these and win over the user's own config, with no `--trust-project`. A typed flag still beats
all of them. `D-your-settings-are-a-floor` holds the rule and the probe to run. Until the
floor lands, pass `--sandbox` and `--read-only` as flags in a checkout you do not trust.

The MCP cache is nonce-guarded: each `LockGuard` writes an unguessable nonce and removes
the file only when it still holds that nonce, so a killed process cannot delete a successor's
lock. The residual race is narrower: a writer stalled past the steal timeout can still be
raced by a successor that acquires the lock before the stalled writer retries. There is also
no eviction bound; `CacheEntry` carries a `last_used` timestamp added this round, but no
pruning logic runs yet.

The separator guard is not delivered. `bindings()` mixes `ctrl-c` with `alt+enter`, and
normalising it rewrites strings that design fixtures assert. It stays named here so it is not
lost.

## 5. Out of scope

- Task progress rendering. It needs a row decision this spec does not make.
- A per-host credential map. `[credentials]` reaches no provider yet.
- A provider name per local host. See the decision.
- The interactive approval prompt, `/model`, `/sessions`, and compaction.
- Retention or pruning of the MCP cache file.

## 6. Delivery order

1. The guard, with its allowlist. It is the only item that protects the rest.
2. The MCP cache call, because a headline feature is broken without it.
3. The base url, with its refusals and its notice.
4. The two switches, skills and agents.
5. Motion.
6. The two source guards: the flag-name check and the separator check.
7. Break each guard on purpose. Step 7 of AGENTS.md.
8. Drive every one of them for real, including a local model host if one is available, and
   record the runs in `docs/verification/`.
9. Reconcile `docs/guide/`, `docs/features.md`, and this spec's status.
