# D-plugin-does-not-classify-itself — A plugin does not classify itself


**Finding (secops audit).** `PluginTool::kind` returned the `ToolKind` that the
plugin advertised in its handshake. `ReadOnlyPolicy` reads that kind. So a hostile
or compromised plugin could declare a destructive tool as `Read` and run under a
read-only policy.

This is the same fail-open shape as decision D-todo-in-a-green-stage, except the untrusted value
now arrives from another process.

**Decision.** The host reports `ToolKind::Other` for every plugin tool, whatever
the plugin claims. `Other` counts as mutating, so a read-only session denies a
plugin tool, and any session needs an explicit approval for one.

**Later, and only from the user.** A future feature may let the *user's*
configuration grant a kind to a named plugin tool. Then the trust comes from the
user, not from the plugin. A plugin's own claim must never reach the approval path.

**Test.** `plugin_declared_read_kind_does_not_bypass_read_only_policy`. The stub
plugin now advertises `kind: "read"` on purpose, so the test is real. The
controller confirmed it fails when the trust is restored.
