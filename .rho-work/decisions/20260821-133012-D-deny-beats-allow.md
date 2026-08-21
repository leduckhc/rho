# D-deny-beats-allow — a deny rule always wins, and order never decides


**Question (controller):** fx matches permission rules by wildcard and lets the last
matching rule win. `F-scoped-command-guardrails` already says deny beats allow. rho cannot
hold both rules. Which one does rho keep?

**Decision:** Deny beats allow. rho keeps that rule, and rho refuses "the last match
wins". Rule order does not decide the outcome. rho evaluates every matching rule, and one
deny refuses the call. A narrower scope does not overturn a deny either.

**Reason:** "the last match wins" is a fail-open default. A user adds an allow rule at the
bottom of a file and silently widens an earlier deny. Nothing warns them, and the file
still reads as if the deny holds. This project has already shipped that class of defect
twice. `ToolKind::Other` counted as non-mutating, so a read-only policy approved any tool
that forgot to declare its kind. See D-plugin-does-not-classify-itself. Step 8 of
AGENTS.md now names a fail-open default as a thing to hunt for.

"Deny beats allow" has one more property that matters. It is order-independent, so a
config merge cannot change a decision. rho merges a user layer with a project layer, and
an order-dependent rule would make the merged result depend on which file was read first.
That is a bug a user cannot see, and cannot debug.

**Cost accepted:** a user cannot write a broad deny and then carve one exception out of it.
They must write the narrower deny instead. That is more typing, and it is readable.

**Rules out:** order-dependent rule evaluation. An allow rule that overturns a deny at any
scope. A merge order that changes an approval outcome. `F-permission-rules` states this
rule, and it names `F-scoped-command-guardrails` as the row that owns the matcher.
