# D-agent-ref-accepts-id-or-name — The id argument accepts a number or a name


**Question (architect):** how does a handle arrive in the `steer_agent`, `agent_status`,
and `cancel_agent` schema, without breaking a caller that sends an integer?

**Decision:** The `id` argument accepts both forms. Its schema is
`{"type": ["integer", "string"]}`. rho parses it into `AgentRef`, an untagged enum over a
number and a name. A number resolves as an id. A digits-only string resolves as an id. Any
other string resolves as a handle, then an alias, inside the caller's tree. See
`SPEC-subagent-slots-handles-grace`.

**Reason:** a model already writes integers, and a hard switch to a string would reject
them. Accepting both keeps the old shape working, so the migration needs no coordinated
release. One field stays simpler than a `oneOf` or a second property.

**Rules out:** a breaking schema change. A separate property for a handle. A caller that
sends the old integer shape and gets refused.
