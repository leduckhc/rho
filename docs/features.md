# rho feature catalogue

This document is the contract for all later stages. The architect writes specs against these IDs. Developers implement against those specs. The website copy comes from the outcomes listed here.

**Status values:**
- `sprint-1` — shipped in sprint 1.
- `sprint-2` — shipped after sprint 1, in sprint 2. The code is in the tree.
- `planned` — on the roadmap. No code yet.
- `considered` — not decided; requires a design spike first.

A status states what the code proves, not what a plan intends. `workflow-sprint-2.yaml`
holds the current sprint. Sprint 2 is at work on config (`rho-config`), the session log,
and the ACP frontend (`rho-acp`). Those two crates hold an empty `lib.rs` today.

**Extension point** describes how a third party replaces or extends the feature without forking rho.

---

## Core runtime

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-01 | Agent loop | The user sends a prompt. rho returns a streamed answer that includes tool calls and results. | `rho-core` | `sprint-1` | Callers supply a `Provider` impl and a `ToolSet`. The loop itself is not pluggable in sprint 1. Planned: a `LoopPolicy` trait so callers can replace retry and stop logic. |
| F-02 | Event stream | Every agent action appears as a typed event on a channel. Actions include tokens, tool calls, tool results, errors, and stops. No action is hidden from the caller. | `rho-core` | `sprint-1` | `rho-acp` and `rho-tui` both consume the same stream. A third-party frontend subscribes to the same channel. |
| F-03 | Turn model | One turn is one LLM response plus all tool calls and results that the response triggers. Turns repeat until `AgentEnd` is emitted. The stop reason is one of `EndTurn`, `MaxTokens`, `Refusal`, `Canceled`, or `MaxTurnRequests`. | `rho-core` | `sprint-1` | No external extension point in sprint 1. Planned: hooks fire before and after each turn (see F-40). |
| F-04 | Cancellation | Pressing Ctrl-C cancels the current turn. In-flight HTTP requests are aborted. No task leaks. | `rho-core` | `sprint-1` | Any holder of the `CancellationToken` can cancel. A frontend cancels by dropping its token. |
| F-05 | Auto-retry | On a 429 or 5xx response, rho retries with exponential backoff and jitter. It never retries a 4xx client error. | `rho-core` | `sprint-1` | A `RetryPolicy` trait (planned, not sprint-1) will let callers replace the backoff formula. Until then, the built-in policy is fixed. |
| F-06 | Auto-continue | When a turn ends with open todos, rho sends the model back to work without user input. | `rho-core` | `planned` | A `ContinuePolicy` trait will let callers control the auto-continue trigger condition. |
| F-07 | Background tasks | Long-running shell commands become named tasks the agent can list, tail, cancel, or wait on. The agent never writes polling loops. | `rho-core`, `rho-tools` | `sprint-2` | Tools register tasks on a shared `TaskRegistry`. Any tool can create or query tasks. |
| F-08 | Message queue | A user message that arrives during a turn is queued. It is never dropped, and it never lands in the middle of a provider request. The queue is bounded. | `rho-core` | `planned` | Callers push a message from any thread. The queue is part of the session API. See `SPEC-15`. |
| F-09 | Message steering | A queued message reaches the model after the current tool calls finish. It arrives before the next provider request. Arrival order is kept. | `rho-core` | `planned` | The TUI and `rho-acp` both steer through the same queue. See `SPEC-15` and F-92. |

---

## Providers

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-10 | Provider trait | Any struct that implements `Provider` works in the agent loop. rho ships three providers. A caller adds a fourth without changing rho. | `rho-core` | `sprint-1` | Implement the `Provider` trait in any crate and pass an instance to the agent loop. No fork required. |
| F-11 | OpenRouter provider | rho streams answers and tool calls from any model on OpenRouter over `POST /api/v1/chat/completions` with SSE. Reasoning tokens pass through. | `rho-provider-openrouter` | `sprint-1` | Callers set the base URL to any OpenAI-compatible endpoint to use a different service. |
| F-12 | AWS Bedrock provider | rho streams answers and tool calls via Bedrock `ConverseStream`. SigV4 auth comes from the standard AWS credential chain: env vars, profile, SSO cache, IMDS. | `rho-provider-bedrock` | `sprint-1` | Callers supply a custom `CredentialProvider` (planned) to replace the standard chain. Until then, the chain is fixed. |
| F-13 | Azure OpenAI provider | rho streams answers and tool calls from Azure OpenAI `/responses`. Two auth modes: API key, and Entra token with audience `https://cognitiveservices.azure.com/`. | `rho-provider-azure` | `sprint-1` | Two auth modes are built in: API key and Entra token. Adding a new auth mode requires a change in `rho-provider-azure`. |
| F-14 | Model registry | rho maintains a list of available models per provider. The caller selects a model by ID. | `rho-config` | `planned` | A third party adds models via the config file. Alternatively, pass a `ModelDescriptor` slice at startup. |
| F-15 | Custom provider extension | A third party ships a crate that implements `Provider` and lists it as a cargo dependency. rho uses it without modification. | `n/a (caller crate)`, `rho-provider-testkit` | `sprint-2` | The `Provider` trait is the full extension point. No other mechanism is needed. |

---

## Tools

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-20 | Tool trait | Any struct that implements `Tool` is callable by the agent. Every tool declares a JSON schema for its inputs and a `ToolKind` via `kind()`. | `rho-core` | `sprint-1` | Implement `Tool` in any crate and register it with the `ToolSet`. No fork required. |
| F-21 | File read tool | The agent reads a file at a given path. The path is confined to the session root. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `read`. |
| F-22 | File write tool | The agent writes a file at a given path. The path is confined to the session root. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `write`. |
| F-23 | File edit tool | The agent replaces an exact text region in a file. The tool fails if the region is not found. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `edit`. |
| F-24 | Directory list tool | The agent lists the files in a directory. Dotfiles and common noise paths are hidden by default. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `list`. |
| F-25 | Glob tool | The agent searches for files by glob pattern within the session root. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `glob`. |
| F-26 | Grep tool | The agent searches file contents by regex within the session root. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `grep`. |
| F-27 | Bash tool | The agent runs a shell command with a timeout. Output streams back to the model. The command runs in the session root. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `bash`. Callers can wrap the built-in impl to add a permission gate. |
| F-28 | Path confinement | Every built-in file tool resolves the path against the session root. Any path that escapes the root is rejected. | `rho-tools` | `sprint-1` | Callers set the session root at startup. No third-party extension point for the confinement logic itself; this is a security boundary. |
| F-29 | Tool approval gate | The caller supplies an `ApprovalPolicy` at startup. The policy runs before each tool call. It returns allow or block. Two policies ship: read-only and allow-all. | `rho-core` | `sprint-1` | Any caller supplies a different policy. No fork required. |
| F-32 | Ask approval policy | rho asks before a mutating tool call, and waits for an answer. A timeout is a denial. A closed channel is a denial. | `rho-core` | `planned` | Implement `ApprovalPolicy` for a different interaction. The Ask policy is one impl among several. See `SPEC-16`. |
| F-33 | Approval mode resolution | rho defaults to Ask where a human or a client can answer. It defaults to read-only where nobody can answer. The resolved mode is stated at startup. | `rho-core`, `rho-cli` | `planned` | A config value or a flag narrows the resolved mode. Only an explicit user value widens it. See `SPEC-16`. |
| F-34 | Permission over ACP | rho asks an ACP client with `session/request_permission`. A client that does not declare the capability gets read-only, never allow-all. | `rho-acp` | `planned` | The ACP protocol is the extension point. Any client renders its own prompt. |
| F-35 | TUI approval prompt | The TUI shows an approval prompt for a mutating tool call. The prompt never blocks the input editor. It never blocks the event stream. | `rho-tui` | `planned` | Implement a different `ApprovalPolicy`, or a richer interface through F-130. See `SPEC-16`. |
| F-130 | Approval UI extensions | Richer approval interfaces beyond a simple TUI prompt are supported. These include remote approval, policy rules, and per-tool overrides. | `rho-tui` | `planned` | Implement a custom `ApprovalGate` closure and pass it at startup (requires F-29). |
| F-30 | Todo tool | A structured todo list the agent uses to record tasks, confidence scores, and completion status. The agent checks confidence before marking done. | `rho-tools` | `planned` | Replace by registering a different `Tool` impl under the name `todo`. |
| F-31 | Bash OS sandbox | The agent runs `bash` under an OS sandbox. `confined` limits writes to the session root and the scratch directory. `strict` also denies the network. When confinement is asked for and no OS backend exists, `bash` refuses the command. | `rho-tools` | `sprint-2` | Set `SessionConfig::sandbox` or pass `--sandbox <mode>`. The macOS `sandbox-exec` path is behind one function, so a replacement is small. See `SPEC-10`. |

---

## Hooks and plugins

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-40 | Hook trait (Tier 1) | A struct implements the `Hook` trait and runs at lifecycle hook points. Sprint 1 delivers the trait, its ordering guarantee, and the `before_tool_call` and `after_tool_result` hook points. | `rho-core` | `sprint-1` | Implement `Hook` in any crate and pass an instance to the agent loop. |
| F-41 | Lifecycle hook points | Hooks fire at these lifecycle points: session start, before provider request, after provider response, and before tool call. They also fire after tool result, at turn end, and at session end. | `rho-core` | `planned` | The set of hook points is fixed per sprint. New points require an interface change. |
| F-42 | Out-of-process plugin (Tier 2) | A subprocess in any language connects over stdio JSON-RPC. It lists its tools and serves tool calls. A crashed plugin does not take down the session. | `rho-plugin` | `sprint-1` | Write a plugin in any language. The JSON-RPC protocol is the extension point. |
| F-43 | Plugin schema cache | rho caches the tool schemas advertised by each plugin on disk. At startup, schemas appear in the first provider request without waiting for the plugin process to connect. | `rho-plugin` | `planned` | Third-party plugins do not need to change. The cache is transparent. |
| F-44 | Slash commands | The user types `/command` in the TUI or ACP client. An in-tree handler or a Tier-1 hook responds. | `rho-core` | `planned` | A `CommandHandler` impl registers a new slash command without forking. |
| F-45 | Skills (filesystem) | rho discovers `SKILL.md` files in configured directories. Skill descriptions appear in the system prompt. The agent loads the full file on demand. | `rho-skills` | `sprint-2` | Add a directory to the `skill_paths` config key. No code change required. |
| F-46 | Prompt templates | The user invokes a `.md` file in a configured directory as a template. rho expands it before sending. | `rho-core` | `planned` | Add a directory to the `prompt_paths` config key. No code change required. |

---

## Sessions and persistence

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-50 | Append-only session log | Every message, tool call, tool result, and event is appended to a JSONL file on disk. The file is never rewritten. The codec is one seam: `serde_json` by default, and `sonic-rs` behind the `fast-json` feature. | `rho-core` | `planned` | A third party reads the file directly; the format is stable and documented. Compile with `--features fast-json` for the faster codec. See `ADR-005`. |
| F-51 | Session resume | The user passes a session file path and rho continues the conversation from the last message. | `rho-core` | `planned` | No external extension point. Callers choose the session file path. |
| F-52 | Session branching | The user navigates to an earlier turn and continues from that point. rho creates a new branch in the same file. The original branch is not deleted. | `rho-core` | `planned` | No external extension point in sprint 1. |
| F-53 | Ephemeral mode | The user opts out of session persistence. No file is written. | `rho-core` | `planned` | Callers set `session_path = None` at startup. |
| F-54 | Pi session import | The `rho-session-import-pi` crate converts a pi session file to rho's JSONL format. The conversion is one-way. The original pi file is not changed. | `rho-session-import-pi` | `planned` | No extension point. The converter is a standalone crate. |
| F-55 | Session close | A close ends a session cleanly and writes the last record. A second close changes nothing, because close is idempotent. | `rho-core` | `planned` | Frontends call close. `rho-acp` maps `session/close` to it. |
| F-56 | Session cancel without close | A cancel stops the running turn and keeps the session open. The file holds no half-written tool pairing. | `rho-core` | `planned` | Any holder of the `CancelToken` cancels. `rho-acp` maps `session/cancel` to it. |
| F-57 | Session list | rho lists sessions with a short summary each. A list of 500 sessions reads 500 header records, not 500 whole files. | `rho-core` | `planned` | `rho-acp` maps `session/list` to it. A caller reads the header record directly. |
| F-58 | Session delete | A delete removes one session file. The command states what happens to a branch inside that file. | `rho-core` | `planned` | `rho-acp` maps `session/delete` to it. |
| F-59 | Session fork | A fork copies a session to a new id from a chosen record. The original file is not changed. | `rho-core` | `planned` | A caller picks the record to fork from. F-52 branches inside one file instead. |

---

## Context engineering

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-60 | Stable prefix for KV cache | The system prompt and tool list are sent in the first request. They never change mid-session. The provider KV cache stays warm across turns. | `rho-core` | `sprint-1` | No extension point. This is a fixed protocol discipline. |
| F-61 | Full tool list at turn one | rho advertises the complete tool list in the first provider request. It does not wait for plugins to connect. Plugin schemas load from cache. | `rho-core` | `planned` | Callers filter the tool list by passing a `ToolSet` with only the tools they want. |
| F-62 | Context compaction | When the context window nears its limit, rho summarizes old messages and replaces them with the summary. The summary preserves goal, decisions, progress, and changed files. | `rho-core` | `planned` | A `CompactionStrategy` trait (planned) lets callers replace the summary prompt or use a different model. |
| F-63 | Branch summary | When the user navigates away from a branch, rho summarizes it. The summary is injected at the new position. | `rho-core` | `planned` | Same `CompactionStrategy` trait as F-62. |
| F-64 | Short system prompt | The default system prompt is under 1000 tokens. rho does not put operating manuals in the system prompt. | `rho-core` | `sprint-1` | Callers pass a `system_prompt` string to override the default. |
| F-65 | Context hook | A hook fires before each provider request. The hook may add, remove, or reorder messages. | `rho-core` | `planned` | Implement a `Hook` that handles the `before_request` lifecycle point (requires F-40). |

---

## Configuration

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-70 | Layered config | Global config (`~/.config/rho/config.toml`) merges with project config (`.rho/config.toml`). Project values override global values. | `rho-config` | `planned` | Third parties add config keys by defining a `ConfigSchema` struct (planned). |
| F-71 | Environment variable override | Every config key can be set with an environment variable. The pattern is `RHO_<KEY>`. | `rho-config` | `planned` | No extension point. Env vars always override file values. |
| F-72 | Credential resolution | API keys resolve from: env var, config file, shell command (`!op read ...`), or env var interpolation. No key is ever logged. | `rho-config` | `planned` | Callers supply a `CredentialResolver` closure to add a custom source (planned). |
| F-73 | Profile support | The user selects a named profile at startup. Each profile overrides any subset of config keys. | `rho-config` | `planned` | Profiles are defined in the config file. No code extension needed. |

---

## Frontends — TUI

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-80 | Minimal TUI | The user runs `rho` and sees a transcript, a streaming answer, and thinking blocks. Tool rows, an input editor, and a status line are also present. | `rho-tui` | `sprint-1` | `rho-tui` is an optional crate. A third party ships a different TUI or omits TUI entirely. |
| F-81 | Pure-function render | TUI state is a pure function of events. Tests drive the renderer with a test backend, not a real terminal. | `rho-tui` | `sprint-1` | No external extension point. This is an internal design constraint. |
| F-82 | Non-blocking input | The input editor never blocks on model work. The user types while the model streams. Ctrl-C cancels the turn. A second Ctrl-C exits. | `rho-tui` | `sprint-1` | No external extension point. |
| F-83 | Themes | The user sets a color theme in the config file. The TUI applies the theme to all rendered output. | `rho-tui` | `planned` | Add a theme file to the `theme_paths` config key. No code change required. |
| F-84 | Keybindings | The user rebinds any TUI key action in the config file. | `rho-tui` | `planned` | Add or override bindings in the config file. No code change required. |
| F-85 | Custom tool renderer | A third party registers a custom render function for a named tool. Tool rows display custom content. | `rho-tui` | `planned` | Implement a `ToolRenderer` trait and register it with `rho-tui` at startup. |
| F-86 | Time to first frame | rho renders the TUI before the first token arrives. The time to first frame is measured and recorded in `docs/benchmarks.md`. | `rho-tui` | `sprint-1` | No external extension point. This is a quality gate, not a feature. |

---

## Frontends — ACP

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-90 | ACP frontend | A client connects over stdio JSON-RPC and drives rho headlessly. Any UI can embed rho without a terminal. | `rho-acp` | `planned` | `rho-acp` is an optional crate. Any process speaks the protocol. The protocol is documented. |
| F-91 | Prompt command | The client sends `session/prompt`. The agent sends `session/update` notifications during the turn. The agent responds to `session/prompt` with a `stopReason` when the turn ends. | `rho-acp` | `planned` | The protocol is the extension point. Any language can implement a client. |
| F-92 | Steer command | The client sends a steering message while the agent is running. The message delivers after the current tool calls finish. | `rho-acp` | `planned` | Same protocol extension point as F-91. |
| F-93 | Abort command | The client sends a `session/cancel` notification. rho cancels the current turn and responds to `session/prompt` with `stopReason: cancelled`. | `rho-acp` | `planned` | Same protocol extension point as F-91. |
| F-94 | Session commands over ACP | The client creates new sessions, switches sessions, and forks sessions over the protocol. | `rho-acp` | `planned` | Same protocol extension point as F-91. |
| F-95 | Extension UI sub-protocol | An ACP client responds to `extension_ui_request` events for select, confirm, and input dialogs from hooks. | `rho-acp` | `planned` | Clients that do not implement the sub-protocol receive a default value after a timeout. |

---

## Extensions — tier 2

An extension adds tools, hooks, or both. It is a normal crate behind a cargo feature, so
a user who does not want it does not compile it. See `docs/extending.md` for the three
tiers and for the design notes behind each row here.

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-140 | Scoped command guardrails | The user allows `bash(git log *)` and denies `bash(rm *)`. A denied command never runs. Deny beats allow. | `new: rho-guard` | `planned` | Config states the patterns. A caller can add its own `Hook` for a rule the patterns cannot express. |
| F-141 | Default destructive-shape deny list | rho refuses a known-destructive command without any configuration. The list covers `rm -rf`, `dd`, `mkfs`, a piped installer, `sudo`, a force push, and a fork bomb. | `new: rho-guard` | `planned` | The list is data, so a caller replaces or extends it. |
| F-142 | Narrower write confinement | The user confines `write` and `edit` to a glob such as `src/**`, tighter than the session root. | `new: rho-guard` | `planned` | Config states the globs. |
| F-143 | Budget caps | A session stops at a token, money, or wall-clock limit. So a runaway loop cannot spend without a bound. | `new: rho-guard` | `planned` | A caller sets the caps. A `Hook` enforces them. |
| F-144 | Web fetch | The agent fetches a URL and reads it as text. The body is capped. A redirect into a private address is refused. | `new: rho-web` | `planned` | Replace by registering a different `Tool` under the name `web_fetch`. |
| F-145 | Web search | The agent searches the web through a configured provider. The key is a `Secret`. | `new: rho-web` | `planned` | The provider is a trait, so a caller adds a search back end without forking. |
| F-146 | Browser control | The agent drives a real browser. It declares `ToolKind::Execute`, because it runs a program and can act on a logged-in session. | `new: rho-web` | `planned` | Replace by registering a different `Tool` under the name `browser`. |
| F-147 | External-content marking | Fetched text is marked as external, so a later rule can treat it as untrusted. Fetched text never widens a permission. | `new: rho-web` | `planned` | No extension point. This is a security boundary. |
| F-148 | Task list | The model records its goals and their state, and the list survives a turn. | `new: rho-todo` | `planned` | Replace by registering a different `Tool` under the name `todo`. |
| F-149 | Task confidence scoring | The model rates its confidence when a task is assigned and again when it is done. A large jump triggers a re-check. | `new: rho-todo` | `planned` | The threshold is configuration. |
| F-150 | Auto-continue on unfinished work | A turn that ends with unfinished tasks sends the model back to work. A transient failure retries, a permanent one stops. | `new: rho-todo` | `planned` | Needs a turn cap and a budget from F-143, so it cannot spend in silence. |
| F-151 | Durable memory | A note store the agent searches at the start of a turn. | `new: rho-memory` | `considered` | The store is a trait. |
| F-152 | Language-server tools | Diagnostics, a definition lookup, and a rename, from a language server. | `new: rho-lsp` | `considered` | One server per language, behind a trait. |
| F-153 | Cost meter | A running token and money count, with a per-session cap. | `new: rho-cost` | `considered` | A `Hook` reads usage and enforces the cap. |

## Hook model gaps

`docs/extending.md` compares rho's hook model with pi's. These rows track the gaps.

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-160 | Terminate on block | A hook stops the whole run, not only one tool call. So a guardrail can end a session that keeps trying a refused action. | `rho-core` | `planned` | `HookOutcome` gains a variant. |
| F-161 | Lifecycle hook points | A hook observes a session start, a turn start, a turn end, and a run end. | `rho-core` | `planned` | New `Hook` trait methods with default bodies, so an existing hook keeps compiling. |
| F-162 | Model request and response hooks | A hook sees the request before it is sent and the response as it arrives. So an extension can meter cost or redact a prompt. | `rho-core` | `planned` | New `Hook` trait methods. Must not break the stable prompt prefix. |
| F-163 | Slash commands | The user types `/name` and an extension answers. | `rho-core` | `planned` | A `CommandHandler` impl. This is F-44 restated for tier 2. |

## Subagents

`docs/specs/SPEC-11-subagents.md` owns these rows. A subagent is another session on the same runtime, so a fan-out is cheap.

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-170 | Subagent spawning | The model delegates work to a child. The child runs in a fresh conversation and returns only a summary. | `rho-tools` | `sprint-2` | Register a different `Tool` under the name `spawn_agent`. |
| F-171 | Policy composition | A child runs under `BothPolicies`, so it can only be more restrictive than its parent. Escalation is unrepresentable. | `rho-core` | `sprint-2` | No extension point. This is a security boundary. See decision D-036. |
| F-172 | Tool-set intersection | A child's tool set is the parent's set filtered by the child's list. A name the parent lacks is dropped and reported. | `rho-core` | `sprint-2` | No extension point. This is a security boundary. |
| F-173 | Inheritance rules | A child inherits the provider and the root, and may narrow the model and the sandbox. The root is never overridable. The sandbox may only narrow. | `rho-core` | `sprint-2` | No extension point. This is a security boundary. |
| F-174 | Four subagent limits | A depth cap, a per-parent cap, a process-wide cap, and a child timeout bound a fan-out. Each refusal names the limit. | `rho-core` | `sprint-2` | A caller sets `SubagentLimits`. |
| F-175 | Cycle guard | The spawn walk carries a visited set. A cycle in the parent chain is refused rather than looped. | `rho-core` | `sprint-2` | No extension point. |
| F-176 | Salvage and retry cap | A child that dies without a report yields a failed result. A re-delegated task stops at the retry cap. | `rho-core` | `sprint-2` | A caller uses `RetryLedger`. |
| F-177 | Agent events | The parent stream shows a child through three events: spawned, progressed, and finished. | `rho-core` | `sprint-2` | New `AgentEvent` variants. A frontend renders them. |
| F-178 | Agent definitions | An agent is a markdown file with frontmatter. A project definition is withheld until the project is trusted. | `rho-skills` | `sprint-2` | Author a definition file. The loader is shared with skills. |

## MCP client

`docs/specs/SPEC-09-mcp.md` owns these rows. An MCP server is a peer, not a trusted part
of rho. So every row below states its trust boundary.

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-180 | MCP client | rho connects to an MCP server over stdio, lists its tools, and calls one. The server's tools join the tool set. | `rho-mcp` | `sprint-2` | Add a server to the config. No code change is required. |
| F-181 | Tool name namespacing | A server tool appears under a server prefix. Two servers cannot collide. A server cannot shadow a built-in tool. | `rho-mcp` | `sprint-2` | No extension point. This is a security boundary. |
| F-182 | Schema cache | rho caches each server's tool schemas on disk. So the first provider request holds the full tool list without a connect. | `rho-mcp` | `sprint-2` | The cache is transparent. A server needs no change. |
| F-183 | Server trust policy | A server declares nothing about its own risk. rho treats every server tool as mutating. So a peer cannot bypass a read-only policy. | `rho-mcp` | `sprint-2` | No extension point. This is a security boundary. |
| F-184 | Server limits | A slow or noisy server meets four bounds. They are a connect timeout, a call timeout, a response size cap, and a line length cap. | `rho-mcp` | `sprint-2` | A caller sets the limits. |
| F-185 | Server output sanitation | rho sanitizes server text before the text reaches the model or the terminal. A control sequence cannot repaint the screen. | `rho-mcp` | `sprint-2` | No extension point. This is a security boundary. |
| F-186 | HTTP and SSE transports | rho connects to a remote MCP server over HTTP with SSE, not only to a local subprocess. | `rho-mcp` | `planned` | The transport is a trait, so a caller adds one without a fork. |

## Observability and telemetry

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-100 | Structured tracing | Every agent action emits a `tracing` span with structured fields. A third party attaches any `tracing` subscriber. | `rho-core` | `planned` | Attach a `tracing::Subscriber` at startup. No rho code change required. |
| F-101 | Token and cost accounting | Every provider response reports input tokens, output tokens, cache read tokens, cache write tokens, and the cost the provider charged. The cost is measured, never estimated. Session totals are F-102. | `rho-core` | `sprint-2` | A hook (F-40) fires after each response and receives the usage struct. |
| F-102 | Session statistics | The caller reads session totals: message count, token totals, cost, context usage percent. | `rho-core` | `planned` | No external extension point. The statistics are read from the session state. |
| F-103 | No secrets in logs | The credential resolution path redacts key values before passing them to `tracing`. Redaction is done by construction, not by a filter. One crate owns redaction. | `rho-redact` | `sprint-2` | No extension point. This is a security constraint. |

---

## Footprint and performance

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-110 | Low resident memory per session | The goal is resident memory per session well below pi (76.5 MB, jcode.sh, August 2026). The rho number is `to be measured`; see `docs/benchmarks.md`. | `rho-core` | `sprint-1` | Callers compile only the crates they need. Feature flags exclude unused providers and frontends. |
| F-111 | Fast cold start | The goal is time-to-first-input well below pi (596 ms, jcode.sh, August 2026). The rho number is `to be measured`; see `docs/benchmarks.md`. | `rho-cli` | `sprint-1` | No Rust runtime to start. No script engine to start. The main binary does not discover or compile extensions at startup. |
| F-112 | Release build size | The release binary uses `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, and `strip = "symbols"`. The resulting binary size is `to be measured`; see `docs/benchmarks.md`. | `rho-cli` | `sprint-1` | Callers compile only the features they need with `--no-default-features`. |
| F-113 | Separate feature flags for providers | Each provider is a cargo feature. A build with only one provider does not pay the compile time or binary size of the others. | `rho-cli` | `sprint-1` | Callers select features with `--features`. No code change required. |

---

## Distribution

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-120 | Single binary | `cargo install rho` produces one self-contained binary with no runtime dependency. | `rho-cli` | `planned` | No extension point. The binary is the distribution unit. |
| F-121 | Library-first API | `rho-core`, `rho-tools`, and `rho-plugin` are usable as library crates without the CLI or TUI. | `rho-core` | `sprint-1` | Third parties add `rho-core` to their `Cargo.toml` and build a custom frontend. |
| F-122 | Website | `getrho.dev` explains what rho is, shows the install command, and links to docs and GitHub. | `n/a (site)` | `sprint-1` | The website is a separate Astro project. Contributors edit `web/`. |
| F-123 | CI pipeline | Every pull request runs `cargo fmt --all --check` and `cargo clippy -D warnings`. It also runs `cargo test --workspace` and `cargo build` on Linux and macOS. | `n/a (CI config)` | `sprint-1` | No extension point. CI is fixed. |
