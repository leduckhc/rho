# D-the-picker-lists-every-configured-provider

Date: 20260918-204102

## The question

The `/model` picker lists models. Today it lists one provider, the session's own. The user
wants it to list models from every built-in and every `[[providers]]` entry, each row
labelled with its provider. How many providers list at once, is listing eager or lazy, what
does a user with ten providers pay, and what happens when one provider fails?

## The decision

`rho-tui` owns a small trait, `CatalogSource`. `rho-cli` implements it. The picker asks it
for the provider names, then for each provider's catalog, lazily, only when the picker
opens.

```rust
// crates/rho-tui/src/app.rs
pub trait CatalogSource: Send + Sync {
    /// The provider ids to list, current provider first. Cheap: it reads config, builds
    /// nothing, and resolves no credential.
    fn provider_names(&self) -> Vec<String>;

    /// Build or reuse the catalog for one provider. `Ok(None)` means the provider cannot
    /// list, such as Azure. `Err` holds one secret-free line. This resolves a credential,
    /// so the App calls it inside a spawned task, never on the event loop.
    fn catalog(
        &self,
        provider: &str,
    ) -> Result<Option<std::sync::Arc<dyn rho_core::ModelCatalog>>, String>;
}
```

### Lazy, so an unopened picker costs nothing

`provider_names` is cheap and needs no build. `catalog` builds the provider and resolves
its credential, so the App calls it only when the picker opens, and only inside a spawned
task. A user who never opens the picker resolves no foreign credential and makes no network
call. This corrects an earlier draft that built every provider at `App` construction. That
draft resolved every provider's credential at every launch, including a `!command`
subprocess, even for a picker the user never opened. It called that build "cheap", which
was an unmeasured claim `AGENTS.md` forbids.

The `rho-cli` implementation reuses the session's already-built provider for the current
name. So the current provider is never rebuilt and its credential is never re-resolved.

### The set

`provider_names` returns the providers rho would build for this session: the built-ins this
binary includes, plus every `[[providers]]` entry from a trusted config layer. An untrusted
project `[[providers]]` entry is not in the set, because `rho-config` already clears it at
load. See `crates/rho-config/src/lib.rs` line 920 and
`D-an-untrusted-clone-supplies-no-credential`.

### The bounds

- `MAX_PROVIDERS = 16`. `provider_names` returns at most this many, and drops the rest with
  a note. So the picker's memory is bounded whatever the config holds.
- `MAX_CONCURRENT_LISTINGS = 4`. A coordinator task holds a semaphore of four permits. At
  most four network loads run at once, so ten providers run in three waves, not ten at once.
- `LIST_TIMEOUT` from `rho-core`, ten seconds, bounds one listing. The App wraps each load
  in that timeout. A slow provider yields a note, not a hung picker.
- `MAX_MODELS` from `rho-core`, two thousand, bounds one listing's row count.

### One cache file per provider

`rho-cli` gives each provider its own cache file, named by the provider fingerprint, for
example `model-catalog-cache-<fingerprint>.json`. So two concurrent listings never share a
file. An earlier draft wrapped every provider in a `CatalogCache` that wrote one shared
file, guarded by a per-instance lock. Four instances held four locks, so a read-modify-write
lost updates: A read the empty file, B read the empty file, A wrote its entry, B overwrote
it. The per-provider file removes the shared write, so no update is lost.

### The concurrency and cancellation contract

- The App holds one `catalog_cancel` token for the whole picker session.
- On open, the App spawns one coordinator task. The coordinator reads `provider_names`,
  acquires a semaphore permit, spawns one load, and repeats.
- Each load calls `catalog(name)`, then `list_models` under `LIST_TIMEOUT`, and selects on
  `catalog_cancel.cancelled()`.
- Every load reports into one channel. Every event names its provider, so a late load
  appends to the right rows and a failure marks only its own provider.
- Closing the picker fires `catalog_cancel`. In-flight loads stop, and the coordinator
  stops launching the pending waves. So closing the picker cancels the queued work.

### The partial-result rule

One provider's failure never empties the picker. A failed or timed-out load adds one note
row for that provider, and the other providers' rows still show. A provider with no
catalog, such as Azure, contributes its current, starred, and suggested rows, because those
come from ids, not from a listing.

### The row

The row draws the full model id as its name, so the maker stays in the name, for example
`anthropic/claude-3.5-sonnet`. The dim column draws the provider id, for example `bedrock`.
Every catalog row carries the provider that listed it, so `append_catalog_models` and
`seed_picker_with_cached_models` each take a provider argument. The fuzzy filter matches the
model id or the provider id, so a provider search works. A row from a provider that is not
the session's current provider is marked only by that dim provider label. Picking it
switches the session.

## What this rules out

- **Eager build at startup.** It resolves every provider's credential for a picker the user
  may never open.
- **A shared cache file across providers.** Concurrent listings lose updates on it.
- **Unbounded providers, concurrency, or list size.** Each has a named cap.
- **A failure that empties the picker.** A partial result is the rule, with a per-provider
  note.
- **A pending wave that runs after the picker closes.** The cancel token stops it.
- **A catalog row with no provider.** The provider is threaded from the event into the row.
- **A second column for the maker.** The maker is in the name, and the dim column is the
  provider. This reverses the split from `D-a-picker-row-labels-its-vendor`.

## Why

Listing across providers must stay cheap until it is wanted, bounded so a big config is
safe, and robust so one bad provider does not hide the good ones. A lazy source, per-provider
cache files, a capped coordinator, and a shared cancel token deliver each of those.
