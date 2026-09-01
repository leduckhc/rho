# SPEC-named-provider-profiles — config entries for protocol-port pairs

Status: draft

Date: 20260901-192553.  
Related: `D-a-provider-is-named-by-its-wire-protocol`, `D-a-provider-names-its-own-credential`.

## The problem

The xdent tunnel speaks two protocols. The user wants five named entries. Each names a protocol, a base URL, and a credential. The command line says `--provider xdent-claude`.

## The public API

### 1. The TOML shape

One new table in `rho-config`, a repeatable array. Each entry names a provider id, a protocol, a base URL, and a credential reference.

```rust
/// One named provider entry. It lives in `rho-config`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct ProviderEntry {
    /// The name the user calls with `--provider <id>`.
    pub id: String,
    /// The wire protocol: "anthropic", "openai-chat", "openai-responses".
    pub protocol: String,
    /// The provider endpoint, for example `http://127.0.0.1:58788/dev1/anthropic`.
    pub base_url: String,
    /// The credential name that resolves in `Config::credentials`.
    pub credential: String,
}
```

The `ConfigLayer` struct gains one field:

```rust
#[serde(default)]
pub providers: Vec<ProviderEntry>,
```

**Example config:**

```toml
[[providers]]
id = "xdent-claude"
protocol = "anthropic"
base-url = "http://127.0.0.1:58788/dev1/anthropic"
credential = "xdent-key"

[[providers]]
id = "xdent-gpt"
protocol = "openai-chat"
base-url = "http://127.0.0.1:58788/chat/openai/v1"
credential = "xdent-key"

[credentials]
xdent-key = "env:XDENT_API_KEY"
```

### 2. Name collision rule

**A named entry never overrides a built-in.** Built-ins are `bedrock`, `azure`, `openrouter`. A config entry with the id `bedrock` is refused at load time. The error names the id and says to rename it.

A built-in is safe code. An override is a base URL. Fail-safe means refuse the override.

### 3. Trust boundary

**A project-file provider entry is refused when untrusted.** The `credential` field resolves after the trust gate. It resolves against the user-global credentials table. A project file gains no new name. It gains no path to a global credential the user did not set.

The existing rule holds: an untrusted credential table entry becomes `RefusedProjectCredential`. A provider entry is a capability, the same as a skill path. An untrusted project supplies none.

**When trusted,** the entry is kept. The `credential` field is a name. It resolves in `Config::credentials`, which is the merged table from all layers. A project file may not **add** a credential, per `D-an-untrusted-clone-supplies-no-credential`. It may name one, and the name resolves in the user's own table.

### 4. Bounds

| Bound | Value | Reason |
|-------|-------|---------|
| Max entries | 64 | A user manages one proxy. 64 routes is generous. Past that, enumerate. |
| Max `id` length | 128 bytes | A CLI flag. 128 UTF-8 bytes is generous. |
| Max `protocol` length | 64 bytes | A known set today. Room for a fourth. |
| Max `base-url` length | 2048 bytes | A URL. 2048 is the common browser limit. |
| Max `credential` length | 128 bytes | A name in the credentials map. |
| Total added to `MAX_CONFIG_BYTES` | The table is a vector. The 1 MiB cap already covers it. | A config file is read under that cap. |

A value over its bound is refused at parse time, per `D-a-limit-too-large-is-refused-not-clamped`.

### 5. Resolution algorithm

Given `--provider xdent-claude`:

```
1. Search built-ins: ["bedrock", "azure", "openrouter"].
   If found, build that provider. Stop.

2. Search `Config::providers` for an entry where `entry.id == name`.
   If not found, return UnknownProvider. Stop.

3. Resolve the credential:
   credential_source = Config::credentials.get(entry.credential)
   If absent, return MissingCredential(entry.credential). Stop.
   secret = credential_source.resolve(env)?

4. Match on entry.protocol:
   "anthropic"        -> rho_provider_anthropic with (entry.base_url, secret)
   "openai-chat"      -> rho_provider_openai_chat with (entry.base_url, secret)
   "openai-responses" -> rho_provider_azure with (entry.base_url, secret)
   _                  -> return UnknownProtocol(entry.protocol)

5. Return the built provider.
```

The built-ins are searched first, so a named entry cannot shadow one.

### 6. Extension point

**A new protocol is a new crate, plus one match arm.** The `protocol` field is a string. A fourth protocol edits the match in step 4 and adds its own crate. The shared code edit is one arm, listed in this spec as the place to add.

A trait with `fn build(base_url: String, secret: Secret) -> Arc<dyn Provider>` would eliminate the arm. That live in `rho-core`, which is out of scope here. The match arm is stated as the extension point, and a trait is the future.

### 7. Error taxonomy

```rust
pub enum ProviderError {
    /// A named entry collides with a built-in.
    #[error(
        "the provider entry \"{id}\" collides with the built-in {id} provider. \
         Rename the entry in your config file."
    )]
    BuiltinCollision { id: String },

    /// The protocol name is unknown.
    #[error(
        "the protocol \"{protocol}\" is not known. Choose one of: \
         anthropic, openai-chat, openai-responses."
    )]
    UnknownProtocol { protocol: String },

    /// The base URL is missing from a named entry.
    #[error(
        "the provider entry \"{id}\" has no base-url. Add one in your config file."
    )]
    MissingBaseUrl { id: String },

    /// The credential name does not resolve.
    #[error(
        "the credential \"{name}\" is not defined. Add it to [credentials] \
         in your global config, or set the variable it names."
    )]
    MissingCredential { name: String },

    /// A config [[providers]] entry, from a project file, when untrusted.
    #[error(
        "the project file defines a provider entry, and the project is not trusted. \
         Pass --trust-project to allow it."
    )]
    UntrustedProjectProvider,

    /// Too many provider entries.
    #[error(
        "the config defines {count} provider entries. The limit is {limit}. \
         Keep the ones you use."
    )]
    TooManyProviders { count: usize, limit: usize },

    /// A field is too long.
    #[error(
        "the {field} field is {length} bytes. The limit is {limit} bytes."
    )]
    FieldTooLong {
        field: &'static str,
        length: usize,
        limit: usize,
    },

    // Existing variants remain.
}
```

### 8. How the file gets written

**Out of scope: a subcommand to edit the file.** The user hand-edits `~/.config/rho/config.toml`. A `rho providers add` command is a future convenience, and a half-built subcommand ships nothing.

## Test cases

Each test name states its assertion.

1. `a_named_entry_resolves_by_id` — a `[[providers]]` entry with `id = "xdent-claude"` resolves when the user passes `--provider xdent-claude`.
2. `a_built_in_is_found_before_a_named_entry` — a built-in provider is never shadowed by a config entry of the same name.
3. `a_collision_with_a_built_in_is_refused` — a `[[providers]]` entry with `id = "bedrock"` fails at load time with `BuiltinCollision`.
4. `protocol_anthropic_builds_the_anthropic_provider` — `protocol = "anthropic"` resolves to `rho_provider_anthropic`.
5. `protocol_openai_chat_builds_the_openai_chat_provider` — `protocol = "openai-chat"` resolves to `rho_provider_openai_chat`.
6. `an_unknown_protocol_is_refused` — `protocol = "nope"` returns `UnknownProtocol`.
7. `the_base_url_reaches_the_provider` — the `base-url` value reaches the crate's config.
8. `the_credential_resolves_from_the_credentials_table` — the `credential` field names an entry in `[credentials]`, and that entry resolves.
9. `a_missing_credential_name_is_refused` — a credential name with no entry returns `MissingCredential`.
10. `the_credential_passes_the_trust_gate` — an untrusted project file cannot name a credential in a provider entry.
11. `an_untrusted_provider_entry_is_refused` — a `[[providers]]` entry from an untrusted project file returns `UntrustedProjectProvider`.
12. `a_trusted_provider_entry_is_kept` — a `[[providers]]` entry from a trusted project file resolves.
13. `an_id_over_128_bytes_is_refused` — the `id` field is capped at 128 bytes.
14. `a_protocol_over_64_bytes_is_refused` — the `protocol` field is capped at 64 bytes.
15. `a_base_url_over_2048_bytes_is_refused` — the `base-url` field is capped at 2048 bytes.
16. `a_credential_name_over_128_bytes_is_refused` — the `credential` field is capped at 128 bytes.
17. `more_than_64_entries_is_refused` — the `[[providers]]` array is capped at 64 entries.
18. `a_named_entry_with_no_base_url_is_refused` — a missing `base-url` returns `MissingBaseUrl`.
19. `two_entries_may_share_one_credential` — two `[[providers]]` entries may name the same credential.
20. `a_named_entry_is_never_the_default` — unset `--provider` resolves to the build default, never a named entry.

## Out of scope

- A subcommand to edit the file. The user hand-edits.
- Runtime provider registration. A provider is a config entry.
- Per-provider retry overrides. Retry policy is global.
- A default model field on the entry. A model is a CLI flag.
- Custom headers. A future addition, when a real proxy needs one.

## Amendments after the contract review, binding

A contract review returned two blockers here, tied to reconciliation with the other two
specs in this batch. Where an amendment disagrees with the text above, the amendment wins.

### 1. Load-time errors go in `ConfigError`

The spec named `BuiltinCollision`, `UnknownProtocol`, `TooManyProviders`, and four others as
new `ProviderError` variants. That contradicts the spec's own decision
`D-a-built-in-is-never-shadowed`, which says the error is a `ConfigError`. It also invents
a third `ProviderError` type where two already sit at load, and it edits a shared exhaustive
enum for every new load-time defect.

These are load-time errors on the config layer, so they belong on `ConfigError`, in
`rho-config`. `ProviderError` stays the wire taxonomy. The reuse rule holds: `Unknown{name}`
on `rho-cli::ProviderError` already covers the runtime lookup.

The load-time variants added to `ConfigError`:

```rust
BuiltinCollision   { id: String }
UnknownProtocol    { protocol: String }
MissingBaseUrl     { id: String }
MissingCredential  { id: String, credential: String }
UntrustedProjectProvider { id: String }
TooManyProviders   { limit: usize, seen: usize }
FieldTooLong       { field: &'static str, limit: usize, seen: usize }
```

### 2. The credential resolution site is the named-entry lookup, once

The reviewer found two resolution paths and no precedence rule. Rewritten as one:

- A **built-in** provider (`bedrock`, `azure`, `openrouter`, `anthropic`, `openai-chat`,
  and whatever the built-in list is when a launcher runs) resolves through
  `Config::resolve_credential_or_env(name, env_var)`. The env var is the fallback.
- A **named-entry** provider resolves through `Config::credentials.get(entry.credential)`.
  The env var is **not** a fallback. A named entry must name a real credential.

So precedence is: `--provider xdent-claude` goes through the named lookup; `--provider anthropic`
goes through the built-in path with env fallback. A user who wants an env fallback for their
named entry adds a `[credentials.xdent-key] type = "env" name = "XDENT_API_KEY"` block, per
the existing shape.

Both providers then take an already-resolved `rho_core::Secret`, matching the amendment on
`SPEC-anthropic-messages-provider`.

### 3. The crate reference names `rho-provider-openai-chat`

Section 5 step 4 now reads:

```
"anthropic"        -> rho_provider_anthropic with (entry.base_url, secret)
"openai-chat"      -> rho_provider_openai_chat with (entry.base_url, secret)
"openai-responses" -> rho_provider_azure with (entry.base_url, secret)
```

`rho-provider-openai-chat` is the renamed `rho-provider-openrouter`, per
`D-provider-openai-chat-owns-the-wire`.

### 4. The shadow precedence test is replaced

Test 2, `a_built_in_is_found_before_a_named_entry`, was vacuous: test 3 already refuses a
same-named entry at load, so the precedence branch it tests can never be constructed. It is
replaced with `a_named_entry_with_a_builtin_id_is_refused_at_load`, which asserts the load
failure directly.

### 5. Numbers marked provisional

The 64 max entries, the 2048-byte URL cap, and the 128-byte id cap are provisional. The 2048
number came from a browser URL limit that does not apply to a proxy path. A later
measurement takes precedence.

### 6. The extension point stays a shared match arm, for now

The reviewer flagged step 4 as a shared-code edit per new protocol. That is real. This spec
keeps the match, because the choice is between an edit to one file per new protocol and a
dynamic registry the project does not need for three protocols today. When a fifth
protocol arrives, the answer flips. The extension point is stated as such: adding a
protocol edits one file at the CLI, not a shared trait.

## Amendment: the pi azure-* pools, and the entries that reach them

The user asked that rho reach every azure-* pool pi reaches, with the same models. A read of
`~/.pi/agent/models.json` names five pools across three wire protocols. Every one maps to a
protocol this spec already resolves, so no new crate and no new protocol string is needed.

### The mapping table

| pi pool | pi `api` | rho `protocol` | rho crate | base URL |
| --- | --- | --- | --- | --- |
| `azure-openai` | `openai-responses` | `openai-responses` | `rho-provider-azure` | `http://127.0.0.1:58788/chat/openai/v1` |
| `azure-openai-dev1` | `openai-responses` | `openai-responses` | `rho-provider-azure` | `http://127.0.0.1:58788/dev1/openai/v1` |
| `azure-claude` | `anthropic-messages` | `anthropic` | `rho-provider-anthropic` | `http://127.0.0.1:58788/dev1/anthropic` |
| `azure-oss` | `openai-completions` | `openai-chat` | `rho-provider-openai-chat` | `http://127.0.0.1:58788/cus/openai/v1` |
| `azure-oss-nc` | `openai-completions` | `openai-chat` | `rho-provider-openai-chat` | `http://127.0.0.1:58788/nc/openai/v1` |

Twenty-three model ids across the five pools, from `gpt-5.5` and `gpt-5.3-codex` to
`claude-opus-5` and `FW-Kimi-K2.6`, are then reachable as
`--provider <entry-id> --model <model-id>`. The model id is the wire model, unchanged.

### The concrete config that lands them

The user hand-writes this once into `~/.config/rho/config.toml`, after which
`rho --provider azure-openai --model gpt-5.5 "hi"` works:

```toml
[credentials.xdent-key]
type = "literal"
value = "dummy"          # the proxy holds the real credential; a dummy resolves the trust gate

[[providers]]
id = "azure-openai"
protocol = "openai-responses"
base-url = "http://127.0.0.1:58788/chat/openai/v1"
credential = "xdent-key"

[[providers]]
id = "azure-openai-dev1"
protocol = "openai-responses"
base-url = "http://127.0.0.1:58788/dev1/openai/v1"
credential = "xdent-key"

[[providers]]
id = "azure-claude"
protocol = "anthropic"
base-url = "http://127.0.0.1:58788/dev1/anthropic"
credential = "xdent-key"

[[providers]]
id = "azure-oss"
protocol = "openai-chat"
base-url = "http://127.0.0.1:58788/cus/openai/v1"
credential = "xdent-key"

[[providers]]
id = "azure-oss-nc"
protocol = "openai-chat"
base-url = "http://127.0.0.1:58788/nc/openai/v1"
credential = "xdent-key"
```

### What the code already covers

`AzureConfig::new(base_url, ...)` at `crates/rho-provider-azure/src/lib.rs:115` already takes a
base URL. The current refusal comes from the CLI plumbing, `refuse_base_url("azure", ...)` at
`crates/rho-cli/src/provider.rs:329`, which the named-entry path in §5 skips because it
constructs the config directly. So `rho-provider-azure` needs no change to speak
`openai-responses` against any base URL. This is why the mapping table names the existing
crate.

`rho-provider-anthropic` and `rho-provider-openai-chat` are the two new crates that this
launch delivers. Both take a base URL by construction, per their specs.

### Tests

- `every_pi_azure_pool_is_reachable_by_a_named_entry` — the config above parses, and each of
  the five entries builds its provider without an error.
- `an_azure_openai_entry_calls_the_responses_api` — an `openai-responses` entry lands in
  `rho-provider-azure` and posts to `<base-url>/openai/v1/responses`.
- `an_azure_claude_entry_calls_the_messages_api` — an `anthropic` entry lands in
  `rho-provider-anthropic` and posts to `<base-url>/v1/messages`.
- `an_azure_oss_entry_calls_the_chat_completions_api` — an `openai-chat` entry lands in
  `rho-provider-openai-chat` and posts to `<base-url>/chat/completions`.
- `a_model_id_is_the_wire_model_unchanged` — a request built for
  `--provider azure-openai --model DeepSeek-V4-Pro` carries `"model": "DeepSeek-V4-Pro"` on
  the wire, with no rewrite.
