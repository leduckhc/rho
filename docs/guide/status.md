# Status at version 0.1.0

rho streams answers, calls tools, and runs subagents today.
It does not record sessions, and it does not ask before a mutating tool call.
This page describes rho as of 2026-08-23.

## Works today

| Capability | Documented on |
|---|---|
| Streaming answers with tool calls | [quickstart](quickstart.md) |
| File read, write, edit, list, glob, and grep tools | [tools](tools.md) |
| Bash tool with timeout and output streaming | [tools](tools.md) |
| Path confinement to the session root | [tools](tools.md) |
| Read-only mode (`--read-only`) | [permissions](permissions.md) |
| Allow-all mode (the default) | [permissions](permissions.md) |
| Auto-retry on 429 and 5xx responses | [providers](providers.md) |
| Cancellation with Ctrl+C | [quickstart](quickstart.md) |
| OpenRouter provider (live-verified) | [providers](providers.md) |
| AWS Bedrock provider (live-verified) | [providers](providers.md) |
| Subagents, with flags to set limits | [subagents](subagents.md) |
| Background tasks the agent can tail or cancel | [tools](tools.md) |
| OS sandbox for bash (`--sandbox confined` or `strict`) | [tools](tools.md) |
| Skills loaded from `SKILL.md` files | [skills](skills.md) |
| `/help` and `/quit` slash commands | [terminal interface](terminal.md) |
| Slash-command list (`/` opens it, typing filters it) | [terminal interface](terminal.md) |
| 1464 tests passing in the workspace | — |

## Partly built

| What works | What does not, and the workaround |
|---|---|
| `approval = "ask"` parses in the config. | rho refuses it with an error message. Use `--read-only` to block writes, or omit the flag to allow all. |
| `session-file` and `ephemeral` parse and merge correctly. | Neither value reaches the agent loop. No session file is written. Silence: rho starts without error and records nothing. |
| The `[subagents]` table parses in the config. | No value from it reaches the agent. Pass the limits as flags: `--max-children-per-parent`, `--max-live-agents`, `--child-timeout-secs`, `--max-queued-per-parent`, `--max-queued-total`, `--agent-grace-turns`, `--max-agent-tool-calls`. |
| The steering queue is built in the library. | The terminal interface wires no queue for your session. A message you type mid-run does not reach the running turn. Enter starts a new turn instead. |
| The approval panel is drawn and its keys are unwired. | Nothing opens it in a real run. This is why `approval = "ask"` has nowhere to go, even in the terminal build. |
| The terminal interface holds code for an image attachment. | No image reaches the provider. A pasted image is dropped at the send step, in silence. |
| The `[credentials]` table parses into named sources. | Nothing resolves an entry and no provider asks for one, so the block changes nothing. Give a provider its key through the environment instead. |
| Azure OpenAI parses and has unit tests. | It has had no live run. Treat it as untested. |

## Not built yet

| If you try it today |
|---|
| `/model`, `/sessions`, `/guide` — each command is in the list and answers `<command> is not built yet.` |
| Ctrl+O is shown in the help screen. It is labelled as not built and does nothing. |
| A `todo` tool — the model keeps no task list, so a long job has no checklist you can read. |
| An `ask_user` tool — the model cannot ask you a question mid-turn. It guesses instead, or it stops. |
| A token count or a cost display — the interface shows neither, so you cannot see what a turn spent. |
| Context compaction — rho has no compaction logic. A turn that writes too much output stops with a `max tokens` reason. An input that outgrows the window returns a provider error. Either way there is no recovery path. |
| A model registry — rho passes whatever model id you give it straight to the provider. An invalid id returns a provider error. |
| A subagent spawning its own subagent — the CLI sets the depth limit to 1 and offers no flag to raise it. |
| Per-tool or per-path permission rules — only the read-only switch and allow-all exist. |
| An undo for a file change rho made — no change log is kept. |
| Prompt templates and custom slash commands — no template expansion runs today. |
| The ACP frontend — the crate exists, and the binary does not expose it. Building with `--features acp` adds the code and no command reaches it. |
| Out-of-process plugins — the plugin host is a library, and the `rho` binary loads no plugin. The `plugins` feature is on by default and reaches nothing. |

The internal roadmap lives at [../features.md](../features.md).
It is an engineering document, so its language is internal.
Feature ids, sprint labels, and extension-point notes are written for contributors, not for users.
