# SPEC-switch-the-provider-mid-session — list across providers and switch live

Status: draft.
Owning crate: `rho-core`, with sides in `rho-cli` and `rho-tui`.

Feature: `F-model-picker`. This spec extends that feature. It does not add a new one.

## The problem

The user wants this: "List across providers AND switch the session live. The picker lists
models from every built-in and every `[[providers]]` entry, each row labelled with its
provider, and picking a foreign model rebuilds the session's provider in place, so it just
works."

Two direct instructions shape the row. First, there is no separate column for the maker;
the maker goes back into the model name, so a row shows `anthropic/claude-3.5-sonnet`.
Second, the dim column holds the rho provider instead: `bedrock`, `azure`, `openrouter`, or
a named `[[providers]]` id.

## The sides

Six sides must agree. Each has one owner.

| Side | Owner | What it must agree to |
| --- | --- | --- |
| The running unit and the selection type | `rho-core` | provider, name, model, and effort are one mutex, read once per turn |
| The provider factory | `rho-core` owns the trait, `rho-cli` owns the impl | build a provider by name, with no HTTP in the core |
| The catalog source and the picker row | `rho-tui` owns both, `rho-cli` implements the source | list lazily, one row per model, dim provider column |
| The bounds and the cache files | `rho-cli` builds them, `rho-tui` schedules them | one cache file per provider, capped and cancellable |
| The provider enumeration and trust gate | `rho-config` | a provider reaches the picker only when the trusted config names a credential for it |
| The persisted session file | `rho-core` session store | `ModelChange` already carries the provider, so no new record |

The persisted format is a real side. `Record::ModelChange { provider: String, model:
String }` already exists in `crates/rho-core/src/session/mod.rs` at line 94. It already
carries a provider. So a provider switch needs no new record type, and a live switch writes
one `ModelChange` with the new provider name.

The contract kinds this change touches are the public API, the data model, the error
taxonomy, the persisted format, and the extension surface. It touches no wire format and no
new config key.

## The extension point

A new provider arrives as a new impl behind `Provider`, plus one line in `build_provider`,
plus one entry in the factory's enumeration and the source's `provider_names`. It never
edits `Session`, `ModelSelection`, `ProviderFactory`, `CatalogSource`, or the picker. A
third party adds a provider crate and a builder branch, and the switch and the listing work
with no core edit. See `D-provider-contract-crate`.

## 1. Contract

Every block below is compilable Rust. The types are verbatim.

### 1a. The selection type, in `rho-core`

`ModelSelection` gains one field, `provider`. `None` keeps the current provider. `Some(name)`
expresses a switch to that provider. The frontend reads the field to decide whether a pick
is a switch. The core never silently drops it: `apply_selection` builds or refuses when the
field names a different provider.

```rust
// crates/rho-core/src/agent.rs

/// The mutable slice of the session: the model, the effort, and the provider.
///
/// `Session::selection` reads it and `Session::apply_selection` writes it. The running turn
/// is not affected. See `D-the-provider-is-mutable-behind-a-mutex`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelSelection {
    /// The provider-specific model id, exactly as `--model` accepts it.
    pub model: String,
    /// How hard the model should think. `None` means the provider's own default.
    pub reasoning_effort: Option<crate::ReasoningEffort>,
    /// The provider to run the model on. `None` keeps the session's current provider, so a
    /// model-only or effort-only change never builds a provider. `Some(name)` names a
    /// switch. `Session::selection` always returns `Some(<running name>)`. See
    /// `D-the-provider-is-mutable-behind-a-mutex`.
    pub provider: Option<String>,
}

impl ModelSelection {
    /// A model and effort change on the current provider. `provider` is `None`.
    pub fn new(model: impl Into<String>, reasoning_effort: Option<crate::ReasoningEffort>) -> Self {
        Self {
            model: model.into(),
            reasoning_effort,
            provider: None,
        }
    }

    /// A switch to `provider`, then this model and effort. `provider` is `Some(name)`.
    pub fn switch(
        provider: impl Into<String>,
        model: impl Into<String>,
        reasoning_effort: Option<crate::ReasoningEffort>,
    ) -> Self {
        Self {
            model: model.into(),
            reasoning_effort,
            provider: Some(provider.into()),
        }
    }
}
```

### 1b. The provider factory and its error, in `rho-core`

```rust
// crates/rho-core/src/agent.rs (or a small sibling module in rho-core)

/// Builds a provider by name for a mid-session switch.
///
/// `rho-core` owns this trait and the `Session` holds it as a trait object, so the core
/// never gains an HTTP or a credential dependency. `rho-cli` owns the one implementation,
/// which wraps `build_provider` with the merged config and the environment. The call is
/// synchronous and does no network I/O. See `D-the-core-takes-a-provider-factory`.
pub trait ProviderFactory: Send + Sync {
    /// Build the provider named `name`, or say why it cannot build. A failure leaves the
    /// session on its current provider. See
    /// `D-a-provider-switch-that-cannot-build-is-refused`.
    fn build(&self, name: &str) -> Result<std::sync::Arc<dyn Provider>, ProviderBuildError>;
}

/// Why a provider switch could not build a provider. It holds one line and no secret. The
/// frontend shows it, and the session keeps its current provider.
#[derive(Debug, Clone, thiserror::Error)]
#[error("cannot switch provider: {0}")]
pub struct ProviderBuildError(pub String);
```

### 1c. The session, in `rho-core`

The provider, its config name, the model, and the effort become one interior-mutable unit,
`RunningState`. The factory lives on `Session`, outside the `Arc`.

```rust
// crates/rho-core/src/agent.rs

/// The running provider, its config name, the model, and the effort, as one unit. Read
/// once per turn, and swapped as a whole by `apply_selection`. So the request and the
/// provider that streams it always agree. See `D-the-provider-is-mutable-behind-a-mutex`.
struct RunningState {
    provider: std::sync::Arc<dyn Provider>,
    /// The config provider name, such as `openrouter` or `xdent-claude`. It is not
    /// `Provider::id`, which is only the protocol, so a named entry keeps its own id.
    provider_name: String,
    model: String,
    reasoning_effort: Option<crate::ReasoningEffort>,
}

struct SessionInner {
    tools: std::sync::Arc<ToolRegistry>,
    hooks: std::sync::Arc<HookChain>,
    context: tokio::sync::Mutex<Context>,
    config: SessionConfig,
    /// One mutex for the whole running unit. It replaces the old `provider` and
    /// `selection` fields. See `D-the-provider-is-mutable-behind-a-mutex`.
    running: std::sync::Mutex<RunningState>,
    queue: crate::MessageQueue,
}

pub struct Session {
    inner: std::sync::Arc<SessionInner>,
    /// Builds a provider for a switch. It sits on `Session`, outside the `Arc`, because only
    /// `apply_selection` reads it and the driver never does. `None` refuses every switch.
    /// The session records no `ModelChange` itself; the frontend seam does that. See
    /// `D-the-core-takes-a-provider-factory` and
    /// `SPEC-the-interactive-session-records-itself`.
    provider_factory: Option<std::sync::Arc<dyn ProviderFactory>>,
}

impl Session {
    /// Inject the provider factory a switch needs. Without it, a switch is refused. This
    /// sets a `Session` field with struct-update syntax, the same shape as `with_queue`.
    pub fn with_provider_factory(self, factory: std::sync::Arc<dyn ProviderFactory>) -> Self {
        Self {
            provider_factory: Some(factory),
            ..self
        }
    }

    /// Override the running provider name, from the resolved config name. `with_config`
    /// seeds the name from `Provider::id`, which is right for a built-in and wrong for a
    /// named entry. `rho-cli` calls this with the real config name. It rebuilds
    /// `SessionInner` by `Arc::try_unwrap`, so it must run before the session is shared,
    /// exactly like `with_queue`.
    pub fn with_provider_name(self, name: impl Into<String>) -> Self {
        let name = name.into();
        let inner = std::sync::Arc::try_unwrap(self.inner)
            .unwrap_or_else(|_| panic!("with_provider_name must run before the session is shared"));
        // Destructure `SessionInner` fully, then rebuild it field by field. A partial move
        // of `running` out of `inner` followed by `..inner` is a use of a moved value, which
        // does not compile. See E0382.
        let SessionInner {
            tools,
            hooks,
            context,
            config,
            running,
            queue,
        } = inner;
        let mut running = running
            .into_inner()
            .expect("the running lock is never poisoned");
        running.provider_name = name;
        Self {
            inner: std::sync::Arc::new(SessionInner {
                tools,
                hooks,
                context,
                config,
                running: std::sync::Mutex::new(running),
                queue,
            }),
            provider_factory: self.provider_factory,
        }
    }

    /// Read the current model, effort, and provider. `provider` is always `Some`, and it
    /// names the running provider. The value is a snapshot.
    pub fn selection(&self) -> ModelSelection {
        let running = self
            .inner
            .running
            .lock()
            .expect("the running lock is never poisoned");
        ModelSelection {
            model: running.model.clone(),
            reasoning_effort: running.reasoning_effort,
            provider: Some(running.provider_name.clone()),
        }
    }

    /// Apply a selection. Build and swap the provider first when the selection names a
    /// different one. Then commit the model and the effort. Take effect at the next turn.
    ///
    /// On a build failure this changes nothing and returns the error. On success it commits
    /// the provider, its name, the model, and the effort under one lock. It records no
    /// `ModelChange` itself. The frontend records a model or provider change through the
    /// `record_turn` seam, after a successful apply, so the session holds no recorder. See
    /// `D-a-provider-switch-that-cannot-build-is-refused` and
    /// `SPEC-the-interactive-session-records-itself`.
    pub fn apply_selection(&self, selection: ModelSelection) -> Result<(), ProviderBuildError> {
        // Decide whether this is a switch, under the running lock, so the read is
        // consistent.
        let switch_to = {
            let running = self
                .inner
                .running
                .lock()
                .expect("the running lock is never poisoned");
            match &selection.provider {
                Some(name) if *name != running.provider_name => Some(name.clone()),
                _ => None,
            }
        };

        // Build before any commit. On failure, change nothing.
        let built = match &switch_to {
            Some(name) => {
                let factory = self.provider_factory.as_ref().ok_or_else(|| {
                    ProviderBuildError("no provider factory is configured".to_string())
                })?;
                Some((name.clone(), factory.build(name)?))
            }
            None => None,
        };

        // Commit under one lock: swap the provider, its name, the model, and the effort
        // together. So a concurrent turn read never sees a half-applied switch. The
        // frontend seam records the `ModelChange` after this returns `Ok`, not the session.
        {
            let mut running = self
                .inner
                .running
                .lock()
                .expect("the running lock is never poisoned");
            if let Some((name, provider)) = built {
                running.provider = provider;
                running.provider_name = name;
            }
            running.model = selection.model;
            running.reasoning_effort = selection.reasoning_effort;
        }
        Ok(())
    }
}
```

`with_config` changes its body, not its signature. It wraps the provider in `RunningState`,
seeds `provider_name` from `Provider::id`, and sets `provider_factory: None`:

```rust
// crates/rho-core/src/agent.rs, inside with_config

let provider_name = provider.id().to_string();
let running = std::sync::Mutex::new(RunningState {
    provider,
    provider_name,
    model: config.model.clone(),
    reasoning_effort: config.reasoning_effort,
});
Self {
    inner: std::sync::Arc::new(SessionInner {
        tools,
        hooks,
        context: tokio::sync::Mutex::new(context),
        config,
        running,
        queue: config_queue,
    }),
    provider_factory: None,
}
```

The turn read changes at `run_turn`, `agent.rs` line 707, and the `build_request` call it
feeds is at line 712. The whole running unit comes from the mutex once, and `build_request`
takes the model and the effort it already read:

```rust
// crates/rho-core/src/agent.rs, inside run_turn

// Read the provider, model, and effort together, once, under one lock. A concurrent
// `apply_selection` swaps all of them at once, so this snapshot is never half-new. See
// `D-the-provider-is-mutable-behind-a-mutex`.
let (provider, model, reasoning_effort) = {
    let running = self
        .inner
        .running
        .lock()
        .expect("the running lock is never poisoned");
    (
        running.provider.clone(),
        running.model.clone(),
        running.reasoning_effort,
    )
};
let request = self.build_request(model, reasoning_effort).await;
let mut stream = match provider.stream(request, self.cancel.clone()).await {
    Ok(stream) => stream,
    Err(error) => {
        let _ = self.tx.send(Err(Error::from(error))).await;
        return TurnOutcome::Failed;
    }
};
```

```rust
// crates/rho-core/src/agent.rs, build_request now takes the snapshot values

async fn build_request(
    &self,
    model: String,
    reasoning_effort: Option<crate::ReasoningEffort>,
) -> CompletionRequest {
    let context = self.inner.context.lock().await;
    CompletionRequest {
        model,
        system: context.system().map(str::to_string),
        messages: context.messages().to_vec(),
        tools: self.inner.tools.specs(),
        max_tokens: None,
        temperature: None,
        reasoning: reasoning_effort,
    }
}
```

### 1d. The picker row and the catalog source, in `rho-tui`

`PickerRow` gains a `provider` field. The dim column draws it, and a pick applies it. The
name is now the full model id. The App holds a lazy `CatalogSource`, not a pre-built list.

```rust
// crates/rho-tui/src/state.rs

pub struct PickerRow {
    /// The full model id. The row draws it as the name, so the maker stays in the name.
    pub id: String,
    /// The provider that applies when this row is picked. The dim column draws it. A pick
    /// with a provider that differs from the running one switches the session. See
    /// `D-the-picker-lists-every-configured-provider`.
    pub provider: String,
    /// True when this row is the current model and provider of the session.
    pub is_current: bool,
    /// True when this row is in the starred file.
    pub starred: bool,
    /// The preview effort. `None` keeps the session's current effort. Cycled by Tab.
    pub effort: Option<ReasoningEffort>,
    /// True when this row came from a stale cache entry, not a fresh listing.
    pub stale: bool,
    /// True when this row came from a provider catalog or a suggestion list.
    pub catalog: bool,
}
```

```rust
// crates/rho-tui/src/app.rs

/// Supplies provider catalogs to the picker, lazily, on open. `rho-cli` implements it. The
/// picker never calls it at startup, so a launch resolves no foreign credential. See
/// `D-the-picker-lists-every-configured-provider`.
pub trait CatalogSource: Send + Sync {
    /// The provider ids to list, current provider first. It names a provider only when the
    /// configuration names a credential for that provider. Cheap: it reads config, builds
    /// nothing, and resolves no credential value. See
    /// `D-a-provider-with-no-credential-is-not-listed`.
    fn provider_names(&self) -> Vec<String>;

    /// Build or reuse the catalog for one provider. `Ok(None)` means the provider cannot
    /// list, such as Azure. `Err` holds one secret-free line. This resolves a credential,
    /// so the App calls it inside a spawned task, never on the event loop.
    fn catalog(
        &self,
        provider: &str,
    ) -> Result<Option<std::sync::Arc<dyn rho_core::ModelCatalog>>, String>;
}

/// The most providers the picker lists at once. `provider_names` returns no more, and drops
/// the rest with a note. See `D-the-picker-lists-every-configured-provider`.
pub const MAX_PROVIDERS: usize = 16;
/// The most catalog listings that run at once.
pub const MAX_CONCURRENT_LISTINGS: usize = 4;

impl App {
    /// Give the picker a lazy catalog source. This replaces `with_catalog`. See
    /// `D-the-picker-lists-every-configured-provider`.
    pub fn with_catalog_source(mut self, source: std::sync::Arc<dyn CatalogSource>) -> Self {
        let _ = source;
        self
    }
}

/// The result of one provider's catalog load. Every event names its provider, so a late
/// load appends to the right rows and a failure marks only its own provider.
enum CatalogEvent {
    Models {
        provider: String,
        models: Vec<rho_core::ModelDescriptor>,
    },
    Error {
        provider: String,
        message: String,
    },
}
```

The two row builders gain a provider argument, so every catalog row carries its provider:

```rust
// crates/rho-tui/src/state.rs

/// Append catalog rows for one provider. Each row's `provider` is `provider`.
pub fn append_catalog_models(&mut self, provider: &str, models: &[rho_core::ModelDescriptor]) {
    let _ = (provider, models);
}

/// Seed the picker with a provider's cached rows, marked stale. Each row's `provider` is
/// `provider`.
pub fn seed_picker_with_cached_models(
    &mut self,
    provider: &str,
    models: &[rho_core::ModelDescriptor],
) {
    let _ = (provider, models);
}
```

### 1e. The fuzzy filter, in `rho-tui`

The query matches the full model id or the provider id, both as a case-insensitive
subsequence. So a provider search works.

```rust
// crates/rho-tui/src/state.rs

/// The indices of the displayed rows that pass the fuzzy filter.
///
/// An empty query returns every display index. A non-empty query keeps a row when the query
/// matches the full model id or the provider id. See
/// `D-the-picker-lists-every-configured-provider`.
pub fn filtered_indices(&self, starred: &[String]) -> Vec<usize> {
    let display = self.display_rows(starred);
    if self.query.is_empty() {
        return (0..display.len()).collect();
    }
    display
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            fuzzy_match(&self.query, &row.id) || fuzzy_match(&self.query, &row.provider)
        })
        .map(|(index, _)| index)
        .collect()
}
```

### 1f. The header mirror, in `rho-tui`

`set_current_selection` now mirrors the provider too. On `Ok` from `apply_selection`, the
frontend re-reads `session.selection()` and mirrors the whole triple. So the header names
the true running provider.

```rust
// crates/rho-tui/src/state.rs

/// Mirror the running model, effort, and provider into the header state. The provider comes
/// from `selection()`, which always returns `Some(<running name>)`, so the header never
/// drifts from the wire. See `D-the-provider-is-mutable-behind-a-mutex`.
pub fn set_current_selection(&mut self, selection: &ModelSelection) {
    self.model = selection.model.clone();
    self.reasoning_effort = selection.reasoning_effort;
    if let Some(provider) = &selection.provider {
        self.provider = provider.clone();
    }
}
```

## 2. How does the core get a new provider without an HTTP dependency

The `Session` takes a factory. `ProviderFactory` lives in `rho-core`, and the `Session`
holds it as a trait object on a field, outside the `Arc`. `rho-cli`
implements it over `build_provider`, so all HTTP and credential code stays in `rho-cli`.
`build` returns `Result<Arc<dyn Provider>, ProviderBuildError>`.

It is synchronous. `build_provider` is synchronous today, it resolves an in-memory config,
and it constructs a client with no network call. `apply_selection` runs inline on the event
loop, so the build runs there. One credential form blocks: a `!command` credential runs a
shell subprocess, bounded by the 30-second credential timeout. See
`D-credential-command-allowlist`. So a switch to a provider with a `!command` credential can
freeze the terminal for up to that timeout. An `env:` credential, a literal, and the
Bedrock AWS chain do not block. This bounded freeze is a known limit, and a frontend may
move the build off the event loop later. See `D-the-core-takes-a-provider-factory`.

## 3. What happens when the switch fails

A failed switch is refused, and the session keeps its current provider, model, and effort.
`apply_selection` builds first and commits second, so a failure writes nothing. The failure
returns `ProviderBuildError`, which holds one line and no secret. The core refuses and the
frontend reports: `rho-tui` pushes one error row and does not mirror the pick into the
state, so the header keeps naming the running provider. A missing credential, an unknown
protocol, an untrusted project credential, and a base-url conflict each map to this one
error. See `D-a-provider-switch-that-cannot-build-is-refused`.

## 4. When does the switch take effect

The next turn. `run_turn` reads the running unit once at the start of a turn and clones the
provider, the model, and the effort for the whole turn. A turn that is already streaming
keeps its old provider and model, so the stable prompt prefix stays byte-identical for that
turn. The next turn reads the unit again and uses the new provider. The read is one lock
hold, so a swap can never pair a new provider with an old model. See
`D-the-provider-is-mutable-behind-a-mutex`.

## 5. How does the picker list several providers

The App holds a lazy `CatalogSource`. On picker open, the App reads `provider_names` cheaply,
then a coordinator task lists each named provider. `provider_names` names a provider only
when the configuration names a credential for it, so a provider the user never configured is
never listed and rho makes no call for it. The check reads configuration and resolves no
credential value. See `D-a-provider-with-no-credential-is-not-listed`. Building a provider
and resolving its credential happen inside a spawned task, on picker open, never at startup
and never on the event loop. So a user who never opens the picker resolves no foreign credential and makes no
network call. The `rho-cli` source reuses the session's already-built provider for the
current name, so the current provider is never rebuilt.

A user with ten providers pays nothing until they open the picker. Then the coordinator
runs at most `MAX_CONCURRENT_LISTINGS` loads at once, so ten providers list in three waves.
Each load runs under `LIST_TIMEOUT`. One provider's failure adds one note row and never
empties the picker, because each load is independent and tagged with its provider. A
provider with no catalog, such as Azure, contributes its current, starred, and suggested
rows, because those come from ids, not from a listing.

Closing the picker fires one shared cancel token. In-flight loads stop, and the coordinator
stops launching the pending waves. So closing the picker cancels the queued work. See
`D-the-picker-lists-every-configured-provider`.

## 6. What does a row look like now

The name is the full model id, sanitized, so the maker stays in the name, for example
`anthropic/claude-3.5-sonnet`. The dim column draws the provider id, for example `bedrock`.
The current model and provider draw the ` (current)` tag. A stale cached row draws the `
(stale)` tag. A row from a provider that is not the session's current provider is marked
only by its dim provider label. No other tag distinguishes a foreign row, because picking
it switches and the provider label already names where it runs.

The dim provider column comes from `PickerRow::provider`, not from the id. So the old
id-derived maker label is gone. This reverses the split from
`D-a-picker-row-labels-its-vendor`.

## 7. What does the fuzzy filter match

The full model id and the provider id. Both match as a case-insensitive subsequence. So a
user who types `bedrock` sees every Bedrock row, and a user who types `sonnet` sees every
sonnet row across providers. See section 1e.

This amends the match rule twice-over. The delivered spec
`SPEC-the-model-picker-groups-and-labels-rows` and `D-a-picker-row-labels-its-vendor`
narrowed the match to the model name or the vendor label. This spec widens it to the full
model id or the provider id, because the vendor label is gone and the provider replaces it.
This spec amends the match rule of `D-model-picker-allows-fuzzy-search-and-typed-fallback`
to read: "case-insensitive subsequence against the full model id or the provider id; the
full id is what `Enter` applies." The controller owns that decision file.

## 8. What are the trust rules

A project config must not cause rho to build a provider with a credential the user did not
authorise. The switch reuses `build_provider`, so it runs the same credential gate as
startup. An untrusted project credential is refused, and the message names `--trust-project`.
See `D-an-untrusted-clone-supplies-no-credential`. A credential name resolves through the
merged, trusted table, and a provider names its own fallback variable. See
`D-a-provider-names-its-own-credential`. The picker's provider set is the trusted build set
only, because `rho-config` clears an untrusted project `[[providers]]` entry at load.

The picker lists a provider only when the configuration names a credential for it. A
built-in with no named credential is absent, so the picker never fills with credential-error
rows. An error row is only for a provider that was configured and then failed at listing
time. The check reads configuration and never resolves a credential value, so
`provider_names` stays cheap. See `D-a-provider-with-no-credential-is-not-listed`.

A base url can redirect a credential, so a switch inherits the startup base-url gate too. A
switch to an OpenAI-compatible provider inherits `config.base_url` and sends the key there.
`rho-config` drops an untrusted project `base-url`, so a clone cannot set one, and a
provider that names its endpoint its own way refuses a base url. A switch also announces a
base-url redirect the way startup announces it, so the user always sees the key's
destination. See `D-a-provider-base-url-is-a-config-key` and
`D-a-provider-switch-that-cannot-build-is-refused`.

## 9. What is the bound

- `MAX_PROVIDERS = 16`. The most providers the picker lists at once. `provider_names`
  returns no more, and drops the rest with a note.
- `MAX_CONCURRENT_LISTINGS = 4`. The most listings that run at once.
- `LIST_TIMEOUT`, ten seconds, from `rho-core`. The longest one listing waits.
- `MAX_MODELS`, two thousand, from `rho-core`. The most models one listing keeps.
- `DEFAULT_CREDENTIAL_TIMEOUT`, thirty seconds, from `rho-config`. The longest a switch
  build waits on a `!command` credential.

### 9a. The cache file is per provider

The catalog cache uses one file per provider, not one shared file. Today `CatalogCache::new`
in `crates/rho-cli/src/catalog_cache.rs` joins one path, `model-catalog-cache.json`, for
every provider. Each `CatalogCache` holds its own lock, and the lock does not span two
instances. So two providers that list at once each read the whole shared file, then each
write the whole file back. The second write drops the first provider's fresh entry. The
picker lists several providers at once, so this race is now reachable.

The fix keys the file by the provider fingerprint. The file name is
`model-catalog-<key>.json`, where `<key>` is a hex-encoded stable digest of
`Provider::catalog_fingerprint()`. The digest keeps the name filesystem-safe, because a raw
fingerprint may hold a `/`, a `:`, or a base url. Each provider writes only its own file, so
two concurrent listings never touch the same path and neither write is lost. The fingerprint
already varies with the endpoint, such as a Bedrock region, so a moved endpoint gets its own
file too.

An old shared `model-catalog-cache.json` is not read and not migrated. A new rho ignores it
and re-fetches each provider once into its per-provider file. The old file is harmless dead
data, and a user may delete it. rho leaves it in place.

## 10. Starred models across providers

`~/.rho/starred-models.toml` stays a flat array of ids, with no provider. So there is no
file migration. A starred id means "show this id in the starred section." The picker binds
each starred id to a provider at display time: the current provider when it offers the id,
else the first listed provider that offers it, else the current provider as a bare fallback.
The bound provider fills the row's dim column and the row's `provider` field. So a picked
starred row applies a concrete provider, which may be a switch.

A starred entry does not need a provider in the file, because the binding is derived from
the live catalog set every time the picker opens. A `provider/id` pair in the file is a
future option, and it needs its own decision, because it is a format migration. See
`D-starred-models-live-in-their-own-file`.

## 11. The persisted file, and resume

`Record::ModelChange { provider, model }` is unchanged. So an old reader parses a new file
with no error. The record is the second line of every file, and it repeats on every switch.
The newest `ModelChange` names the running pair. See
`D-resume-restores-the-recorded-provider`.

A live switch records one `ModelChange` with the new provider name. The frontend records it,
not the session. `SPEC-the-interactive-session-records-itself` wires a recorder onto the
interactive TUI path and folds each record through the `record_turn` seam. So the TUI now
attaches a recorder, and a live switch is persisted. The frontend calls the seam after a
successful `apply_selection`, and the seam writes one `ModelChange` with the new provider
name. See `SPEC-the-interactive-session-records-itself` and
`D-the-interactive-recorder-lives-on-the-app`.

### 11a. Resume restores the recorded pair

A resume restores the provider and the model together. It reads both from the newest
`ModelChange`. So a session left on provider XXX and model YYY resumes on XXX and YYY.

The corrected read returns both fields. The old `stored_model` dropped the provider with
`..`, and that was the defect. This is the renamed replacement:

```rust
// crates/rho-cli/src/recording.rs

/// The newest provider and model the file states.
///
/// The newest `ModelChange` names the running pair. A resume rebuilds this provider and sets
/// this model together. `None` means the file holds no `ModelChange`, so a resume keeps the
/// resolved provider. See `D-resume-restores-the-recorded-provider`.
fn stored_selection(read: &rho_core::ReadResult) -> Option<StoredSelection> {
    read.entries
        .iter()
        .rev()
        .find_map(|entry| match &entry.record {
            Record::ModelChange { provider, model } => Some(StoredSelection {
                provider: provider.clone(),
                model: model.clone(),
            }),
            _ => None,
        })
}

/// The provider and the model a resume restores. Both come from one `ModelChange`, so they
/// can never split.
struct StoredSelection {
    provider: String,
    model: String,
}
```

The pattern `Record::ModelChange { provider, model }` names both fields. It carries no `..`.
So a reader can never drop the provider again.

### 11b. The flag wins over the record

A user may resume and also pass `--provider`. The flag wins. The flag is a present-tense
instruction, and the file is a record of the past. Absent the flag, the recorded provider
wins. Absent both, the resolved default stands.

```rust
// crates/rho-cli/src/recording.rs

/// The provider a resume runs on, and whether it came from the record.
///
/// The explicit `--provider` flag wins, because it is a present-tense instruction. Absent
/// the flag, the recorded provider wins, so a resume runs where the user left off. Absent
/// both, the resolved default stands. The `bool` says the name came from the record, so a
/// build failure names the record and never blames the flag. See
/// `D-resume-restores-the-recorded-provider`.
fn resume_provider(
    flag: Option<&str>,
    recorded: Option<&str>,
    default: &str,
) -> (String, bool) {
    match (flag, recorded) {
        (Some(name), _) => (name.to_string(), false),
        (None, Some(name)) => (name.to_string(), true),
        (None, None) => (default.to_string(), false),
    }
}
```

rho announces the override when the flag differs from the record. The notice says: you
passed `--provider <flag>`, so this resume runs on that provider. A second line names the
record: the recorded provider was `<recorded>`. A resume that keeps the recorded provider
announces nothing new.

### 11c. When the recorded provider cannot rebuild

A resume builds the recorded provider through `build_provider`. That is the same gate
startup and a live switch use. So a resume runs the same trust and credential checks. A
silent fallback to another provider is forbidden. Three cases split:

- **The `[[providers]]` entry is gone.** `build_provider` returns `ProviderError::Unknown`.
  The resume refuses. The message names the recorded provider. It tells the user to restore
  the entry or pass `--provider`.
- **The credential is gone.** `build_provider` returns `ProviderError::Credential`. The
  resume refuses. The message names the variable to set, and holds no secret. See
  `D-a-provider-names-its-own-credential`.
- **The base url now points elsewhere.** The file records no base url, so the current config
  is the only source. The provider still builds. The resume continues, and announces the
  base-url redirect the way startup announces it. So the user always sees where the key
  goes. See `D-a-provider-base-url-is-a-config-key`.

### 11d. The resume build error

The refusal in 11c has one named type. It names the recorded provider, then defers to the
build error, which already names the fix.

**A path constraint governs this type, and every new `rho-cli` file.** `rho-cli` has no
library target. It builds only the `rho` binary. So an integration test cannot import a
`rho-cli` module. A test reaches one instead with a `#[path = "../src/<file>.rs"]` module
include, exactly as `crates/rho-cli/tests/session_cli.rs` includes `recording.rs`. When a
file is included that way, `crate::` names the test binary's own root, not the `rho` binary
root. So a `crate::provider::ProviderError` path inside `recording.rs` fails to compile in
the test build, because the test root holds no `provider` module. Use a `super::`-relative
path. `super::provider::ProviderError` resolves against the parent module in both builds: the
`rho` binary and the test that includes `recording.rs` and `provider.rs` as sibling modules.
Every new `rho-cli` file that names another `rho-cli` module follows this rule.

```rust
// crates/rho-cli/src/recording.rs

/// Why a resume could not rebuild the provider the file records.
///
/// It names the recorded provider, then defers to the build error. The build error already
/// names the fix: an environment variable, `--trust-project`, or a feature to compile. It
/// holds no resolved credential value, because `ProviderError` holds none. `rho-cli` reports
/// it on the resume path, before the run spends a token. See
/// `D-resume-restores-the-recorded-provider`.
#[derive(Debug, thiserror::Error)]
#[error(
    "this session ran on provider {provider}, and rho cannot rebuild it: {source}. Pass \
     --provider <name> to resume on another provider."
)]
pub struct ResumeProviderError {
    /// The provider the newest `ModelChange` records.
    pub provider: String,
    /// The build error, which names the exact fix. `super::provider::ProviderError`, not
    /// `crate::provider::ProviderError`, so the type resolves in the `#[path]` test build.
    #[source]
    pub source: super::provider::ProviderError,
}
```

rho wraps a build error as `ResumeProviderError` only when the provider name came from the
record. When the name came from the flag, a build failure is the ordinary flag error,
because the user named that provider now.

### 11e. The trust rule

A recorded provider name is a name, not a grant. `rho-config` clears an untrusted project
`[[providers]]` entry at load. So an untrusted recorded name resolves to nothing, and the
build refuses it as an unknown provider. An untrusted project credential is refused too, and
the message names `--trust-project`. So a resume cannot rebuild a provider the user would not
authorise today. See `D-an-untrusted-clone-supplies-no-credential`,
`D-a-provider-names-its-own-credential`, and `D-trust-is-provenance-not-a-field-list`.

### 11f. A resume restores no permission

`D-resume-never-widens` guards the approval mode and the sandbox mode. A provider is neither
mode. So restoring a provider does not touch the widen order, and the mode check still runs
first and unchanged. A provider restore can only narrow. The current config may remove the
entry, revoke the credential, or drop trust. Each tighten refuses the build. So a resume
cannot gain a credential the current config denies. The provider restore has no
`--allow-widen`, because a provider has no total order to compare. A build failure refuses it
instead of an order check. That is where `D-resume-never-widens` does not apply.

### 11g. A file with no `ModelChange`

A well-formed file always holds a `ModelChange` on its second line. A truncated tail can
drop it. Then `stored_selection` returns `None`. The resume keeps the resolved provider and
model, and refuses nothing. It records a new `ModelChange` only when the resolved model
differs from the record, exactly as today.

### 11h. What an old rho does

An old rho reads only the model back. It drops the recorded provider in silence, and runs on
the flag or the default. That silent drop is the defect this amendment fixes. `ModelChange`
is unchanged, so no version bump is needed. Only the reader behaviour changes. A new rho
refuses an unknown recorded provider rather than answer from the wrong one.

## 12. What the contract forbids

- A second column for the maker. The maker is in the name, and the dim column is the
  provider.
- A silent drop of `ModelSelection::provider`. `apply_selection` builds or refuses when the
  field names a different provider.
- A fail-open default provider. An unbuildable name is a hard error, with no `Unknown`
  variant and no fallback.
- A half-applied switch. `apply_selection` builds before the atomic commit.
- The provider and the model in two mutexes. One `RunningState` mutex holds both, read once
  per turn.
- `Provider::id` as the running-provider identity. `RunningState::provider_name` holds the
  config name.
- A switch that sends a credential to an un-announced host. A switch inherits the base-url
  gate and the redirect notice.
- A drift between the header and the wire. `set_current_selection` mirrors the provider on
  success.
- A network listing at startup. Listing is lazy, on picker open.
- A shared cache file across providers. Each provider has its own file.
- A pending listing wave that runs after the picker closes. The cancel token stops it.
- An unbounded provider count, concurrency, or list size. Each has a named cap.
- A new session record type. `ModelChange` already carries the provider.
- A silent drop of the recorded provider on resume. `stored_selection` returns both fields,
  and a resume refuses an unknown recorded provider.

## 13. Test cases

Each test lives in this repo when the status flips to `delivered`. Each row names the file
and the assertion.

### `rho-core` — the selection and the swap

- `a_none_provider_selection_never_builds` — `crates/rho-core/tests/selection.rs`.
  `apply_selection` with `provider: None` returns `Ok` and never calls the factory, even
  when no factory is injected.
- `a_same_provider_selection_never_builds` — `crates/rho-core/tests/selection.rs`.
  `apply_selection` with `Some(running_name)` writes the model and never calls the factory.
- `a_switch_builds_and_swaps_the_provider` — `crates/rho-core/tests/selection.rs`. A session
  built with a real factory through `with_provider_factory`, then `apply_selection` with a
  new provider name, calls the factory once and `selection().provider` then names the new
  provider. A no-op builder fails this test, because the switch never happens.
- `selection_reads_the_provider_from_the_running_unit` — `crates/rho-core/tests/selection.rs`.
  `selection().provider` always names the running provider, from `RunningState`.
- `a_named_entry_keeps_its_id_and_re_pick_does_not_rebuild` —
  `crates/rho-core/tests/selection.rs`. A session named `xdent-claude` over an anthropic
  protocol provider reports `xdent-claude`, and `apply_selection` with `Some("xdent-claude")`
  builds nothing.

### `rho-core` — the failure path

- `a_failed_switch_keeps_the_current_provider` — `crates/rho-core/tests/selection.rs`. A
  factory that returns `Err` leaves `selection()` unchanged, and returns the error.
- `a_switch_without_a_factory_is_refused` — `crates/rho-core/tests/selection.rs`.
  `apply_selection` with a new provider and no factory returns `Err` and changes nothing.
- `a_provider_build_error_carries_no_secret` — `crates/rho-core/tests/selection.rs`. A
  `ProviderBuildError` from a credential failure holds no resolved credential value.

### `rho-core` — the turn boundary and atomicity

The session records a model change no longer. `SPEC-the-interactive-session-records-itself`
records it through the app seam, and `a_selection_step_records_a_model_change` in
`crates/rho-tui/tests/recording.rs` proves the record. So the earlier
`a_switch_records_a_model_change_with_the_new_provider` moves to that suite and is not a
test of this spec.

Both turn-boundary tests hold the turn open deterministically. The provider's `stream`
blocks on a `tokio::sync::Notify`, so the test controls the exact interleaving. No test
sleeps, and no test races a wall clock. See AGENTS.md step 5.

- `a_running_turn_keeps_its_provider_across_a_switch` — `crates/rho-core/tests/session_turn.rs`.
  A provider whose `stream` blocks on a `Notify` holds turn one open. A switch runs while the
  turn is blocked. The test then notifies, and turn one still streams from the first
  provider. The next turn uses the new provider. So a switch never changes the provider a
  running turn already read.
- `a_turn_never_pairs_a_new_provider_with_an_old_model` — `crates/rho-core/tests/session_turn.rs`.
  The interleaving is forced, not timed. The first provider's `stream` records the request
  model it received and its own provider id as one pair, then blocks on a `Notify`. While it
  blocks, the test applies a switch that changes both the provider and the model. The test
  then notifies. The recorded pair is the whole old pair, `(first provider id, old model)`,
  never a mixed pair, because `run_turn` reads the provider, the model, and the effort under
  one lock before it calls `stream`. A build that reads the model and the provider from two
  separate lock holds fails this test.

### `rho-cli` — the factory

- `the_factory_builds_a_known_provider` — `crates/rho-cli/tests/provider_factory.rs`. The
  factory builds a provider whose credential resolves, and returns `Ok`.
- `the_factory_maps_a_missing_credential_to_a_build_error` — `crates/rho-cli/tests/
  provider_factory.rs`. A missing credential returns `ProviderBuildError` whose message names
  the variable to set.
- `the_factory_refuses_an_untrusted_project_credential` — `crates/rho-cli/tests/
  provider_factory.rs`. An untrusted project credential returns `ProviderBuildError` whose
  message names `--trust-project`.
- `the_factory_error_holds_no_credential_value` — `crates/rho-cli/tests/provider_factory.rs`.
  The mapped error line never holds a resolved credential value.
- `a_switch_build_inherits_the_base_url_trust_gate` — `crates/rho-cli/tests/provider_factory.rs`.
  A build through the factory drops an untrusted project base url, and refuses a base url for
  a provider that names its endpoint its own way.

### `rho-cli` — the catalog source

- `provider_names_lists_each_provider_with_a_named_credential_once` —
  `crates/rho-cli/tests/catalog_source.rs`. `provider_names` names each built-in and each
  trusted `[[providers]]` entry that the configuration names a credential for, once, current
  provider first.
- `a_provider_with_no_named_credential_is_not_listed` —
  `crates/rho-cli/tests/catalog_source.rs`. A built-in the configuration names no credential
  for is absent from `provider_names`, and no call is made for it. See
  `D-a-provider-with-no-credential-is-not-listed`.
- `a_provider_with_a_named_credential_is_listed` —
  `crates/rho-cli/tests/catalog_source.rs`. A provider the configuration names a credential
  for is present in `provider_names`. A named credential is enough; the value is not
  resolved.
- `provider_names_resolves_no_credential_value` —
  `crates/rho-cli/tests/catalog_source.rs`. `provider_names` reads the configuration only. It
  runs no `!command`, reads no key file, and makes no network call.
- `an_untrusted_project_provider_is_not_named` — `crates/rho-cli/tests/catalog_source.rs`. An
  untrusted project `[[providers]]` entry is absent from `provider_names`.
- `provider_names_is_capped_at_max_providers` — `crates/rho-cli/tests/catalog_source.rs`. A
  config that names a credential for more than `MAX_PROVIDERS` providers yields exactly
  `MAX_PROVIDERS` names.
- `an_unbuildable_provider_catalog_is_an_error` — `crates/rho-cli/tests/catalog_source.rs`. A
  provider with a missing credential returns `Err` from `catalog`, with a secret-free line.
- `the_current_provider_catalog_is_not_rebuilt` — `crates/rho-cli/tests/catalog_source.rs`.
  `catalog` for the current provider reuses the session provider and resolves no credential.
- `two_concurrent_listings_both_persist` — `crates/rho-cli/tests/catalog_source.rs`. Two
  providers list at once, and both entries survive on disk, because each has its own file.

### `rho-cli` — resume restores the recorded provider

- `a_resume_restores_the_recorded_provider_and_model` — `crates/rho-cli/tests/
  resume_provider.rs`. A file whose newest `ModelChange` names `openrouter` and a model
  resumes on that provider and that model, with no `--provider` flag. A reader that drops the
  provider fails this test.
- `an_explicit_provider_flag_overrides_the_recorded_provider` — `crates/rho-cli/tests/
  resume_provider.rs`. A resume with `--provider bedrock` runs on `bedrock`, adds a notice
  that names the recorded provider, and writes one new `ModelChange` with `bedrock`.
- `a_resume_whose_recorded_provider_entry_is_gone_is_refused` — `crates/rho-cli/tests/
  resume_provider.rs`. A recorded `[[providers]]` name absent from the current config returns
  `ResumeProviderError` whose message names the recorded provider and `--provider`.
- `a_resume_whose_recorded_credential_is_gone_is_refused` — `crates/rho-cli/tests/
  resume_provider.rs`. A recorded provider with no credential returns `ResumeProviderError`
  whose message names the variable to set.
- `a_file_with_no_model_change_keeps_the_resolved_provider` — `crates/rho-cli/tests/
  resume_provider.rs`. A file with no `ModelChange` yields `stored_selection` of `None`, so
  the resume keeps the resolved provider and refuses nothing.
- `an_untrusted_recorded_provider_is_refused` — `crates/rho-cli/tests/resume_provider.rs`. A
  recorded name that names an untrusted project entry resolves to nothing, so the resume
  refuses and names `--trust-project` or the missing entry.
- `the_resume_provider_error_holds_no_credential_value` — `crates/rho-cli/tests/
  resume_provider.rs`. A `ResumeProviderError` from a credential failure holds no resolved
  credential value.
- `a_resume_on_the_recorded_provider_announces_a_base_url_redirect` — `crates/rho-cli/tests/
  resume_provider.rs`. A recorded provider whose current config sets a base url adds the same
  redirect notice startup adds.

### `rho-tui` — the row and the filter

- `a_row_draws_the_full_id_then_a_dim_provider` — `crates/rho-tui/tests/render.rs`. The row
  draws the full model id as the name, then the provider dim.
- `filtered_indices_matches_the_provider_id` — `crates/rho-tui/tests/model_picker.rs`. The
  query `bedrock` keeps every Bedrock row.
- `filtered_indices_matches_the_full_model_id` — `crates/rho-tui/tests/model_picker.rs`. The
  query `sonnet` keeps a sonnet row across providers.
- `a_current_row_is_the_running_model_and_provider` — `crates/rho-tui/tests/model_picker.rs`.
  The `(current)` tag marks the row whose id and provider both match the session.
- `a_catalog_row_carries_the_provider_that_listed_it` — `crates/rho-tui/tests/model_picker.rs`.
  `append_catalog_models` tags each row with the provider argument.
- `a_provider_with_no_catalog_still_contributes_id_rows` —
  `crates/rho-tui/tests/model_picker.rs`. `build_display` for a provider whose catalog is
  absent, such as Azure, still shows its current and starred rows, because those come from
  ids and not from a listing.

### `rho-tui` — the listing, the pick, and the header

**`crates/rho-tui/tests/app.rs` does not exist, and no cargo test can create it usefully.**
The event loop owns a terminal, and the coordinator, the cancel, the pick, and the header
mirror all live inside `select!` arms. No in-crate test reaches a `select!` arm. The
recording spec faced the same wall and solved it the honest way: it extracted the logic into
free functions a test drives, and named `docs/verification/` as the guard for what only a
live drive can reach. See `SPEC-the-interactive-session-records-itself` sections 1 and 14,
and the comment at `crates/rho-tui/src/app.rs` that reads "no test reaches a `select!` arm".

This spec follows that rule. Three pieces move out of the `select!` into free functions:

- **The listing coordinator.** A free `async fn` takes the provider names, a loader seam,
  the concurrency cap, the timeout, and a cancel token, and it sends `CatalogEvent` values on
  a channel. It builds no terminal. So a unit test drives it with a stub loader and a
  `tokio::time` pause, never a real clock.
- **The pick-to-selection map.** A free function maps a highlighted `PickerRow` and the
  running provider name to a `ModelSelection`. A foreign row yields a switch selection.
- **The switch apply.** The free `apply_selection(session, state, selection)` calls
  `Session::apply_selection`, and on `Ok` mirrors the running triple into the state, and on
  `Err` pushes one error row and mirrors nothing.

These tests are unit tests in `crates/rho-tui/src/app.rs`, because the coordinator, the map,
and `CatalogEvent` are private. They drive the extracted functions, so no terminal is
needed:

- `one_provider_failure_does_not_empty_the_picker` — `crates/rho-tui/src/app.rs`. The
  coordinator over a loader that fails one provider emits one `CatalogEvent::Error` for that
  provider and `CatalogEvent::Models` for the others. No failure empties the stream.
- `a_provider_with_no_catalog_gives_no_error_event` — `crates/rho-tui/src/app.rs`. The
  coordinator over an `Ok(None)` loader, such as Azure, emits no `Models` and no `Error` for
  that provider. Its id rows come from `build_display`, proved by
  `a_provider_with_no_catalog_still_contributes_id_rows` below.
- `at_most_max_concurrent_listings_run_at_once` — `crates/rho-tui/src/app.rs`. A loader that
  counts live calls, driven with more than `MAX_CONCURRENT_LISTINGS` providers, never sees
  more than that many calls in flight at once.
- `a_listing_that_exceeds_the_timeout_yields_a_note` — `crates/rho-tui/src/app.rs`. A loader
  that never returns for one provider, with `tokio::time` paused and advanced past
  `LIST_TIMEOUT`, yields one `CatalogEvent::Error` note for that provider and the stream ends.
- `closing_the_picker_cancels_pending_waves` — `crates/rho-tui/src/app.rs`. A cancel token
  fired before the pending wave starts stops the coordinator, so a counting loader records no
  call for a provider in a wave that never ran.
- `picking_a_foreign_row_applies_a_switch_selection` — `crates/rho-tui/src/app.rs`. The
  pick-to-selection map of a foreign row yields a `ModelSelection` whose provider is the
  row's provider, and of a current-provider row yields `provider: None`.
- `a_successful_switch_names_the_new_provider_in_the_header` — `crates/rho-tui/src/app.rs`.
  `apply_selection` with a session over a stub factory that returns `Ok` sets the state
  provider to the new provider id.
- `a_refused_switch_pushes_an_error_and_keeps_the_header` — `crates/rho-tui/src/app.rs`.
  `apply_selection` with a session over a stub factory that returns `Err` pushes one error
  row and leaves the state provider unchanged.

### `rho-tui` — what only a live drive guards

The wiring inside the `select!` arms owns the terminal, so a cargo test cannot prove it.
`docs/verification/switch-provider-live.md` is the named guard. Build the release binary and
drive it in a pty. Record the commands and the real output. These claims are the drive:

- The open-picker arm spawns the coordinator, so several providers list on open.
- The close-picker arm fires the cancel token, so a pending wave stops when the picker
  closes mid-load.
- The apply-selection arm calls `apply_selection`, so a picked foreign row switches the
  running provider and the header names the new provider at the next turn.
- A refused switch draws one error row and leaves the header on the running provider.

See section 14a.

### `rho-tui` — the starred binding

- `a_starred_id_binds_to_the_current_provider_when_it_offers_it` — `crates/rho-tui/tests/
  model_picker.rs`. A starred id offered by the current provider draws the current provider
  in its dim column.
- `a_starred_id_binds_to_a_listed_provider_that_offers_it` — `crates/rho-tui/tests/
  model_picker.rs`. A starred id offered only by a foreign provider binds to that provider, so
  a pick switches.

## 14. Migration

### The `rho-core` callers

- **`ModelSelection` gains a field.** Every struct-literal construction of `ModelSelection`
  breaks. A construction that feeds `apply_selection` adds `provider: None`. A caller that
  uses `ModelSelection::new` does not change, because `new` sets `provider: None`.
- **An expected value read back from `selection()` is not `provider: None`.** `selection()`
  returns `provider: Some(<running name>)`. So a test that compares `selection()` to a
  literal sets the expected provider to `Some(<id>)`, not `None`. The test
  `a_selection_reads_back_the_seed_from_the_config` in `crates/rho-core/tests/selection.rs`
  compares `selection()` to a literal. Its expected value gains `provider:
  Some("recording".to_string())`, because `RecordingProvider::id` is `"recording"` and no
  `with_provider_name` call overrides it.
- **`Session::set_selection` becomes `apply_selection`.** The method returns `Result<(),
  ProviderBuildError>`. Its production caller, `apply_selection` in
  `crates/rho-tui/src/app.rs`, handles the `Err`. Tests that called `set_selection(sel)`
  change to `session.apply_selection(sel).expect(...)`; a `None` provider never fails, so the
  change is mechanical.
- **`SessionInner` drops `provider` and `selection`, and gains `running`.** `with_config`
  wraps the provider in `RunningState`, seeds `provider_name` from `Provider::id`, and sets
  `provider_factory: None`. `run_turn` reads the unit once, and `build_request` takes the
  model and effort as arguments.
- **`Session` gains `provider_factory`.** It sits on `Session`, outside the `Arc`.
  `with_provider_factory` sets it with struct-update syntax. `with_provider_name` rebuilds
  `SessionInner`, like `with_queue`.
- **`rho-cli` seeds the provider name.** The session build in `crates/rho-cli/src/cli.rs`
  calls `session.with_provider_name(&provider_name)` before it hands the session to the App,
  so a named entry reports its own id.
- **The frontend records the model change, not the session.** `Session::apply_selection`
  writes no `ModelChange`. After a successful apply, the app seam `record_turn` writes one
  `ModelChange` with the running provider name, read from `session.selection()`. See
  `SPEC-the-interactive-session-records-itself`.

### The `rho-tui` callers

- **`App::with_catalog` becomes `App::with_catalog_source`.** The caller in
  `crates/rho-cli/src/cli.rs` passes an `Arc<dyn CatalogSource>` instead of one catalog.
- **`PickerRow` gains `provider`.** Every construction of `PickerRow`, including the
  starred-fallback push in `build_display`, sets it.
- **`CatalogEvent` changes shape.** Both variants gain a `provider` field. The event loop
  passes it into `append_catalog_models` and `seed_picker_with_cached_models`.
- **`append_catalog_models` and `seed_picker_with_cached_models` gain a `provider: &str`
  parameter.** Each row they build carries that provider.
- **`set_current_selection` mirrors the provider.** On `Ok` from `apply_selection`, the App
  re-reads `session.selection()` and calls `set_current_selection`, so the header and
  `guide_pages(&self.model, &self.provider)` read the new provider.
- **The picker labels change.** The dim column reads `PickerRow::provider`, not the id.
  `vendor_label` and `model_label` are removed. See the delivered-spec edits below.

### The delivered spec this change breaks

The delivered spec `SPEC-the-model-picker-groups-and-labels-rows` names tests that this
change contradicts or deletes. `check-spec-tests.py` enforces a delivered spec, so the
controller must amend that spec in the same change, and record each deliberate deletion in
`bench/deleted-tests.txt` with the commit and the reason. This spec does not edit that file.
The controller edits needed are in section 16.

An old caller of the session file needs no change. `ModelChange` is unchanged, so an old
reader parses a new file.

## 14a. Verification by driving

The `select!` arms and the coordinator spawn own the terminal. A cargo test cannot reach
them. So `docs/verification/switch-provider-live.md` is the named guard, exactly as the
recording spec used `docs/verification/` for wiring a cargo test could not see. Build the
release binary and drive it in a pty. Record the commands and the real output.

- **The picker lists several providers on open.** Configure two providers with a named
  credential. Open the picker. Assert rows from both, each with its dim provider label.
- **A switch takes effect at the next turn.** Pick a foreign row. Prompt again. Assert the
  header names the new provider, and the turn ran on it.
- **A refused switch keeps the running provider.** Pick a row whose credential is missing.
  Assert one error row, and a header still on the running provider.
- **Closing the picker cancels a pending wave.** Open the picker with many providers, then
  close it mid-load. Assert no further listing starts.

## 15. Out of scope

- **A new session record type.** `ModelChange` already carries the provider.
- **Wiring a recorder onto the interactive TUI path.** `SPEC-the-interactive-session-records-itself`
  owns that wiring. This spec relies on it, and records a switch through that seam.
- **A `provider/id` pair in the starred file.** The binding is derived at display time. A
  file format change needs its own decision. See section 10.
- **An async provider factory.** The build is synchronous. See
  `D-the-core-takes-a-provider-factory`.
- **Moving the switch build off the event loop.** The build is synchronous and inline. A
  `!command` credential can freeze the terminal for up to the credential timeout. See
  section 2.
- **Per-provider sub-headers in the catalog section.** The dim column names the provider on
  each row, so the section keeps one `models` header.
- **Changing the session root, the approval policy, or the sandbox mid-session.** Only the
  model, the effort, and the provider are mutable. See
  `D-the-provider-is-mutable-behind-a-mutex`.
- **A change to `ModelDescriptor`.** The provider comes from the catalog it lists in, not
  from the descriptor. See `D-a-model-descriptor-carries-no-capability-claim`.

## 16. The edits the older spec needs

`SPEC-the-model-picker-groups-and-labels-rows` is delivered, and this change contradicts
part of it. The controller owns that file. It needs these edits when this spec's code lands.

### The two direct contradictions

- `filtered_indices_drops_a_full_id_only_match` asserts a full-id-only match returns empty.
  The new filter matches the full id, so this test now asserts the opposite. Delete it, or
  rewrite it to assert a full-id query keeps its row.
- `the_drawn_rows_match_filtered_indices_for_a_divergent_query` builds a query the full id
  matches but the model name and vendor do not. That divergence no longer exists, because the
  filter matches the full id. Delete it, or rewrite it for a provider-id divergence.

### The tests that lose their subject

These test `vendor_label` or `model_label` or the dim vendor column, which this change
removes. The controller deletes each, and records each in `bench/deleted-tests.txt`.

- `vendor_label_reads_the_slash_prefix`
- `vendor_label_reads_the_dot_prefix`
- `vendor_label_skips_a_bedrock_region_prefix`
- `vendor_label_is_empty_without_a_vendor`
- `vendor_label_returns_the_raw_unsafe_segment`
- `model_label_strips_the_vendor_and_region`
- `filtered_indices_matches_the_vendor_label`
- `a_row_draws_the_name_then_a_dim_vendor`
- `an_empty_vendor_draws_no_separator`
- `a_narrow_row_drops_the_vendor_first`
- `the_vendor_is_sanitized_before_it_draws`
- `a_region_only_id_never_draws_an_empty_name`
- `the_vendor_separator_is_exactly_two_spaces`
- `the_vendor_separator_width_is_measured_as_it_is_drawn`

The model-name sanitisation test stays useful, because the full id is still untrusted. The
controller keeps `the_model_name_is_sanitized_before_it_draws`, and points it at the full-id
name column.

### The prose edits

- Section 8's match-rule sentence changes from the model name or the vendor label to the
  full model id or the provider id.
- The row-layout section changes the dim column from a vendor label derived from the id to
  the provider id from `PickerRow::provider`.
- The `## Out of scope` line "Cross-provider listing" is removed, because this spec builds it.

## 17. The `F-model-picker` row text the controller should apply

The controller owns `docs/features.md`. This is the replacement row text for `F-model-picker`.

> | F-model-picker | Model picker (`/model`) | The `/model` slash command opens a panel over the composer. The panel lists the current model and every id in `~/.rho/starred-models.toml`. It also lists models from every configured provider that can list. That means each built-in and each trusted `[[providers]]` entry. Starred models draw in their own section at the top, under a `starred` header. The catalog draws below, under a `models` header. A section header sticks while the list scrolls. A header is never selectable. Each row draws the full model id as its name. So the maker stays in the name. A dimmed provider id follows, such as `bedrock` or `openrouter`. Listing is lazy on open, bounded to 16 providers, 4 concurrent loads, and a 10-second timeout each. Closing the picker cancels the pending loads. One provider's failure adds a note and never empties the picker. A provider that cannot list, such as Azure, still contributes id rows. Every printable key types into a fuzzy filter. A row survives when the query matches its full model id or its provider id as a case-insensitive subsequence. Arrow keys move the selection over the filtered rows. Enter applies the highlighted row, or the query verbatim when no row matches. Picking a model from another provider rebuilds the session provider in place, and takes effect at the next turn. A build failure is refused, and the session keeps its provider. `Tab` cycles the preview effort of the highlighted row. `Shift+Tab` and `Ctrl+S` toggle a star and write the file at once. `Backspace` removes a query character. `esc` closes with no change. `/model <id>` and `/effort <level>` apply directly and never open the panel. `/speed fast` sets effort off; `/speed normal` restores the start effort. A running turn is not mutated. A stale cached list still shows, marked stale, while a fresh list loads. See `SPEC-model-selection-in-tui`, `SPEC-choose-a-model-and-configure-a-run`, `SPEC-the-model-picker-groups-and-labels-rows`, and `SPEC-switch-the-provider-mid-session`. | `rho-core`, `rho-tui`, `rho-cli` | A new provider: add a `Provider` impl, a `build_provider` branch, and a factory entry. The picker lists it and a switch reaches it. No edit to `Session` or the picker is needed. A new effort level: extend `ReasoningEffort`. See `D-the-core-takes-a-provider-factory` and `D-the-picker-lists-every-configured-provider`. |
