# D-redact-tool-arguments — A tool argument is redacted on the way into the file


**Question (T1 architect):** how does a credential-shaped tool argument stay out of the
session file?

**Decision:** The recorder passes every tool argument through a new
`rho_redact::redact_json_secrets`, which masks a value under a key that
`looks_like_a_secret` flags. `rho-redact` is the one home for this, per decision D-one-redaction-home.

**Reason:** a session file must hold no credential. A message content block never holds
a `Secret` type, but a tool argument is free JSON and can carry one. So the writer masks
it before the record is written.

**Rules out:** a second redaction implementation outside `rho-redact`. Writing a raw
tool argument to disk and filtering it later.
