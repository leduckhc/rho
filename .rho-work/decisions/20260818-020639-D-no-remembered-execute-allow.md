# D-no-remembered-execute-allow — A remembered allow never covers a tool that runs a program

**Question (controller, T1b review):** `SPEC-approval` scopes a remembered allow to the tool
name. What does that mean for `bash`?

**Decision:** `AskPolicy` refuses to remember an allow for `ToolKind::Execute`. It treats
an `AllowAlways` answer for that kind as `AllowOnce`, and it reports the change on the
event stream. A `RejectAlways` answer is remembered for every kind. A user who wants
every command approved must set `approval = allow-all` on purpose.

**Reason:** A remembered allow keyed by the tool name is safe for `read` and for `write`,
because the path is still confined. It is not safe for `bash`, because one allow would
cover every later command in the session. A user who approves `git log` would approve a
later `rm -rf`. That is one click between a prompt and blanket command approval, and it
is the `ToolKind::Other` fail-open family from decision D-plugin-does-not-classify-itself in a new place.

**Rules out:** A remembered allow for any kind that runs a program. A memory keyed by the
command string, because a shell string has too many equivalent spellings to compare
safely. A silent degrade: the policy must report that the answer became one call only.
