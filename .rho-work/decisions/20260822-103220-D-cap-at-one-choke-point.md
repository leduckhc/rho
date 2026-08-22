# D-cap-at-one-choke-point — every tool result is capped in one place


**Question (controller):** which code caps a large tool result? Each tool, or the harness?

**Decision:** The harness, in `Session::finish_tool`. That function is the one place every
tool output passes through on its way into the context. `bash`, `read`, `grep`, an MCP tool,
a plugin tool, and a tool a third party writes tomorrow all flow through it.

A tool may still bound its own output for its own reasons. `bash` caps what it reads so a
runaway command cannot exhaust the host, and that cap stays. This decision is about the
context, not about the host.

**Reason:** a per-tool cap is a rule every tool author must remember, and one that a
reviewer must check for every new tool. That is the shape of `ToolKind::Other`, where a
tool that forgot to declare itself was treated as safe. See
D-plugin-does-not-classify-itself. rho already learned that a boundary a peer can forget is
not a boundary.

Today `finish_tool` appends the whole output with no bound at all. So an MCP server that
returns ten megabytes puts ten megabytes into the context window, and the user pays for it
on every later turn of the session. No tool in `rho-tools` does that, which is exactly why
no test caught it: the defect needs a peer rho does not ship.

**What this buys.** A new tool is bounded before it is written. A plugin author cannot opt
out. The cap is one function with one test, rather than a convention in a style guide.

**Rules out:** a per-tool cap as the mechanism. A cap in the provider crates, which would
have to be repeated three times and would miss a fourth. A cap in the session writer only,
which bounds the file and leaves the context unbounded. Trusting a peer to bound itself.
