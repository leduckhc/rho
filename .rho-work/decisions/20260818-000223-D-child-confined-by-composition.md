# D-child-confined-by-composition — A child is confined by composition, not by comparison


The owner asked for a subagent feature, robust and improved over jcode and pi. The spec is
`SPEC-subagents`. One design point deserves its own decision, because the controller's first
attempt was wrong.

**The wrong version.** The brief said "a child's approval policy must be at most as
permissive as its parent's". A survey of the real code showed that is not implementable.
`ApprovalPolicy` is a trait object with one async method and **no ordering and no
comparison**, so two arbitrary policies cannot be ranked. Code that claimed to check the
rule would have been lying.

**The right version.** Express the rule as composition. `BothPolicies` allows a call only
when the parent's policy and the child's policy both allow it. The parent is always one of
the two conjuncts, so a child can only be more restrictive.

**Escalation stops being a check and becomes unrepresentable.** That is the difference
between a rule that is enforced and a rule that is tested.

The controller verified the type in a scratch crate against the real `rho-core`: a
`ReadOnlyPolicy` parent with an `AllowAllPolicy` child denies a write and allows a read.

**The tool set uses the same rule, and here rho improves on pi.** pi's agent frontmatter
lists tools, and that list is independent of the caller, so a child can receive a tool its
parent never had. rho **intersects**: the child's set is the parent's set filtered by the
child's list, and a dropped name is reported so a bad definition is visible.

**Credit where it is due.** The file-based agent definition, with frontmatter naming the
tools and the model, is pi's design, and rho reuses its own `rho-skills` parser for it.
Three robustness ideas are jcode's: salvage when a worker dies, a **cap** on that salvage so
a poisoned task is failed rather than requeued forever, and two separate limits for the
machine and for the run. The cap is the detail that turns salvage from a loop into a
guarantee, and a first attempt would have missed it.

**The honest cost, recorded in the spec rather than discovered later.** Sessions share an
address space, so there is **no fault isolation**: a panic in a child can take the process
down. jcode and pi buy isolation with a process each and pay about forty times the memory.
rho trades isolation for density, and a host that needs isolation can run rho in separate
processes, because rho is a library.
