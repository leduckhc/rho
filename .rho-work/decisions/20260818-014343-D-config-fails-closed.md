# D-config-fails-closed — Config fails closed on a malformed file, an unknown key, or an unreadable file


**Question (T1 architect):** what does `rho-config` do with a bad config file?

**Decision:** A malformed file, an unknown key, and an unreadable file each return a
typed `ConfigError`. The run stops. It never falls back to a default that grants more
access than the file asked for. `serde(deny_unknown_fields)` catches an unknown key. A
missing file is not an error.

**Reason:** decision D-plugin-does-not-classify-itself shipped a fail-open default once. `ToolKind::Other` read as
non-mutating and opened a boundary. A broken `approval` key that read as `allow-all`
would be the same defect in a new place.

**Rules out:** a permissive fallback on a parse error. Treating an unreadable file as an
empty layer.
