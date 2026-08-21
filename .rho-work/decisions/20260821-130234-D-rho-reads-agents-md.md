# D-rho-reads-agents-md — rho reads AGENTS.md, and the narrowest scope wins


**Question (controller):** rho ships no project-instruction discovery. Does it add one,
and which file does it read?

**Decision:** rho reads `AGENTS.md`. It gathers three sources, in this order of breadth:
a user file at `~/.config/rho/AGENTS.md`, then every `AGENTS.md` from the session root
down, then the `AGENTS.md` nearest to a path a tool call targets. When two files
conflict, the narrowest scope wins. A direct user instruction beats every project file.

The gathered text joins the stable prefix, exactly as the skills block does. So the
provider prompt cache stays warm. See `F-stable-prefix-for-kv-cache`.

**Reason:** this is the largest gap the fx comparison found. rho's own repository holds a
200-line `AGENTS.md` full of rules, and rho reads none of it today. A user who writes
project rules and sees them ignored reads that as a defect, not as a missing feature.
`AGENTS.md` is the name pi, fx, and jcode all read, so a user needs no new file.

**Bounded by construction:** one file is capped, and the whole set is capped. An omitted
or truncated file is reported to the caller. Silence about dropped instructions is a
defect, because the model then answers from an incomplete contract.

**Trust:** a project file comes from the repository under edit, so it is untrusted input.
rho reads it anyway, and rho makes it grant nothing. See
D-project-instructions-are-authority-inert for the three rules that enforce this, and for
why this differs from D-project-skill-needs-trust.

**Rules out:** inventing a rho-specific instruction filename. Reading an instruction file
from an additional directory, because that directory grants tool access and not
authority. Treating project instructions as able to widen a permission. Loading the file
set into the dynamic part of the prompt, which would break the cache prefix.
