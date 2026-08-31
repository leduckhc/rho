# SPEC-choose-a-model-and-configure-a-run — list models, pick one, and tune a run from the terminal

Status: draft, for review before any implementation.
Prior art: pi, jcode, and fx, read as source. See `docs/comparison.md`.

The user asked to choose a model, set reasoning effort, switch fast or normal, and change
other model settings from inside the terminal. The user chose the full-depth option. That
option lists the models a provider offers. It does not stop at typing an id.

This spec is a contract. It goes through review before any side writes code. It changes the
frozen `Provider` trait, so every provider and the testkit are sides. See step 3 of `AGENTS.md`.

## 0. The problem, and the facts that shape it

- rho has no runtime model list. `REMEMBERED_MODELS` in `rho-provider-bedrock` is a cache of
  ids already seen. It is not a catalogue. No code calls a listing API.
- A provider list already exists. `--provider nope` fails and names openrouter, bedrock, azure.
- `docs/verification/models.md` proves a hard fact. A provider can list a model that rho cannot
  use. Of six small Bedrock models, one cannot call a tool and one is flaky. `ACTIVE` is not
  callable. So "listed" and "usable" are different facts.
- Providers differ in kind. Bedrock has `ListFoundationModels`. OpenRouter has an HTTP models
  endpoint. Azure names deployments the account owner chose, not models. A contract that forces
  every provider to list is wrong for Azure.
- Reasoning effort exists. `rho-core::ReasoningEffort` has the ladder `off`, `low`, `medium`,
  `high`, `xhigh`. `ReasoningDisplay` has `off`, `summary`, `full`, `live`. See
  `SPEC-reasoning-across-providers`. This spec does not redesign them. It changes them mid-session.
- A model change already persists. `rho-core::session` writes a `ModelChange { provider, model }`
  record. rho keeps reasoning text for every turn.
- `rho-provider-bedrock` drops a stored reasoning replay whose owner model differs. It reports
  each drop in `dropped_replays`. A model switch triggers that drop.
- rho caches an MCP schema to `~/.rho/mcp-schema-cache.json`, keyed by a fingerprint. The first
  run with a new config shows no MCP tools, and that confused a first-time user. A model list is
  a slow remote call too, so it must not repeat that empty first run.

## 1. The sides

| Side | Owner | Must agree on |
| --- | --- | --- |
| The trait | `rho-core` | `catalog()` and `ModelCatalog::list_models` |
| The data model | `rho-core` | `ModelDescriptor` and its two fields |
| Each provider | `rho-provider-bedrock`, `rho-provider-openrouter`, `rho-provider-azure` | whether it lists, and what it returns |
| The contract testkit | `rho-provider-testkit` | the check that a provider answers `catalog()` honestly |
| The cache | `rho-cli` | where the list is stored, its key, and its lifetime |
| The screen | `rho-tui` | the picker, the filter, the keys, and the typed commands |
| The persisted session | `rho-core::session` | what a mid-session change writes |

## 2. Contract kinds this change touches

The public API, the data model, the error taxonomy, the persisted format, the configuration,
the extension surface, and the behaviour rules. This spec writes each one below.

## 3. The trait

Listing is optional. The type system carries that fact, not prose. A provider that can list
implements `ModelCatalog`. A provider that cannot returns `None` from `catalog()`. So a caller
tells "cannot list" from "listed nothing": `None` is the first, an empty `Vec` is the second.

`catalog()` is a required method with no default body. A default body would let a new provider
stay silent, and silence is the fail-open trap `AGENTS.md` step 8 names. Each provider states
its answer, the way `is_leaf_record` forces a record to state its class.

`crates/rho-core/src/provider.rs` gains this, and `list_models` uses `#[async_trait]` like
`stream`:

```rust
use crate::{CancelToken, ProviderError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// One model a provider offers. It carries the wire id and an optional label.
///
/// It carries no capability claim. A listing proves a model exists. It never proves the
/// model calls tools. See section 5, and `docs/verification/models.md`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    /// The exact value rho puts in `CompletionRequest.model`. Nothing transforms it.
    pub id: String,
    /// A human label from the provider, when it gives one. `None` shows `id`. rho never
    /// invents a label, because an invented label is a claim rho cannot prove.
    pub display_name: Option<String>,
}

/// A provider that can list its models. A provider that cannot does not implement this.
#[async_trait]
pub trait ModelCatalog: Send + Sync {
    /// List the models this provider offers. This is a network call.
    ///
    /// An empty `Ok(vec)` means the provider listed nothing. It is not the same as a
    /// provider that cannot list, which returns `None` from `Provider::catalog`.
    ///
    /// The call must select against `cancel.cancelled()` and stop when it fires.
    async fn list_models(
        &self,
        cancel: CancelToken,
    ) -> Result<Vec<ModelDescriptor>, ProviderError>;
}
```

`Provider` gains one method. It reuses `ProviderError`, so no new error type appears:

```rust
    /// The model catalogue for this provider, or `None` when the provider cannot list.
    ///
    /// Azure returns `None`, because it names deployments, not models. Bedrock and
    /// OpenRouter return `Some(self)`.
    fn catalog(&self) -> Option<&dyn ModelCatalog>;
```

## 4. What a model description carries

Two fields, and no more. `AGENTS.md` says to prefer the smaller interface.

- `id`: the whole point. It is the value that reaches `CompletionRequest.model`. Required.
- `display_name`: a label to show in the picker. Optional, because Bedrock gives one and a bare
  id does not. `None` shows the id, so rho invents nothing.

The user asked for a "fast or normal" switch. Speed is not a field. A listing API does not
report speed for a tool-calling run, so a speed field would be a claim rho cannot prove. Speed
is a separate concept. Section 8 maps it onto the existing reasoning effort ladder.

No context-length, price, or region field appears. rho cannot prove those for a real run, and
`AGENTS.md` forbids a claim without a measurement. A caller who needs one adds it in its own
crate, behind its own type.

## 5. Capability versus availability

`docs/verification/models.md` proves a listed model may not call a tool. So rho claims only what
it can prove. A listing proves availability. It never proves capability.

The picker shows the id and the label only. It shows one fixed line under the list:
`availability only; rho has not verified tool use`. It shows no green check, no "supports tools"
badge, and no speed rating. A verified result lives in `docs/verification/models.md`, which a
human wrote from a real run. The picker never restates it as a per-model claim.

## 6. The extension point

A fourth provider implements listing with no edit to shared code. It implements `ModelCatalog`
in its own crate and returns `Some(self)` from `catalog()`. A provider that cannot list returns
`None`. rho reads `ModelDescriptor`, which it already owns. A third party writes one trait impl
and one `catalog()` body. It edits no file in `rho-core`, `rho-tui`, or `rho-cli`.

## 7. Freshness and cost

Listing is a network call. It must not slow a session that already works.

- When it happens: rho lists lazily, on the first `/model` open, not at startup. A session that
  never opens the picker makes no listing call.
- First-run cost: one round trip on the first open. The picker is never empty. It shows the
  current model at once, before the call returns. It marks the list `loading…` while the call
  runs. This differs on purpose from the MCP cache, whose empty first run confused a user.
- Cache: rho writes the result to `~/.rho/model-catalog-cache.json`. The key is a fingerprint of
  the provider id and its endpoint. The endpoint is the AWS region for Bedrock and the base URL
  for OpenRouter. `rho-cli` owns the cache, because `rho-core` holds no HTTP or disk-config code.
- Invalidation: a cache entry lives for 24 hours. A fingerprint change writes a new entry, the
  way the MCP cache does. An old entry is never served for a changed endpoint.
- On failure: a listing error never stops a session. The picker keeps the current model, shows
  one line naming the failure, and lets the user type an id. A stale cache entry, when present,
  still shows, marked stale.

## 8. Applying a change mid-session

The prompt stays append-only. A stable prefix keeps the provider cache warm. So no change edits
a sent turn. Each change takes effect on the next provider request.

| Change | When it takes effect | The turn in flight | Written to the session file | Stored reasoning on the old model |
| --- | --- | --- | --- | --- |
| Model | Next request | Finishes on the old model | `ModelChange { provider, model }` | The provider drops a replay whose owner differs, and reports it in `dropped_replays` |
| Reasoning effort | Next request | Unchanged | Nothing this spec adds; see Out of scope | Unaffected |
| Reasoning display | At once, on screen | Redraws, including kept reasoning | Nothing; display is not persisted | Unaffected |
| Speed | Next request | Unchanged | Nothing beyond the effort it sets | Unaffected |

The `ModelChange` record is sufficient for identity. It names the provider and the model, and a
reader replays each turn against its recorded owner. Speed is an alias over effort. `/speed fast`
sets `ReasoningEffort::Off`. `/speed normal` restores the effort the session started with. Speed
stores no new state, so it needs no new field and no new record.

## 9. The interface

The user opens the picker with the `/model` command. `/model` already exists in
`rho-tui::bindings`, marked not built. This spec builds it. The user filters by typing after the
list opens. The user moves the selection with `↑` and `↓`. The user chooses with `Enter`. The
user closes with `Esc`.

No new global key is added. `/` opens the command list, `↑ ↓` move a selection, `Enter` runs,
and `Esc` closes a panel. Those bindings exist in `rho-tui::bindings`. The picker reuses them, so
it collides with nothing.

Typed commands do the same without the picker:

- `/model <id>` sets the id directly and skips the list.
- `/reasoning <level>` sets the effort. The levels are `off`, `low`, `medium`, `high`, `xhigh`.
- `/reasoning-display <mode>` sets the display. The modes are `off`, `summary`, `full`, `live`.
- `/speed fast` and `/speed normal` set the speed alias.

Each typed command reuses the parser in `rho-core::reasoning`, so a name maps one way only.

## 10. What the contract forbids

Each rule has a name and a test in the next section.

- `catalog_none_is_not_empty_list`: a provider that cannot list returns `None`, never
  `Ok(vec![])`. Azure must not pretend it listed nothing.
- `no_unproven_capability_claim`: the picker shows no tool-use, speed, or price claim.
- `picker_never_empty_on_first_run`: the picker always shows the current model, even before the
  first listing call returns.
- `listing_failure_never_stops_the_session`: a listing error keeps the session and the current
  model alive.
- `a_typed_id_is_always_allowed`: rho accepts a typed id even when the list omits it. The list is
  advisory, because `models.md` shows a needed id form can be absent.
- `no_mid_turn_model_switch`: a model change never alters the turn in flight.
- `stale_reasoning_never_replays_to_a_new_model`: a switch drops reasoning bound to the old model.

## Test cases

- `catalog_reports_none_for_azure` proves Azure returns `None` from `catalog()`.
- `catalog_reports_some_for_bedrock` proves Bedrock returns `Some` and lists models.
- `catalog_reports_some_for_openrouter` proves OpenRouter returns `Some` and lists models.
- `empty_list_is_ok_not_none` proves an empty `Ok(vec)` is distinct from `None`.
- `list_models_stops_on_cancel` proves a cancelled `list_models` returns and does not hang.
- `descriptor_has_only_id_and_label` proves `ModelDescriptor` carries no third field.
- `descriptor_none_label_shows_id` proves the picker shows the id when the label is `None`.
- `picker_shows_no_capability_badge` proves the picker draws no tool-use claim.
- `picker_shows_current_model_before_list_returns` proves the first open is never empty.
- `listing_failure_keeps_the_current_model` proves a list failure mid-session keeps the session.
- `listing_failure_shows_one_error_line` proves the failure draws one line, not a stack.
- `typed_id_absent_from_list_is_accepted` proves a typed id runs even when the list omits it.
- `model_change_writes_a_model_change_record` proves the switch appends `ModelChange`.
- `model_change_takes_effect_next_request` proves the current turn finishes on the old model.
- `model_switch_drops_reasoning_bound_to_old_model` proves the drop reaches `dropped_replays`.
- `reasoning_effort_change_reaches_next_request` proves the next `CompletionRequest.reasoning`.
- `display_change_redraws_kept_reasoning` proves a switch to `full` shows earlier reasoning.
- `speed_fast_sets_effort_off` proves `/speed fast` sets `ReasoningEffort::Off`.
- `speed_normal_restores_start_effort` proves `/speed normal` restores the start effort.
- `cache_key_changes_with_region` proves a Bedrock region change writes a new cache entry.
- `stale_cache_is_marked_stale` proves an expired entry shows, marked stale.
- `run_all_checks_catalog_answer` proves `rho-provider-testkit::contract::run_all` asserts every
  provider answers `catalog()` without a panic.

## Out of scope

- A record for a reasoning effort, display, or speed change. This spec persists none of them.
  Resume uses the CLI flag or config value. The owner of `SPEC-reasoning-across-providers` adds a
  record later, if resume fidelity needs one.
- Verifying tool use during listing. A verified result stays in `docs/verification/models.md`.
- Model price, context length, and region metadata.
- Azure deployment discovery. Azure returns `None`, and the user names the deployment.
- A config-file model registry. `F-model-registry` covers that, and this spec does not build it.
- Ranking or scoring the list. The picker shows the provider's order.
