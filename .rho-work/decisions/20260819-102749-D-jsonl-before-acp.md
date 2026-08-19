# D-jsonl-before-acp — JSONL frontend ships first; ACP arrives as a bridge

**Question:** which headless frontend protocol does rho implement first?

**Decision:** rho implements a small JSONL protocol first, in a new crate
`rho-jsonl`. ACP comes later, as a bridge over that layer.

**This decision amends D-acp-is-real-acp. It does not overturn it.** The
premise of D-acp-is-real-acp still holds. `makit` really does drive pi over
ACP. The owner's app confirms this at
`makit/server/src/adapters/acp.ts` line 11: pi runs through this adapter via
the `pi-acp` bridge, which spawns a subprocess. A bridge maps ACP onto the
smaller protocol. rho can do the same thing. The destination is unchanged.
Only the order changes.

**Reason: `rho-acp` has zero lines of implementation.** pi's headless protocol
has 35 commands. ACP uses JSON-RPC 2.0, with `jsonrpc`, `method`, and `params`
fields. The JSONL protocol has no `jsonrpc` field, no `method`, and no numeric
error codes. It is smaller. It is a faster path to a working headless frontend.

**What this decision rules out:**

- Inventing a private dialect that no bridge can map onto ACP. `rho-jsonl`
  is a public contract. It is documented. Any language can implement a client.
- Calling the new protocol JSON-RPC. The wire has no `jsonrpc` field.
  D-acp-is-real-acp exists to keep protocol names honest. `rho-jsonl` must
  not repeat pi's confusion, where the protocol is called "RPC mode" but the
  wire carries no `jsonrpc` field.
- Treating SPEC-acp as cancelled. It is still valid. It describes the bridge
  target.

**Naming rule.** pi calls its headless protocol "RPC mode." The wire has no
`jsonrpc` field. rho must not copy that name. The new protocol is the
"JSONL frontend." Its crate is `rho-jsonl`. Its spec is `SPEC-jsonl-frontend`.
