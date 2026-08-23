# rho user guide

rho is a coding agent you run in your terminal. This guide shows you how to use it.

Version 0.1.0. rho is early software, so [status](status.md) lists what works today, what is
half built, and what is missing. Read it when a page surprises you.

## Start here

| Page | What it gives you |
| --- | --- |
| [Quickstart](quickstart.md) | Build rho, set a key, and get your first answer. |
| [Status](status.md) | What works today, what is partly built, and what is missing. |

## Using rho

| Page | What it gives you |
| --- | --- |
| [Terminal interface](terminal.md) | Every key, every slash command, and the status line. |
| [Commands and flags](cli.md) | The full reference for the `rho` command. |
| [Sessions](sessions.md) | What rho keeps after a run, and what it throws away. |
| [Troubleshooting](troubleshooting.md) | A message you do not understand, and its fix. |

## Configure rho

| Page | What it gives you |
| --- | --- |
| [Configuration](configuration.md) | The config file, every key, profiles, and the variables. |
| [Providers](providers.md) | OpenRouter, AWS Bedrock, and Azure OpenAI. Keys and models. |
| [Permissions](permissions.md) | What rho may do on your machine, and how you narrow it. |

## Give rho more to work with

| Page | What it gives you |
| --- | --- |
| [Tools](tools.md) | Every tool the model can call, and its limits. |
| [Skills](skills.md) | Teach rho a procedure it can load when it needs it. |
| [Project instructions](project-instructions.md) | Carry your project rules through `AGENTS.md`. |
| [MCP servers](mcp.md) | Add tools from another process. |
| [Subagents](subagents.md) | Hand a piece of work to a subagent. |

## Two sentences on what rho is

rho is a harness, not a model. It sends your prompt to a provider you choose, runs the tools
the model asks for, and shows you the result.

The parts are separate crates. You can build rho without a provider, without the terminal
interface, or with your own version of either.

## If you want the engineering documents

This guide is for people who use rho. The rest of `docs/` is for people who build it. Start
at [../index.md](../index.md) for that map. Its language is internal, and it names features
by id.
