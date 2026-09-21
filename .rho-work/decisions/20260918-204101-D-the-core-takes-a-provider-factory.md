# D-the-core-takes-a-provider-factory

Date: 20260918-204101

## The question

A mid-session switch needs a new `Arc<dyn Provider>`. Building one reads the merged config,
resolves a credential, and constructs an HTTP or an AWS client. `rho-core` must never gain
an HTTP or a credential dependency. That is a product rule in `AGENTS.md`. So the core
cannot build a provider. How does the core get a new provider without that dependency?

## The decision

`rho-core` owns a small trait, `ProviderFactory`. The `Session` holds it as an optional
trait object, and `apply_selection` calls it to build the new provider during a switch.

```rust
pub trait ProviderFactory: Send + Sync {
    /// Build the provider named `name`, or say why it cannot build.
    fn build(&self, name: &str) -> Result<std::sync::Arc<dyn Provider>, ProviderBuildError>;
}

/// Why a provider switch could not build a provider. It holds one line and no secret.
#[derive(Debug, Clone, thiserror::Error)]
#[error("cannot switch provider: {0}")]
pub struct ProviderBuildError(pub String);
```

`rho-cli` owns the one implementation. It wraps `build_provider` with the merged `Config`
and an `EnvLookup`, both captured when the binary built the factory. So all provider,
credential, and HTTP knowledge stays in `rho-cli`, behind the trait object. The core sees
only `dyn ProviderFactory`.

### The factory lives on `Session`, not on `SessionInner`

The factory field sits on `Session`, next to `recorder`, outside the `Arc<SessionInner>`.
Only `apply_selection` reads it, and the driver never does, so it does not belong in the
shared inner. This placement makes the builder trivial and correct:

```rust
pub fn with_provider_factory(self, factory: std::sync::Arc<dyn ProviderFactory>) -> Self {
    Self {
        provider_factory: Some(factory),
        ..self
    }
}
```

An earlier draft put the field on `SessionInner` and wrote a builder body that returned
`self` unchanged. That body dropped the factory, so every switch was refused: a dead
switch. A field inside an already-formed `Arc<SessionInner>` cannot be set by a move, and a
rebuild by `Arc::try_unwrap` panics once the session is shared. The field on `Session`
avoids both faults, and it mirrors `with_recorder`, which also sets a `Session` field.

### It is synchronous

`build` is synchronous, not `async`. `build_provider` is synchronous today. It resolves an
in-memory `Config` and an `EnvLookup`, and it constructs a client with no network call. A
synchronous trait method is object-safe with no `async_trait` boxing, and it keeps
`apply_selection` synchronous and `&self`.

`apply_selection` runs inline on the frontend's async event loop. So a synchronous build
runs there too. One credential form blocks: a `!command` credential runs a shell
subprocess. That subprocess is bounded, because `Config` resolves a credential command
under a 30-second timeout. See `D-credential-command-allowlist`. So a switch to a provider
with a `!command` credential can freeze the terminal for up to that timeout.

Most credentials do not block. An `env:` credential, a literal, and the Bedrock AWS chain
resolve in memory. So the freeze is the rare case, and it is bounded. This decision accepts
the bounded freeze rather than split the build from the commit, because splitting doubles
the switch surface. A frontend may move the build onto a blocking task in a later change.
This is a known limit, and it is written here, not left silent.

### The error carries no secret

`ProviderBuildError` holds one line and nothing else. The `rho-cli` implementation maps
`ProviderError` to this type through its `Display`, and `ProviderError` never puts a
credential in a message. A test already pins that: `a_credential_error_never_holds_the_
resolved_value`. So the mapped line is safe to show.

## What this rules out

- **A provider builder inside `rho-core`.** It would drag an HTTP and a credential
  dependency into the core and break a product rule.
- **A concrete builder type in the core.** A trait object keeps the core open for a new
  provider crate with no edit to the core. See `D-provider-contract-crate`.
- **The factory on `SessionInner`.** A field behind the shared `Arc` cannot be set by the
  builder shape this uses, and it made the shipped draft a silent no-op.
- **An `async` factory method.** It would force an `await` into the swap for no gain the
  synchronous path lacks.
- **A fail-open build.** There is no `Unknown` or default provider fallback. An unbuildable
  name is a hard error. See `D-a-provider-switch-that-cannot-build-is-refused`.
- **A factory that also lists providers.** Listing is a picker concern, and it uses a
  separate `rho-tui` trait, `CatalogSource`. See
  `D-the-picker-lists-every-configured-provider`.

## Why

The core needs one capability it cannot hold itself: turn a name into a provider. A trait
object is the least it can take. The name-to-provider knowledge already lives in `rho-cli`,
in `build_provider`, so the implementation is a thin wrapper and every credential and trust
rule holds unchanged.
