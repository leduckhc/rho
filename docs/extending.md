# Extending rho

rho ships a small fixed set of tools, and everything else is added. This page names the
three tiers, says which is which, and shows how to add to each.

## The three tiers

| Tier | Name | What it contributes | Where it lives |
| --- | --- | --- | --- |
| 0 | **Core tools** | The irreducible set. Read a file, change a file, find a file, run a command. | `rho-tools`, always compiled in |
| 1 | **Capability loaders** | Nothing of their own. They load somebody else's capability. | `rho-skills`, `rho-mcp`, `rho-plugin` |
| 2 | **Extensions** | New tools and new hooks. | Any crate, behind a cargo feature |

The tiers are named for what they **contribute**, not for how important they are. That
distinction is the useful one, because it decides where a new thing belongs.

### Tier 0: core tools

Nine tools. This list is deliberately closed.

| Tool | Kind | What it does |
| --- | --- | --- |
| `read` | `Read` | Read a file, with an optional offset and limit. |
| `write` | `Edit` | Write a whole file. |
| `edit` | `Edit` | Replace an exact span. `replace_all` changes every match. |
| `list` | `Read` | List a directory. |
| `glob` | `Search` | Find files by pattern. |
| `grep` | `Search` | Search file contents. |
| `bash` | `Execute` | Run a command. Backgrounds a long one by itself. |
| `task` | `Read` | List, read, or wait on a background task. |
| `task_cancel` | `Execute` | Kill a background task. |

Three properties define the tier, and each is a rule rather than an accident.

**No network.** Not one core tool opens a socket. So the core set works offline, in a
sandbox, and in a test, and `rho-tools` links no HTTP client. Web access is tier 2.

**Every path goes through `rho_core::confine`.** A path outside the session root is
refused, including one reached through a symlink. The single exception is `bash`, and
`SPEC-tool-interface` section 7 says plainly why: confining a shell needs a sandbox, not a path
check.

**Every tool declares a `ToolKind`.** The kind drives approval. `ToolKind::is_read_only`
is an allowlist with no wildcard arm, so a new kind is mutating until somebody classifies
it. See decision D-todo-in-a-green-stage.

**Why closed.** Every tool costs context in every request, whether the model uses it or
not. A tenth core tool must earn its place against that permanent cost. Anything
answering "some users will want this" is tier 2.

### Tier 1: capability loaders

A loader adds no capability of its own. It finds capability that somebody else wrote, and
presents it to the model. All three share one property: **what they load is untrusted**.

| Loader | Loads | Trust rule |
| --- | --- | --- |
| `rho-skills` | Instructions, as `SKILL.md` files | A project skill is withheld until the user trusts the root. See D-project-skill-needs-trust. |
| `rho-mcp` | Tools from an MCP server | Every tool reports `ToolKind::Other`, so a server never classifies itself. See D-mcp-does-not-classify-itself. |
| `rho-plugin` | Tools from a subprocess | Same rule. A plugin never classifies itself. See D-plugin-does-not-classify-itself. |

The repeated shape is not a coincidence. **A loader is a boundary, so each one fails
closed.** A capability arriving from outside is mutating until a human says otherwise.

### Tier 2: extensions

An extension adds tools, hooks, or both. It is a normal crate behind a cargo feature, so
a user who does not want it does not compile it.

The extension point is a trait, not a plugin format:

- `rho_core::Tool` adds a tool.
- `rho_core::Hook` observes and controls a tool call.

There is no registry to register with and no manifest to write. An extension is a crate
that returns `Arc<dyn Tool>` and `Arc<dyn Hook>` values.

## How to add a tool

```rust
use async_trait::async_trait;
use rho_core::{Tool, ToolContext, ToolError, ToolKind, ToolOutput, confine};

pub struct WordCountTool;

#[async_trait]
impl Tool for WordCountTool {
    fn name(&self) -> &str {
        "word_count"
    }
    fn description(&self) -> &str {
        "Count the words in a file."
    }
    fn kind(&self) -> ToolKind {
        // Declare a real kind. `Other` is treated as mutating, so a read-only session
        // would refuse this tool. See decision D-todo-in-a-green-stage.
        ToolKind::Read
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let path: String = args["path"].as_str().unwrap_or_default().to_string();
        // Never join a path yourself. `confine` resolves symlinks and refuses an escape.
        let path = confine(&ctx.session_root, std::path::Path::new(&path))?;
        let text = tokio::fs::read_to_string(&path)
            .await
            .map_err(|error| ToolError::Io(format!("cannot read {}: {error}.", path.display())))?;
        Ok(ToolOutput::text(format!("{}", text.split_whitespace().count())))
    }
}
```

Then register it beside the built-ins. Prove it conforms the same way a provider does: a
schema round-trip test, a confinement test, and an approval test.

## How to add a hook, and why hooks matter most

A hook is where rho gets interesting, and it is the part worth copying from pi. A hook
sees a tool call **before** it runs, and it can do three things:

1. **Observe.** Log it, count it, measure it.
2. **Rewrite it.** `ToolCallView` gives mutable access to the arguments, so a hook can
   patch a command or redirect a path before the tool sees it.
3. **Refuse it.** `HookOutcome::Block { reason }` stops the call. The model receives the
   reason as an error tool result, so it can choose differently.

```rust
use async_trait::async_trait;
use rho_core::{Hook, HookOutcome, ToolCallView, ToolOutput};

/// Refuse a command that removes a directory tree.
pub struct NoRecursiveDelete;

#[async_trait]
impl Hook for NoRecursiveDelete {
    fn name(&self) -> &str {
        "no-recursive-delete"
    }

    async fn before_tool_call(&self, call: &mut ToolCallView<'_>) -> HookOutcome {
        if call.name() != "bash" {
            return HookOutcome::Continue;
        }
        let command = call.args()["command"].as_str().unwrap_or_default();
        if command.contains("rm -rf") || command.contains("rm -fr") {
            // Say what to do instead. A refusal with no alternative wastes a turn.
            return HookOutcome::Block {
                reason: "this command removes a directory tree. Delete a named path \
                         instead, or ask the user to run it."
                    .to_string(),
            };
        }
        HookOutcome::Continue
    }

    async fn after_tool_result(&self, _name: &str, _output: &mut ToolOutput) {}
}
```

Registration order is run order, and the first `Block` wins. `after_tool_result` chains
like middleware: each hook sees the output the previous one left.

**The rule for a refusal message.** Say what to do instead. A model that hears only "no"
tries a variation, and the variation is often worse.

## The extension catalogue

None of these ships yet. Each has a feature ID in `docs/features.md`, and each needs its
own spec before any code, per `AGENTS.md` step 3.

### Guardrails, `rho-guard`

Hooks that refuse dangerous work. This is the most valuable extension, because it is what
makes an unattended run safe enough to leave alone.

rho already has three coarse controls: `ToolKind` approval, `--read-only`, and the
`--sandbox` OS confinement mode from `SPEC-bash-sandbox`. Approval and `--read-only` are
per-kind, so `bash` is all or nothing. The sandbox bounds an approved `bash` call
by write path and by network, but not by command shape. A guardrail extension adds
control at the level of the **command**, which is where a shape rule lives.

Grounded in what Claude Code exposes, the useful shape is a **scoped pattern** rather
than a tool name:

| Control | Example | Note |
| --- | --- | --- |
| Allow a scoped command | `bash(git log *)`, `bash(npm test)` | A pattern, so `git log` is allowed and `git push` is not. |
| Deny a scoped command | `bash(rm *)`, `bash(sudo *)` | Deny beats allow, always. |
| Confine a write | `write(src/**)`, `edit(src/**)` | Narrower than the session root. |
| Refuse a known-destructive shape | see the list below | On by default. |
| Cap a budget | tokens, money, wall time | Stops a runaway loop. |

The default deny list, and each entry has a reason:

| Shape | Why |
| --- | --- |
| `rm -rf`, `rm -fr` | Removes a tree, and a wrong argument removes the wrong tree. |
| `dd`, `mkfs`, `> /dev/` | Destroys a device. |
| `chmod -R 777`, `chown -R` on `/` | Wrecks a system. |
| `curl ... \| sh`, `wget ... \| bash` | Runs code nobody read. |
| `git push --force`, `git reset --hard` | Destroys work that is not local. |
| `sudo` | Escalates past every other guard. |
| `:(){ :\|:& };:` | A fork bomb. |
| `find ... -delete`, `truncate` | Deletes in bulk, quietly. |
| `history -c`, `> ~/.bash_history` | Hides what happened. |

**Two rules this extension must not break.**

**Prefer the operating system over a pattern.** `SPEC-bash-sandbox` adds a real confinement mode
for `bash`, using `sandbox-exec` on macOS and `bwrap` on Linux. It fails closed: ask for
confinement with no backend available and the command is refused. That is the boundary. A
pattern list is a filter in front of it.

Note what that is and is not. A sandbox profile is available to anybody, so shipping one
is a win in correctness, not an architectural lead. rho's only edge here is narrow: many
sessions in one process can share one sandbox supervisor.

**Pattern matching is not a sandbox.** A shell has a hundred routes to the same effect:
a variable, a here-document, `base64 -d`, an alias, a script file. A pattern catches the
careless case and the obvious injection. It does not stop a determined one. Say so in the
extension's own documentation, because a guard that oversells itself is worse than none.
`SPEC-tool-interface` section 7 makes the same admission about `bash`.

For a **real** boundary, use the OS sandbox, not a pattern. `SPEC-bash-sandbox` adds a
`--sandbox <off|confined|strict>` mode that confines an approved `bash` call with
`sandbox-exec` on macOS or `bwrap` on Linux. It limits writes to the session root
and the scratch directory, and `strict` also denies the network. So a guardrail
pattern and the sandbox do different jobs: the pattern refuses a shape the model
should not try, and the sandbox bounds what any approved command can do. A
guardrail extension should build on the sandbox for confinement, not try to
replace it with a string match.

**A refusal must teach.** Name the pattern, and name the safe alternative.

### Web access, `rho-web`

Three tools, and they are tier 2 precisely because they open sockets.

| Tool | Kind | Note |
| --- | --- | --- |
| `web_fetch` | `Fetch` | Fetch a URL and return readable text. Cap the body. Refuse a redirect to a private address. |
| `web_search` | `Fetch` | Search, through a configured provider. Needs a key, so the key is a `rho_core::Secret`. |
| `browser` | `Execute` | Drive a real browser. `Execute`, not `Fetch`, because it runs a program and can act on a logged-in session. |

The hazards to design against, not to discover later:

- **Server-side request forgery.** A model reads a URL from a file and fetches it. So
  refuse `localhost`, a link-local address, and a private range by default, and re-check
  after every redirect.
- **Fetched content is a prompt injection.** A page is untrusted text that reaches the
  model. It cannot be sanitised into safety. So mark it as external content, and never
  let it widen a permission.
- **A cookie jar is a credential.** A browser tool with a real profile can act as the
  user. That is `Execute`, and a read-only session must refuse it.

### Task list, `rho-todo`

A structured list the model keeps as it works. jcode reports that this changes behaviour
rather than only the display: an agent that records its goals finishes more of them.

Two ideas worth taking from jcode, both cheap:

- **A confidence score, recorded when a task is assigned and again when it is marked
  done.** jcode found the score at assignment is real signal, while the score at
  completion is almost always high. So a large jump means the work deserves a re-check.
- **Auto-continue.** When a turn ends with unfinished tasks, poke the model back to work
  instead of stopping. Retry a transient failure, and stop on a permanent one.

The second idea needs care: an auto-continue loop with no cap is a way to spend money in
silence. It needs a turn cap and a budget, which is why it belongs with the guardrail
extension.

### Others worth a spec

| Extension | Adds |
| --- | --- |
| `rho-memory` | A durable note store, searched at the start of a turn. |
| `rho-lsp` | Diagnostics and a rename, from a language server. |
| `rho-vcs` | A git-aware diff and a blame, cheaper than shelling out. |
| `rho-cost` | A token and money meter, with a per-session cap. |

## What pi does that rho does not

pi's extension model is broader than rho's, and the gap is worth stating rather than
implying parity. rho has the two hook points that matter most, and it lacks the rest.

| pi capability | rho today |
| --- | --- |
| Block a tool call with a reason | **yes**, `HookOutcome::Block` |
| Mutate tool arguments before execution | **yes**, `ToolCallView` |
| Patch a tool result, middleware style | **yes**, `after_tool_result` |
| `terminate`, to stop the whole run on a block | no. A block ends one call. |
| Session and turn lifecycle events | no |
| Model request and response events | no |
| Register a slash command | no. Feature F-slash-commands. |
| Register a keyboard shortcut or a renderer | no |
| Change the active tool set at run time | no. It would rewrite the prompt prefix, which `SPEC-core-runtime` forbids. |
| Compact the context from an extension | no |
| Extensions written in a scripting language | no. rho has tier-1 subprocess plugins instead. |

The last row is the real trade. pi loads a TypeScript file, so an extension is a few
lines in a scratch file. rho links a crate, so an extension is a compiled dependency.
rho gains a smaller binary, no runtime, and no compile-at-load cost. It loses the
five-minute experiment. Tier-1 plugins recover part of that, since a plugin can be a
shell script.

`SPEC-hooks-and-plugins` records which of these gaps are planned.
