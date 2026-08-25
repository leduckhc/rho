# SPEC-wire-the-dead-switches — six switches that reach nothing, and the guard for the class

Status: draft. No code yet. The tests below are named and not written.
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
them by hand. The task-progress row is out of scope; it needs a
row-rendering decision this spec does not make.

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
    pub reduce_motion_env: bool,
}

pub fn motion_enabled(inputs: MotionInputs) -> bool;
```

`TuiState` carries one resolved flag, `motion: bool`, and `footer_line` consults it before it
sweeps. The four-field struct loses the two fields nobody sets.

### `rho-mcp`: the cache is written

`Extensions` gains nothing public. `extensions::load` writes the cache after a successful
handshake, so the next session advertises the tools. The notice changes accordingly, and no
notice promises a session that cannot differ.

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
6. When definitions exist and agent discovery is off, rho says how many it skipped and which
   switch did it.
7. `--no-motion`, `tui-motion = false`, or `RHO_REDUCE_MOTION=1` each stop the sweep. The
   footer still names the state in words.
8. A successful MCP handshake writes the schema cache. A later session advertises those tools.
9. The refusal that names `--allow-widen` no longer names a flag that does not exist.
10. Every key in the binding table uses one separator style.

## 4. Test cases

### The base url

- `a_base_url_key_reaches_the_provider` — the resolved config carries it, and the provider is
  built with it.
- `no_base_url_keeps_the_default_endpoint` — the OpenRouter endpoint is unchanged.
- `a_plain_http_remote_base_url_is_refused` — the run stops, and the message names the scheme.
- `a_loopback_http_base_url_is_allowed` — `http://localhost:11434` and `http://127.0.0.1:8080`
  both pass, because that traffic never leaves the machine.
- `an_untrusted_project_file_loses_the_base_url` — it joins the drop list.
- `a_trusted_project_file_keeps_the_base_url` — `--trust-project` restores it.
- `setting_a_base_url_names_the_host_in_a_notice` — the user is told where the key goes.

### Skills and agents are two switches

- `no_skills_leaves_agent_discovery_on` — `spawn_agent` is still registered.
- `no_agents_leaves_skill_discovery_on` — skills still load.
- `no_agents_stops_the_definition_search` — no spawn tool is registered.
- `a_skipped_definition_is_reported` — the notice names the count and the switch.

### Motion

- `motion_is_on_by_default` — today's behaviour is unchanged.
- `the_no_motion_flag_stops_the_sweep` — the frame draws the word plain.
- `the_tui_motion_key_stops_the_sweep` — the config key does the same.
- `the_reduce_motion_variable_stops_the_sweep` — `RHO_REDUCE_MOTION=1` does the same.
- `a_non_terminal_stdout_stops_the_sweep` — the existing rule still holds.
- `the_footer_still_names_the_state_without_motion` — the state never rests on the animation.

### The MCP cache

- `a_successful_handshake_writes_the_schema_cache` — the file exists afterwards.
- `a_cached_schema_advertises_a_tool_on_the_next_session` — the tool reaches the registry with
  no handshake.
- `a_failed_handshake_writes_no_cache` — a broken server leaves no false promise.
- `the_notice_does_not_promise_a_session_that_cannot_differ` — the wording follows the code.

### The guard

- `the_guard_reports_an_uncalled_public_function` — a fixture crate with one uncalled function
  fails the check.
- `the_guard_counts_no_test_as_a_caller` — a function called only from `tests/` still reports.
- `an_allowlist_entry_with_a_reason_passes` — the ledger works.
- `an_allowlist_entry_without_a_reason_fails` — a bare path is not an exemption.
- `an_allowlist_entry_naming_an_unknown_spec_fails` — a lane cannot be invented.
- `an_unnecessary_allowlist_entry_fails` — a function that gained a caller must leave the list.

### The two smaller defects

- `no_error_message_names_a_flag_that_does_not_exist` — a source guard over every message in
  `crates/*/src` that says `pass --`, checked against the real flag list.
- `every_binding_uses_one_separator_style` — the help table cannot mix `ctrl-c` and
  `alt+enter`.

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
