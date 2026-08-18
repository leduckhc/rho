# D-skill-allowed-tools-ignored — `allowed-tools` in a skill is parsed, warned about, and ignored


The Agent Skills frontmatter allows a skill to pre-approve tools for itself.

**Decision.** rho reads the field, warns that it has no effect, and ignores it.

**Reason.** A skill granting itself approval inverts the boundary that decisions D-todo-in-a-green-stage
and D-plugin-does-not-classify-itself worked to make fail closed. The approval policy stays the single authority.
Honouring the field needs a design that asks the **user**, not one that believes the
skill.
