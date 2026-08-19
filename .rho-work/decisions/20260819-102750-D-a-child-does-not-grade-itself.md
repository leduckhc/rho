# D-a-child-does-not-grade-itself — rho's gate decides done, not the child


**The question.** Who decides a task is done? The child that did the work, or rho?

**The decision.** rho's gate decides. The gate checks the artifacts and it runs the
acceptance checks. The child only reports claims. A child-reported completion flag is never
trusted. `SPEC-agent-tasks` builds this into the types. The child fills `ChildClaims`, and
those fields are marked unverified. Only the gate builds a `CheckResult`. No type gives the
child a `passed` field. So a child cannot forge a pass. The failure is unrepresentable, not
merely tested.

**The reason: this repository shipped the fail-open bug twice.** A plugin declared its own
`ToolKind`, and a read-only policy trusted it, so a destructive tool ran as read. See
decision D-plugin-does-not-classify-itself. An MCP server had the same hole. See decision
D-mcp-does-not-classify-itself. In both, an untrusted party classified its own work, and rho
believed it. A `definition_of_done` that the child reports as met is the same shape. It is a
slogan, not proof.

**The reason: jcode rejected the bypassable path.** jcode considered letting the agent write
the flow as a script. It refused. Its words:

> a script ... rigor lives in unvalidated agent code and the gates are bypassable. This is
> the under-biased failure mode. Decision: A — the graph is a server-side object mutated
> through validated ops, not an agent-side script.

rho follows jcode's option A. rho owns the gate. A child cannot skip it.

**What this rules out.** It rules out trusting any child-reported completion flag. It rules
out a `definition_of_done` that the child marks as met. It rules out reading the child's
summary as proof. An artifact must be checked. The file must exist. The command must exit
zero. The check must pass. A child claim is context, never a verdict. See `SPEC-agent-tasks`
sections 5 and 6.
