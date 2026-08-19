# D-cli-depth-is-zero — The command line sets the agent depth to zero, and says so


A live sweep on Bedrock drove the subagent feature end to end. See
`docs/verification/subagents-bedrock.md`. It found that three of the four subagent limits
told the user to raise a flag, and that none of those flags existed.

**The question.** What is the honest depth limit for a run started from `rho` on the command
line?

**The finding.** `rho-cli` captures the parent tool set **before** `spawn_agent` joins it. So
no child ever receives `spawn_agent`, whatever its definition asks for. The live sweep
confirmed this: a definition that listed `spawn_agent` had it dropped, and the drop was
reported. A grandchild is therefore impossible from the command line.

Meanwhile `SubagentLimits::new` set `max_depth: 2`, and the refusal text told the user to
raise `--max-agent-depth`. That flag did not exist, and no flag could have helped, because
the child holds no spawn tool. A test even asserted the flag name, so the suite pinned the
wrong message.

**The decision.** Three parts.

1. `rho-cli` passes `max_depth: 0`. That is the truth for a command-line run.
2. `rho-cli` gains a flag for each limit it can really change:
   `--max-children-per-parent`, `--max-live-agents`, and `--child-timeout-secs`.
3. The depth refusal names no flag. It says to do the work in this session, and it explains
   that a command-line subagent holds no spawn tool.

**Why not make a grandchild possible instead?** That is the other repair, and it is larger.
A child would need its own `spawn_agent` bound to its own node, which means the tool factory
must build the tool per child rather than share the parent's instances. That is a real
change to `ChildToolFactory`, and it widens the blast radius of a prompt injection, because
a fan-out becomes reachable from a definition file. The depth cap in `rho-core` already
supports it, so a library caller can do it today by passing a spawn tool to a child. rho
keeps that door open and leaves it shut by default.

**What this rules out.** It rules out advertising a depth flag that the command line cannot
honour. It rules out a refusal that sends the user after a fix that cannot work. It does not
rule out a grandchild for a library caller, and `SubagentLimits::max_depth` stays in the type
for exactly that caller.

**The rule this follows.** A refusal must teach, and it must teach something true. AGENTS.md
step 8 warns about a value that crosses a boundary and fails open. A limit that names a
missing flag is the same family: it looks like a control and it is not one.
