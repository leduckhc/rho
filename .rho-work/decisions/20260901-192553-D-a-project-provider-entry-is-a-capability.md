# D-a-project-provider-entry-is-a-capability — gated the same as a skill path

**Question:** a project file defines a `[[providers]]` entry. Does it work when untrusted?

**Decision: no. A provider entry is a capability. An untrusted project supplies none.** A `[[providers]]` entry from an untrusted project file returns `UntrustedProjectProvider`. The user passes `--trust-project` to allow it.

## The reason

A provider entry names a credential. The credential resolves in the merged `[credentials]` table, which includes the user-global file. So a project file could name a credential the user set for another provider, and send it somewhere else.

**The `credential` field is a name, not a value.** That name resolves after the trust gate. An untrusted project file may not add a credential, per `D-an-untrusted-clone-supplies-no-credential`. It may name one, but the capability to name one is still a capability.

The rule is the same as `skill-paths` and `mcp-config`. An untrusted project supplies no paths that run code or redirect secrets. A provider entry redirects a credential, so it is gated.

## What this rules out

**Allowing an untrusted provider entry.** A clone could name a credential in the user's global config. It could send that credential to a different endpoint.

**Gating only the `credential` field.** The whole entry is a capability. A partial gate would make the rest of the entry reach a provider. That is a dead switch waiting for a second field to join.

**Resolving the credential in the project-file layer alone.** A merge includes the global file. A project credential reference would still resolve in the merged table.

## What it costs

A project that ships a `[[providers]]` entry needs `--trust-project` once. That is the cost every other gated capability already accepts.

A project that names no entry, and relies on the user's own `OPENROUTER_API_KEY`, needs no flag. The built-in provider reads the credential from the user's environment or global config, not from the project.
