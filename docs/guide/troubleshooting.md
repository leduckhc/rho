# Troubleshooting

rho 0.1.0. This page explains the messages rho prints most. Find the message, read the cause, run the fix.

## Find out more

Three tools give you more detail before you read the sections below.

**`--log`** turns on structured log output to stderr.
Pass a filter string: `--log info` shows all info lines.
Pass `--log rho_core=debug` to narrow to one module.

**`RHO_LOG`** does the same as `--log` but from the environment.
`RHO_LOG=rho_config=debug rho run "hi"` shows every config decision.

**`--version`** prints the build version and the compiled-in provider features.
Run it first when a provider message looks wrong.

Log output goes to stderr only. Redirect with `rho run "task" 2>rho.log` to keep a file.

---

## At startup

### Unknown config key

```
TOML parse error at line 3, column 1
  |
3 | moel = "..."
  | ^^^^
unknown field `moel`
```

The config file has a key rho does not recognise. A typo stops the run.
rho names the line and the key. Fix the spelling and run again.

This is the most common first-day error. The valid keys are in [configuration](configuration.md).

### Config file does not parse

```
rho: cannot parse the config file /home/you/.config/rho/config.toml: ...
```

The TOML is broken. rho names the path. Open it and fix the syntax, then run again.

### Config file cannot be read

```
rho: cannot read the config file /home/you/.config/rho/config.toml: permission denied
```

rho cannot open the file. Check the file permissions.

### Bad value for a security key

```
rho: the sandbox value "firm" is not valid: ...
rho: the approval value "maybe" is not valid: ...
```

rho stops when a security key holds an unknown value. It names the key. It does not guess.

Valid values for `sandbox`: `off`, `confined`, `strict`.
Valid values for `approval`: `allow-all`, `read-only`, `ask`.

### `approval = "ask"` is refused

```
approval = "ask" needs an interactive frontend, which this build does not have here. Use read-only or allow-all, or pass --read-only.
```

`ask` needs an interactive prompt. No interactive prompt exists in `rho run`.
Use `approval = "allow-all"` or `approval = "read-only"` instead.

### A bad value for a display key

When `tui-reasoning` holds a bad value, rho stops and names the key, the value, and the
valid names:

```
rho: the tui-reasoning value "loud" is not valid: unknown reasoning mode "loud": the valid names are off, summary, full, and live
```

Fix the value the message names. rho never guesses a display value.

---

## Choosing a provider or model

### Missing credential

```
set the OPENROUTER_API_KEY environment variable to your OpenRouter API key.
```

The variable is missing or empty. rho names the variable to set.
Export it in your shell:

```sh
export OPENROUTER_API_KEY=sk-...
rho run "hello"
```

An empty variable counts as missing.

### A key the provider rejects

```
rho: authentication failed: OpenRouter rejected the API key (status 401). Set a valid OPENROUTER_API_KEY.
```

The key reached the provider and the provider refused it. So the variable is set, and its
value is wrong, revoked, or from another account. Check the key in your provider dashboard,
then export it again.

This is the most common failure on a first day. rho cannot tell a typed key from a real one
until the provider answers.

### Unknown provider name

```
the provider "groq" is not known. Choose one of: openrouter, bedrock, azure. Set it with --provider or the RHO_PROVIDER variable.
```

The name you passed is not one of the three supported providers. Fix the spelling.

### Provider not compiled in

```
the provider "azure" is not in this build. Rebuild rho with the feature: cargo build --features azure.
```

This is a different error from an unknown name. The name is valid, but this binary was built without that provider.
Run the build command rho gives you:

```sh
cargo build --features azure
```

### No provider chosen

```
no provider was chosen. Set --provider or the RHO_PROVIDER variable to one of: openrouter, bedrock, azure.
```

Pass `--provider openrouter` or set `RHO_PROVIDER=openrouter`.

### No model given

rho picks a default and prints a notice:

```
rho: no model given, so using the default for openrouter: anthropic/claude-haiku-4.5. Set --model or RHO_MODEL to choose another.
```

This is not an error. Pass `--model your-model-id` to choose a different model.

---

## While a tool runs

### Path outside the session root

```
path ../secrets/.env escapes the session root
```

A tool tried to read or write outside your project directory. rho refuses the path.
Run rho from the directory that contains the files it should touch.

### Sandbox unavailable

```
the sandbox mode "confined" needs an OS sandbox, but none is available on this host. Install bubblewrap (bwrap) on Linux, or pass --sandbox off to run without confinement.
```

You set `--sandbox confined` or `--sandbox strict`, but the host has no sandbox binary.
On Linux, install `bwrap`:

```sh
sudo apt-get install bubblewrap   # Debian / Ubuntu
sudo dnf install bubblewrap       # Fedora
```

On macOS, `sandbox-exec` ships with the OS. If it is missing, pass `--sandbox off`.

### Large tool output cut

```
[rho cut this result at 65536 bytes of 120000. The rest is not available, because this session has no result store.]
```

Every tool result is capped at 64 KiB before it reaches the model. rho says so inline.
A result above 16 KiB is stored outside the context when a result store is attached. The model sees a preview and a handle, and can call `read_tool_result` to read the rest.
Without a store, the tail is gone. To avoid the cut, scope your request: ask rho to read one file or one directory at a time.

---

## While a capability loads

### Project skill not loaded

```
2 project skill(s) were found and not loaded: my-skill, other-skill. A skill can instruct the model and can carry scripts, so a skill from this repository stays off until you trust it. Pass --trust-project to load them.
```

rho found skills in the project but withheld them. Pass `--trust-project` to load them:

```sh
rho run "task" --trust-project
```

### Skill loaded with a warning

```
rho: skill my-skill: the name must use only lowercase letters, digits, and hyphens.
```

The skill loaded, but rho noticed a problem in its metadata. The message names the skill and the issue. Fix the skill's frontmatter or rename the file.

### Project config ignored fields

`skill-paths`, `mcp-config`, and `!command` credentials in a project config file are dropped unless you pass `--trust-project`.
A project config can change the model or sandbox without trust. It cannot add skills or credentials without trust.

### The model cannot delegate, and `spawn_agent` is missing

You wrote an agent definition, and the model says it has no way to spawn a subagent. Check
whether you passed `--no-skills`.

> `--no-skills` stops the skill search only. `--no-agents` stops the agent-definition
> search, so rho offers no subagent, and `--no-skills` still loads subagents. Pass
> `--no-agents` to drop subagents without dropping skills.

A definition file that does not load is a different case, and rho names it at start-up. See
[subagents](subagents.md).

When discovery works, rho says so at startup:

```
rho: 1 agent definition(s) available to spawn_agent: summariser
```

No such line means no definition was found.

### A `[credentials]` block has no effect

Nothing resolves a named credential in this version, and no provider asks for one. So the
whole table is inert. Give the provider its key through the environment instead, as
[providers](providers.md) shows.

### Instructions above your project are skipped

```
rho: project instructions: skipped the search of the directories above the session root because the session root is not below the home directory
```

You are working outside your home directory, for example in `/tmp` or `/opt`. rho then reads
no ancestor `AGENTS.md`. The file in the session root itself still loads. Move the project
under your home directory to get the wider search.

### Nothing was saved after the run

rho records no session file, so there is no history to reopen. Redirect `rho run` output to
keep a copy. See [sessions](sessions.md).

```
project instructions: skipped .rho/AGENTS.md because the file is a symlink
```

rho skips instruction files that are symlinks, non-regular files, or cannot be read. It prints the reason. Replace the symlink with a plain file.

### MCP tools do not appear

```
rho: 1 MCP server(s) are configured, and no tool schema is cached yet. rho is connecting now, and their tools are available in the next session.
```

rho writes the schema cache before it exits. The tools are available the next time you run rho.

If the notice repeats across many runs, the cache write may have failed. rho prints a notice
when a cache write fails. Check that `~/.rho/` is writable and that no other process holds a
stale lock file at `~/.rho/mcp-schema-cache.lock`.

If a server starts but its tools do not appear, check the tool names. rho rejects empty names,
names with path separators, names with control characters, names with a leading dot, and schemas
larger than 100 KB. A rejection is printed at startup.

### MCP config does not parse

```
cannot parse /home/you/.rho/mcp.json: expected value at line 3 column 1. Expected {"servers": [...]}.
```

The MCP config file has broken JSON. rho names the path and the shape it expects. Fix the file and run again.

### MCP server does not start

```
failed to start the MCP server my-server: No such file or directory (os error 2). Check the command path.
```

The command in the MCP config does not exist. Check the `command` field in `mcp.json` and verify the binary is on `PATH`.

### MCP handshake failed

```
the handshake with the MCP server my-server failed: ... Check the server speaks MCP over this transport.
```

The server process started but did not speak MCP. Check the server is an MCP server, not a plain script.

---

## In the terminal interface

### Slash command not built yet

> **Not built yet.** `/model`, `/sessions`, and `/guide` are in the slash list and do nothing.
> rho answers `<command> is not built yet. See F-slash-commands in docs/features.md.` Use
> `--model` and `--provider` on the command line instead.

`/help` and `/quit` are the two that work.
