# D-a-caller-addresses-only-its-own — A caller addresses only its own descendants


A security review of the subagent controls found this, and proved it with a scratch
crate. It is the most serious finding of the review.

**The question.** When a tool names a subagent by id, whose subagents may it reach?

## What was wrong

`AgentRegistry::handle`, `cancel`, and `live` resolved a bare `u64` against the **whole**
registry. The registry is process-wide by design, because `max_live_total` protects the
machine and must count every session's children. So nothing stopped one session from naming
another session's child.

The review built one registry, two independent trees, and showed the second tree enumerating
the first tree's child, cancelling it, and pushing a steering message into it. All three
succeeded.

**The shipped command line was safe, and that is exactly the problem.** Each `rho run` builds
its own registry, `rho-acp` wires no subagents, and a child never receives a control tool. So
the boundary held **by wiring, not by the contract**. Any library host that follows the
documented "one registry per process" model, or that grants a child a control tool, got
cross-session cancel and steer with no check at all.

A boundary that depends on how a caller happens to be wired is not a boundary. This project
has met that shape before, in `ToolKind::Other`: safe until somebody forgot.

## The decision

A live child carries its ancestor chain, and the registry offers scoped accessors:

- `live_under(caller)` lists only the caller's own descendants.
- `descendant(caller, id)` resolves an id only when the caller is above it.
- `cancel_descendant(caller, id)` cancels only such a child.

**Every tool uses the scoped form.** `steer_agent` and `cancel_agent` now pass their own
`AgentNode`, so a tool acting for one session cannot reach another. A refusal reads the same
for another tree's child as for a child that already finished, because neither is addressable
and the caller does not need to tell them apart.

Descendancy, not parenthood. A root reaches any depth below it, so a fan-out of a fan-out
stays manageable.

**The unscoped forms stay, and they say what they are for.** `live`, `handle`, and `cancel`
are documented as process-wide, for a host that owns the whole process, such as a terminal
frontend rendering every session, or a shutdown path stopping everything. Their doc comments
name the scoped form a tool must use instead.

## What this rules out

It rules out a tool that takes an id and trusts it. It rules out a child reaching a sibling.
It rules out one session stopping another session's work.

It does not rule out a host doing any of that on purpose, because a host that owns the process
already owns every child in it.

## The guards

- `one_tree_cannot_cancel_another_trees_child`
- `a_grandchild_is_addressable_by_its_ancestor`
- `live_under_lists_only_the_callers_own_descendants`
- `steer_agent_cannot_reach_another_sessions_child`, in `rho-tools`, which fails when the tool
  is reverted to the unscoped lookup.
