# D-rho-jsonl-asks-a-factory-for-a-session — the frontend links no provider

Date: 20260826. Reference: `D-rho-jsonl-asks-a-factory-for-a-session`.
Spec: `docs/specs/20260819-102749-SPEC-jsonl-frontend.md`, section 9.

## The question

Two commands in the protocol need a **new** session: `set_model` and `new_session`. Building
a session needs a `Provider`, and building a provider needs a credential and an HTTP client.
`rho-jsonl` depends on `rho-core` only, and `rho-core` links no HTTP client. So who builds
the session?

## The decision

`rho-jsonl` declares one trait and never builds a session itself:

```rust
#[async_trait]
pub trait SessionFactory: Send + Sync {
    async fn build(&self, request: &SessionRequest) -> Result<Session, FactoryError>;
    fn providers(&self) -> Vec<String>;
}
```

`rho-cli` implements it, because `rho-cli` already owns provider construction, the config
merge, the tool registry, and the skill loader. `rho-jsonl` holds an `Arc<dyn SessionFactory>`
and calls it.

This keeps three rules true at once. `rho-jsonl` depends on `rho-core` only. `rho-core` keeps
no HTTP dependency. No crate in `crates/` depends on `rho-cli`, because the dependency points
the other way: `rho-cli` depends on `rho-jsonl`.

## Why this is the extension point

AGENTS.md step 3 asks what a third party adds without a fork. This is the answer, and it is
a real one. An embedder writes its own `SessionFactory`, calls `rho_jsonl::serve`, and gets
the whole protocol over any pair of streams. It chooses its own providers, its own tools, and
its own approval policy. It edits no shared code and adds no enum variant.

`FactoryError` is the error taxonomy of that seam. It has three cases: an unknown provider, a
missing credential, and a refused model. Each one maps onto a named `ReplyError`, so a client
learns which of the three happened.

## Why not the alternatives

- **Let `rho-jsonl` depend on the provider crates.** A frontend would then pin the provider
  set, and the minimal build would carry every HTTP client. It also breaks the product rule
  that a provider is optional.
- **Take one `Session` and forbid `set_model`.** Then two of the six feature rows are
  unbuildable, and `F-jsonl-session-commands` is a promise with no code.
- **Take a closure instead of a trait.** A closure cannot answer `providers()`, which
  `get_state` needs, and a second closure per question is a trait with worse names.

## What it rules out

- No provider crate in the `rho-jsonl` dependency list. Ever.
- No `Session` construction inside `rho-jsonl`.
- No `rho-jsonl` knowledge of a credential, a base URL, or a config key.
