# D-an-acceptance-check-has-a-trusted-author — An acceptance check has a trusted author


`SPEC-agent-tasks` gives a task an acceptance check. `ArtifactSpec::Command` runs a shell
command and requires exit zero. rho's gate runs that command in the parent process, after
the child stops. The reviewer of the contract raised the risk before any code existed.

**The question.** Who may author an acceptance command?

**The risk.** A gate command is arbitrary shell, and it runs with the parent's authority.
The child cannot run the gate, so `D-a-child-does-not-grade-itself` holds. But a child can
still write text. If any child output reached the gate, a prompt-injected child would
choose the command that judges it. The gate would then run attacker-chosen shell as the
parent. That defeats the whole feature and it adds a new hole.

**The decision.** An acceptance check comes only from a trusted author. Three sources
qualify, and no other:

1. The agent definition file, under the trust rule in `SPEC-skills`.
2. The rho configuration, under `SPEC-config`.
3. The caller that builds the `AgentTask` in Rust.

**What this rules out.** The gate never reads a check from the child's summary, from the
child's report, from a tool result, or from any model-authored string in the running turn.
A parent model may choose **which** named check to apply. It may not write the command.

**Why a name, and not a string.** A model selects a check by label from the set the
definition already declares. So the model keeps the useful freedom, which is deciding what
"done" means for this task. It gains no freedom to write shell. This is the same shape as
`BothPolicies`: the dangerous case is unrepresentable rather than checked.

**The parent sandbox still applies.** The gate runs through `CommandRunner`, so the command
obeys the parent's sandbox mode and the parent's approval policy. A confined parent runs a
confined gate. See `SPEC-bash-sandbox` and `D-sandbox-is-correctness`.

**A project definition needs trust first.** An agent definition from the repository carries
a tool list, a model choice, and now a gate command. So `D-project-skill-needs-trust`
applies with more force here, exactly as `SPEC-subagents` section 5 already says.

**What the spec must add.** A named error for a check whose author is not trusted, and a
test that a check taken from child output is refused. Both are listed in
`SPEC-agent-tasks`. The security review in step 9 happens before implementation, not after.
