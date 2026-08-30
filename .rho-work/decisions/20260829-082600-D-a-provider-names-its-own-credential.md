# D-a-provider-names-its-own-credential — the fallback variable belongs to the builder

**Question:** a provider asks `Config` for its credential. No user has a `[credentials]`
table today, so an absent entry must still work. Where does the fallback live?

`.rho-work/i15-credential-expansion.md` parked this as U3. Its stated default was **U3(a)**:
`rho-config` seeds a default `CredentialSource::Env` for each known provider.

**Decision: not U3(a). The provider builder names its own fallback variable, and passes it
to one new `Config` method.** So `build_openrouter` asks for the name `openrouter` with the
fallback `OPENROUTER_API_KEY`, and `build_azure` asks for `azure` with
`AZURE_OPENAI_API_KEY`.

```rust
let secret = config.resolve_credential_or_env(name, fallback_var, env)?;
```

## Why the parked default is wrong

`SPEC-config-call-site` froze this extension point: "a new provider credential needs no edit
to shared code, because `credentials` is a map from a provider name to a `CredentialSource`."

U3(a) breaks that. A seed table inside `rho-config` is shared code. A fourth provider would
have to edit `rho-config` to name its variable, and `rho-config` does not own a provider.
The spec is the contract, and the contract wins over a parked default.

U3(a) also puts provider knowledge in the wrong crate. `rho-config` knows nothing about
Azure today. It must not learn that Azure spells its key `AZURE_OPENAI_API_KEY`.

## Why it is not U3(b) either

U3(b) let `provider.rs` fall back **after** a `Credential` error. That reads an environment
variable at the call site, which section 2 of the spec forbids. It also loses the trust
gate: a `RefusedProjectCommand` error would fall back to an environment read and hide the
refusal.

The chosen shape keeps one resolve path. `rho-config` builds the fallback as a
`CredentialSource::Env` and resolves it through the same code, so the gate, the `Secret`
type, and the error taxonomy all still hold.

## Rules out

**A seed table of provider names inside `rho-config`.** It breaks the frozen extension
point and puts provider knowledge in the config crate.

**A naming rule, such as `<PROVIDER>_API_KEY`.** Azure spells its key
`AZURE_OPENAI_API_KEY`, so a rule would be wrong for the second provider rho ships.

**A fallback after the error.** The refusal of an untrusted project command would then read
as an absent entry, and the command gate would teach the user nothing.

**Making a `[credentials]` entry required.** That breaks every user who sets
`OPENROUTER_API_KEY` today, which is the U2(a) default and it stands.

## What it costs

One new method on `Config`. `resolve_credential` keeps its meaning: an entry by that name,
or an error. The new method adds one fallback, and it is the only method a provider calls.
