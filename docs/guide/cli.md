# CLI reference

Complete flag reference for the `rho` binary. Version 0.1.0.

## Two modes

Run rho with no subcommand to open the terminal interface:

```sh
rho
rho --model anthropic/claude-3-7-sonnet-20250219
```

Run one prompt and print the answer to stdout:

```sh
rho run "add tests for the auth module"
rho run --read-only "what does this file do?"
```

Every flag below works in both modes.

## Precedence

A flag beats an environment variable, which beats a config file.
See [configuration](configuration.md) for the full order.

## Model selection

| Flag | Environment variable | What it does |
|---|---|---|
| `--base-url <URL>` | The provider endpoint. Use it for a local model host such as Ollama or vLLM. A remote plain-`http` url is refused, and a notice names the host your key goes to. |
| `--provider <NAME>` | `RHO_PROVIDER` | The provider to use, for example `openrouter` or `azure`. |
| `--model <ID>` | `RHO_MODEL` | The model id to send. For Azure, this is your deployment name. |
| `--profile <NAME>` | — | Load a named block from the config file. |

When you omit `--model`, rho picks the provider default and prints a notice.
Azure has no default. You must pass `--model` or set `RHO_MODEL`.

## What rho may do

| Flag | Default | What it does |
|---|---|---|
| `--read-only` | off | Deny every tool that writes, edits, or runs a command. Read and search still work. |
| `--sandbox <off\|confined\|strict>` | `off` | `confined` limits writes to the session root. `strict` also denies the network. |
| `--root <PATH>` | current directory | Tools cannot touch a path outside this directory. |

`--read-only` beats `approval = "allow-all"` in a config file.
It applies the merge order: a flag wins over a file.

When you ask for `confined` or `strict` and no OS sandbox is available,
`bash` refuses the command with an error.

The config key `approval = "ask"` is not supported here. rho stops and prints:

```
approval = "ask" needs an interactive frontend, which this build does not have here. Use read-only or allow-all, or pass --read-only.
```

Pass `--read-only` instead.

## Interface

| Flag | Default | What it does |
|---|---|---|
| `--no-mouse` | — | Return mouse control to the terminal. Drag selects text without a modifier. |
| `--mouse` | — | Capture the mouse. The scroll wheel moves the transcript. On by default. |
| `--no-motion` | Stop the animation that sweeps the working word. The footer still names the state. |
| `--reasoning <off\|summary\|full\|live>` | `summary` | How rho draws reasoning in the terminal interface. `summary` shows one row. `full` adds the text, dimmed. `live` streams the text then collapses it. `off` hides it. |
| `--reasoning-effort <off\|low\|medium\|high\|xhigh>` | provider default | How hard the model thinks. Unset means rho sends no field to the provider. |

`--no-mouse` wins over `--mouse`. You cannot pass both.

In `rho run`, `full` and `live` print reasoning to stderr. `summary` and `off` print nothing.

The TUI requires the `tui` cargo feature.
A build without it prints: `this build has no terminal UI. Use "rho run <prompt>"`.

## Skills and tools

| Flag | What it does |
|---|---|
| `--trust-project` | Load skills from the repository you are editing. Off by default. A skill can instruct the model and carry scripts. |
| `--skill <PATH>` | Load a skill from this path. Repeatable. Works even with `--no-skills`. |
| `--no-skills` | Stop the skill directory search. An explicit `--skill` still loads. It no longer touches subagents. |
| `--no-agents` | Stop the agent-definition search, so rho offers no subagent. |
| `--mcp-config <PATH>` | Read MCP servers from this file instead of `~/.rho/mcp.json`. |

## Subagents

| Flag | Default | What it does |
|---|---|---|
| `--max-children-per-parent <N>` | 4 | How many subagents one parent runs at once. |
| `--max-live-agents <N>` | 32 | How many agents run at once in the whole process. |
| `--child-timeout-secs <N>` | 600 | Seconds before rho cancels a child. |
| `--max-queued-per-parent <N>` | 16 | How many children one parent may queue for a slot. |
| `--max-queued-total <N>` | 128 | How many children may wait across the whole process. |
| `--agent-grace-turns <N>` | 5 | Turns of warning before a child hits its turn cap. `0` turns off the warning. |
| `--max-agent-tool-calls <N>` | 64 | How many tool calls one child may make. |

When a child hits `--max-children-per-parent`, rho queues it.
When the queue hits `--max-queued-per-parent`, rho refuses with an error that names the flag.

> **Not built yet.** There is no `--max-depth` flag. A subagent cannot spawn
> a grandchild. rho sets depth to 1 in code and offers no way to change it.
> Passing a depth flag is not possible today.

## Diagnostics

| Flag | What it does |
|---|---|
| `--log <FILTER>` | Set the log filter, for example `info` or `rho_core=debug`. Beats `RHO_LOG`. |
| `-V`, `--version` | Print the version and exit. |
| `-h`, `--help` | Print a short usage summary and exit. |

## What does not work yet

> **Not built yet.** rho records no session file today. There is no `--resume` flag
> and no session list. Each run starts fresh. Nothing persists between sessions.

> **Partly built.** `--no-skills` also turns off subagent discovery, and it says nothing.
> rho then registers no `spawn_agent`, so the model cannot delegate, and your agent
> definitions are ignored. A live run proved it: with `--no-skills` the tool list stopped at
> `read_tool_result`, and without it five subagent tools appeared. There is no flag that
> keeps subagents while dropping skills.
