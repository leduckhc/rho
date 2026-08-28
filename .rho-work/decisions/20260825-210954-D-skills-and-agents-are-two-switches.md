# D-skills-and-agents-are-two-switches — one flag stopped two capabilities

**Question:** `--no-skills` also turns subagent discovery off. rho then registers no
`spawn_agent`, ignores every agent definition, and says nothing about either. Is that one
switch or two?

**Two.** A skill teaches the model a procedure. An agent definition is a worker the model can
start. A user who does not want skills in the prompt has said nothing about delegation.

## How it was found

By driving it. With `--no-skills`, the model listed ten tools ending at `read_tool_result`,
then read my agent definition off disk and impersonated the agent instead of spawning one. No
notice mentioned that anything had been skipped. Without the flag, rho printed
`1 agent definition(s) available to spawn_agent: summariser` and five subagent tools appeared.

The cause is one line in `build_session`:

```rust
discover: loaded.discover_skills,
```

One field governed both loaders.

## The decision

- `--no-skills` stops the **skill** search only. Its help text already says that, and now it
  will be true.
- A new switch stops the **agent** search: `--no-agents`, with `no-agents` in the config and
  `RHO_NO_AGENTS` beside it. It follows `--no-skills` exactly, so a user learns one shape.
- An explicit `--skill <path>` still loads with `--no-skills`, as it does today. There is no
  explicit-path equivalent for an agent yet, and this decision does not add one.
- **Silence is the defect, so rho speaks.** When a definition is found and discovery is off,
  rho says how many it skipped and which switch did it.

## Rules out

**Keeping one switch and documenting it.** A flag that removes a headline feature it does not
name is a trap, and the documentation for it would be an apology.

**A combined `--no-extensions`.** It would fold MCP in as well, and MCP has its own path
already. Three loaders, three switches, one shape.

**Reading the agent set anyway and hiding the tools.** Discovery reads files from the project,
so an off switch must stop the read, not the display.

## Rules that hold

- Both switches default to off, so the behaviour a user sees today is unchanged unless they
  passed `--no-skills`, in which case they get their agents back.
- A trusted-project rule is unchanged. `--trust-project` still governs a project definition.
