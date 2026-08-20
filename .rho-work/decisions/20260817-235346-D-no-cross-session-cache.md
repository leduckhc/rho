# D-no-cross-session-cache — No cross-session shared file cache, because rho has no tenancy model


The same review proposed a content-addressed file and grep cache shared between sessions in
one process, and then ranked it last itself, with a serious warning. The controller agrees
and is recording the refusal so nobody builds it as an obvious optimisation.

**The risk.** Sessions can have different confinement roots. `docs/benchmarks.md`
deliberately gives every session its own provider client, its own tool registry, and its own
context, "because a real host does not share those between users". A shared file cache would
hand one session's file contents to another, and path confinement would not catch it,
because the second session never touched the path.

**Decision.** Not built. It stays out until rho has an explicit tenancy model that says
which sessions may share what. The same argument applies to a shared grep index.

**What is already shared, and why that is safe.** Decision D-mcp-shared-by-default shares an MCP server
process between sessions. That is different in kind: an MCP server is a program the user
configured, it holds no session's file contents, and the sharing is keyed on a config
fingerprint. A file cache would hold exactly the data that must not cross.

**The general rule this establishes.** Density is rho's advantage, and density means shared
state. So every proposal to share something must answer one question first: what stops one
session reading another's data? If the answer needs a tenancy model, the proposal waits.
