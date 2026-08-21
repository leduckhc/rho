# D-a-tool-keyword-stands-alone — `tools: all` means inherit, and a keyword never shares a line

**Question (user, reading the definition format):** Can a definition say `tools: all` to give a
child every tool the parent holds?

**The state before this decision.** It could not, and the failure was quiet. `parse_tool_list`
split the line on commas and spaces, so `all` became a tool name. `intersect_tools` then
dropped it, because no parent tool is called `all`. `ChildToolFactory::build(&[])` registered
nothing. The child ran with an empty registry, and the parent's result carried one note about a
dropped name. So the definition asked for everything and the child received nothing.

pi accepts `all`, `*`, and `none`, so an author who reads pi's documentation writes `tools: all`
here and gets a useless child.

**Decision:** `rho-skills` resolves three keywords when it loads a definition.

| Line | Meaning | Field value |
| --- | --- | --- |
| `tools` absent | Inherit the parent's set | `None` |
| `tools: all` or `tools: *` | Inherit the parent's set | `None` |
| `tools: none` | No tool at all | `Some([])` |
| `tools: read, grep` | Intersect these with the parent's set | `Some([read, grep])` |

A keyword is case-insensitive, because `ALL` is the same wish as `all`.

**A keyword must stand alone.** `tools: all, read` is a contradiction. The keyword is dropped,
the named tools stand, and a warning names what it dropped. `tools: all, none` resolves to
`none`, with a warning.

**Reason for the narrow reading.** A line with a keyword and a name has two possible readings.
One widens the child's tool set and one narrows it. rho takes the narrow one every time, because
a wrong widening is a privilege escalation and a wrong narrowing is a visible failure. This is
the same rule that made `ToolKind::Other` a defect.

**Why the keyword lives in `rho-skills` and not in `intersect_tools`.** `intersect_tools` is the
security core in `rho-core`. It must keep one literal meaning: a requested name is kept only when
the parent holds it. A keyword resolved inside it would let any future caller pass `all` and
receive the parent's whole set. A definition file has a trusted author, and the loader is the
right boundary for a keyword. See decision D-child-confined-by-composition.

**`none` earns its place.** An empty list said the same thing before, and no reader could tell it
from a mistake or a typo. `none` states it on purpose.

**Rules out:** A keyword inside `rho-core`. A keyword the model can pass through `spawn_agent`,
because a model-supplied `all` would be an escalation request. A silent widening when a line mixes
a keyword with a name. A keyword that means "every tool that exists", because a child can never
receive a tool its parent lacks.

**Proof.** Five tests in `crates/rho-skills/tests/agents.rs`. Two deliberate breaks were run: one
made a mixed line widen to inherit-everything, and one made `none` inherit. Each broke exactly the
test written for it, and the good file was restored by copy, never by `git checkout`.
