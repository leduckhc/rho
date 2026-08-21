# D-handle-is-a-second-address — A handle names a child, and the id stays the identity


**Question (architect):** how does a model address a child by name, without weakening the
id?

**Decision:** The id stays the identity, unique per process. A handle is a second address
for the same child. rho derives a handle from the agent name and numbers a collision, so a
second `explore` becomes `explore-2`. A caller may set one alias per child. An alias never
shadows a derived handle. A handle is unique per tree, not per process. See
`SPEC-subagent-slots-handles-grace`.

**Reason:** pi derives a handle the same way, and a model handles a name better than a
number. Per-tree scope keeps a handle from reaching another tree's child, which the id
scope already forbids. A derived handle always resolves first, so an alias cannot hide a
real child.

**Rules out:** a handle that replaces the id. A handle that resolves across trees. An alias
that shadows a derived handle. A handle for a nested grandchild.
