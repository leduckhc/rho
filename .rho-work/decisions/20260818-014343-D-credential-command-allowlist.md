# D-credential-command-allowlist — A credential command inherits an allowlist, stricter than the bash denylist


**Question (T1 architect):** what environment does a `!op read ...` credential command
inherit?

**Decision:** The child inherits `PATH`, `HOME`, and only the names in an explicit
`pass_env` allowlist. It inherits no variable that `rho_redact::looks_like_a_secret`
flags unless `pass_env` names it. It inherits no credential rho itself resolved. See
`SPEC-config` section 5.

**Reason:** `bash` uses a denylist, because a shell needs a wide and open-ended set of
variables. A credential helper needs a tiny set, so an allowlist is the safer trade. The
helper runs closer to the key, so it earns the tighter rule.

**Rules out:** a credential command inheriting the full environment. A credential
command inheriting another resolved credential.
