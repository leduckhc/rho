# D-project-instructions-are-authority-inert — AGENTS.md advises, and never grants


**Question (controller):** `D-project-skill-needs-trust` withholds a project skill until
the user trusts the session root, because "a skill inside the repository under edit is a
prompt injection with a filename". An `AGENTS.md` in that same repository is the same
text from the same untrusted source. Does rho withhold it too?

**Decision:** No. rho reads a project `AGENTS.md` by default, and rho makes it
authority-inert. The file advises the model. It grants nothing.

Three rules make that real, and each one is a test:

1. A project instruction cannot widen a permission. It cannot allow a tool, relax the
   sandbox, or approve a call. rho reads authority from config and from the user, never
   from an instruction file.
2. A project instruction cannot add a discovery root. It cannot name a skill directory, a
   plugin, an MCP server, or a provider. Those stay in config, which the user owns.
3. rho marks the text as project-supplied in the prompt, and states that a direct user
   instruction wins. The model sees where the text came from.

**Reason for the split from a skill.** The threat is the same, and the blast radius is
not. A skill is a directory that carries scripts, and rho hands the model a path and tells
it the skill is available. `AGENTS.md` is prose with no attachment and no path to run.
`D-project-skill-needs-trust` withholds a skill because loading one is closer to running
code than to reading advice.

The second reason is that option (a) does not work. If rho withheld `AGENTS.md` by
default, then rho would ignore the file in every fresh clone. Every other agent reads it,
so a user would read the silence as a defect and not as a security control. A control the
user routes around protects nobody. rho's own `AGENTS.md` is the proof: 200 lines of rules
that rho would discard on the first run.

**What remains for the paranoid user.** A config key turns project instruction discovery
off. The user file and the session root stay separable, so a locked-down deployment reads
only the file it owns.

**Rules out:** reading authority of any kind from an instruction file. Loading the file
with no marking, which is what makes the text look like a rho rule. Inferring trust from
`--read-only`, which D-project-skill-needs-trust already refuses. Silently ignoring the
file, which trades a small risk for a certain defect.
