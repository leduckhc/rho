# D-redact-json-secrets — Session redaction uses one new function in rho-redact


**Question (T1b architect):** how does a credential-shaped tool argument stay out of
the session file, given the function does not exist?

**Decision:** Add `rho_redact::redact_json_secrets(&serde_json::Value) ->
serde_json::Value` to `rho-redact`. It masks a value under a key that
`looks_like_a_secret` flags, keeps every other value and key and the tree shape, and
reads a key name only. It is new work for stage T5. No session record may be written
before it exists. `rho-redact` stays the one home for redaction, per D-one-redaction-home.

**Reason:** `SPEC-sessions` section 5 and decision D-redact-tool-arguments both depend on this function, and a
reviewer proved the call fails with `E0425` today. This is the same family as
`confine`, left `todo!()` through a green stage, so the surface is named and gated.

**Rules out:** a second redaction implementation outside `rho-redact`. Writing a raw
tool argument to disk and filtering it later. Building the session writer before the
function lands.
