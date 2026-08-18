# D-mcp-does-not-classify-itself — An MCP server does not classify its own tools


MCP has no equivalent of `ToolKind`, and a server's own opinion would not be
trustworthy if it had one.

**Decision.** Every MCP tool reports `ToolKind::Other`, which `is_read_only` treats as
mutating. So a read-only session denies an MCP tool, and any session needs an explicit
approval for one.

This is decision D-plugin-does-not-classify-itself applied to a second source. A plugin's self-declared kind was
refused for the same reason. A later feature may let the user's configuration grant a
kind to a named MCP tool. Trust then comes from the user.
