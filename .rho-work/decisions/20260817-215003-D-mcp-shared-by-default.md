# D-mcp-shared-by-default — An MCP server is shared between sessions by default


An MCP server is a whole process, often a Node or Python program costing tens of
megabytes.

rho's measured cost is about 25 KB per extra session. If each session spawned its own
copy of every configured server, fifty sessions with three servers each would spawn 150
processes, and the footprint argument that justifies this project would collapse.

**Decision.** A server is shared by default, keyed by a fingerprint of its config, and
reference counted. The last session to release it stops it. A server that must not be
shared sets `shared: false`.

**Consequence to watch.** A shared server means one session's misbehaviour can affect
another. So every limit in `SPEC-mcp` section 6 applies per call, not per session, and a
call timeout protects a session from a server that another session has wedged.
