# D-one-live-agent-handle — One handle to a live child, and it can really stop it


Two handle types existed for the same idea. `crates/rho-core/src/agent_handle.rs` held an
`AgentHandle`, and `crates/rho-core/src/subagent.rs` holds `LiveAgent`. The owner asked for
the weaker one to go if the other is better. It is, and here is the evidence rather than the
assertion.

**The question.** Which type represents a running subagent?

## What the removed type was

`AgentHandle` was an **orphan**. `crates/rho-core/src/lib.rs` never declared
`mod agent_handle`, so Rust never compiled the file. No code referenced it, and it was never
committed to git. It was dead in the strongest sense: not merely unused, but not part of the
crate.

Its public surface was `new`, `id`, `name`, `usage`, `outcome`, `finished`, plus two
crate-internal setters. Its doc comment said:

> A handle can query the agent's progress or cancel it without affecting siblings.

**It could not cancel anything.** The word "cancel" appeared in that file only inside the doc
comment. There was no token, no method, and no mechanism. The type promised a capability it
had no way to deliver, which is the defect family this project keeps meeting: a claim with
nothing behind it. See decision D-provider-extension-verified-outside.

## The decision

`LiveAgent` is the one handle. `agent_handle.rs` is deleted.

`LiveAgent` holds the child's **derived** `CancelToken`, so `cancel` stops that child and
leaves its siblings and its parent running. It holds a `watch` receiver, so `progress`
reports turns and usage while the child works. It holds the child's `MessageQueue`, so
`steer` reaches the running child. `spawn_child` registers it and `ChildSlot::drop`
deregisters it, both in one file, so a caller cannot forget either side. Twelve tests cover
it, and each guard was proved by breaking the implementation.

## What the removal gives up, and why that is right

`AgentHandle` had `outcome` and `finished`. `LiveAgent` has neither, on purpose.

A `LiveAgent` exists only while the child is live. A handle to a finished child is a handle
that cancels nothing and steers nothing, and handing one out is the bug, not the feature. So
the outcome arrives by a different route: `AgentReport` carries it to the caller, and
`AgentEvent::AgentFinished` carries it to a frontend. Both are verified paths.

**Rules out.** Two types for one concept. A handle that outlives the child it names. A doc
comment that promises cancellation without a cancel token.

## The authorship note, recorded because it is unresolved

The controller cannot prove who wrote `agent_handle.rs`. This tree has more than one writer,
see D-shared-working-tree, and the controller earlier blamed a second writer using file group
ownership as evidence. That evidence was wrong: every file in this tree shares the same
group, including the controller's own. The file is preserved outside the repository at
`/tmp/removed-agent-handle/` rather than only in this history, because the deletion removes
something that was never committed and therefore cannot be recovered from git.
