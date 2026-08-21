# D-artifact-not-chat — Agents share a typed artifact, not messages


**The question.** How do subagents communicate? A parent delegates work, and a child
produces a result. Should agents talk to each other, or should work flow another way?

**The decision.** A child delivers a typed artifact on the dependency edge. It does not send
a message to another agent. The artifact is the unit of transfer. rho defines it now in
`SPEC-agent-tasks` as `ArtifactSpec`. The edge and the scheduler come later.

**The reason: jcode built the chat version first, then judged it wrong.** jcode's recorded
rework is "dataflow first, chat second", and it is a rework "by subtraction, not addition".
jcode found that agent-to-agent chat is "the wrong shape". Its own words:

> It is chat, not dataflow. Every existing channel is push-notification messaging between
> agents. But in the DAG, the primary information transfer is node -> dependent via the
> artifact on the edge, which does not exist as a comm primitive yet ... so agents would
> have to simulate dataflow by DMing each other — exactly the lossy coordination we are
> replacing.

jcode also found the chat surface too large:

> Too many overlapping primitives. DM vs broadcast vs channel vs shared-context-fanout are
> four ways to push text at other agents ... More actions means more model error.

jcode's target is a typed handoff on the edge:

> the handoff artifact on edges ... Unlike a fire-and-forget DM it is typed, durable, by
> reference, and survives reloads.

rho takes that lesson. A typed artifact is checkable. A message is not.

**What this rules out.** It rules out, as primary mechanisms:

- a direct message from one agent to another,
- a broadcast to many agents,
- a topic channel that agents subscribe to,
- a shared key-value store that agents read and write.

jcode keeps a slim exception channel for conflict resolution and for a question up the
ownership tree. rho does not add even that yet. rho has no messaging at all in this spec.
The repository is a library, not a swarm. See `SPEC-agent-tasks` section 8.
