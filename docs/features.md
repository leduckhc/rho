# rho feature catalogue

This document is the contract for all later stages. The architect writes specs against these IDs. Developers implement against those specs. The website copy comes from the outcomes listed here.

**Status values:**
- `sprint-1` — shipped in sprint 1.
- `sprint-2` — shipped after sprint 1, in sprint 2. The code is in the tree.
- `planned` — on the roadmap. No code yet.
- `partial` — some of the feature is in the tree, and the row says which part. The rest
  reports that it is not built, so a user never meets silence.
- `considered` — not decided; requires a design spike first.
- `superseded` — the feature shipped and was then replaced. The row stays, because an
  older spec still names it, and a dangling reference is worse than a history note. The
  row says which feature replaced it.

A status states what the code proves, not what a plan intends. `agentic-workflow.yaml`
holds the template that every unit of work follows. A filled copy lives in
`.rho-work/tracks/`, and `.rho-work/progress.md` records what each sprint delivered.

A status states what the code proves, not what a plan intends. `workflow-sprint-2.yaml`
holds the current sprint. Sprint 2 is at work on config (`rho-config`), the session log,
and the ACP frontend (`rho-acp`). Those two crates hold an empty `lib.rs` today.

**Extension point** describes how a third party replaces or extends the feature without forking rho.

---

## Core runtime

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-agent-loop | Agent loop | The user sends a prompt. rho returns a streamed answer that includes tool calls and results. | `rho-core` | `sprint-1` | Callers supply a `Provider` impl and a `ToolSet`. The loop itself is not pluggable in sprint 1. Planned: a `LoopPolicy` trait so callers can replace retry and stop logic. |
| F-event-stream | Event stream | Every agent action appears as a typed event on a channel. Actions include tokens, tool calls, tool results, errors, and stops. No action is hidden from the caller. | `rho-core` | `sprint-1` | `rho-acp` and `rho-tui` both consume the same stream. A third-party frontend subscribes to the same channel. |
| F-turn-model | Turn model | One turn is one LLM response plus all tool calls and results that the response triggers. Turns repeat until `AgentEnd` is emitted. The stop reason is one of `EndTurn`, `MaxTokens`, `Refusal`, `Canceled`, or `MaxTurnRequests`. | `rho-core` | `sprint-1` | No external extension point in sprint 1. Planned: hooks fire before and after each turn (see F-hook-trait-tier-1). |
| F-cancellation | Cancellation | Pressing Ctrl-C cancels the current turn. In-flight HTTP requests are aborted. No task leaks. | `rho-core` | `sprint-1` | Any holder of the `CancellationToken` can cancel. A frontend cancels by dropping its token. |
| F-auto-retry | Auto-retry | On a 429 or 5xx response, rho retries with exponential backoff and jitter. It never retries a 4xx client error. | `rho-core` | `sprint-1` | A `RetryPolicy` trait (planned, not sprint-1) will let callers replace the backoff formula. Until then, the built-in policy is fixed. |
| F-auto-continue | Auto-continue | When a turn ends with open todos, rho sends the model back to work without user input. | `rho-core` | `planned` | A `ContinuePolicy` trait will let callers control the auto-continue trigger condition. |
| F-background-tasks | Background tasks | Long-running shell commands become named tasks the agent can list, tail, cancel, or wait on. The agent never writes polling loops. | `rho-core`, `rho-tools` | `sprint-2` | Tools register tasks on a shared `TaskRegistry`. Any tool can create or query tasks. |
| F-message-queue | Message queue | A user message that arrives during a turn is queued. It is never dropped, and it never lands in the middle of a provider request. The queue is bounded in messages and in bytes, so one large message cannot spend the host's memory. | `rho-core` | `sprint-2` | Callers push a message from any thread. The queue is part of the session API. `MessageQueue::with_limits` states both bounds, and `--max-agent-steer-bytes` sets the child queue cap. See `SPEC-steering`. |
| F-message-steering | Message steering | A queued message reaches the model after the current tool calls finish. It arrives before the next provider request. Arrival order is kept. | `rho-core` | `planned` | The TUI and `rho-acp` both steer through the same queue. See `SPEC-steering` and F-steer-command. |

---

## Providers

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-provider-trait | Provider trait | Any struct that implements `Provider` works in the agent loop. rho ships three providers. A caller adds a fourth without changing rho. | `rho-core` | `sprint-1` | Implement the `Provider` trait in any crate and pass an instance to the agent loop. No fork required. |
| F-openrouter-provider | OpenRouter provider | rho streams answers and tool calls from any model on OpenRouter over `POST /api/v1/chat/completions` with SSE. Reasoning tokens pass through: the delta reader takes the first non-empty of `reasoning_content`, `reasoning`, and `reasoning_text`. The effort level does not reach this crate yet. | `rho-provider-openrouter` | `sprint-1` | Callers set the base URL to any OpenAI-compatible endpoint to use a different service. |
| F-aws-bedrock-provider | AWS Bedrock provider | rho streams answers and tool calls via Bedrock `ConverseStream`. SigV4 auth comes from the standard AWS credential chain: env vars, profile, SSO cache, IMDS. | `rho-provider-bedrock` | `sprint-1` | Callers supply a custom `CredentialProvider` (planned) to replace the standard chain. Until then, the chain is fixed. |
| F-azure-openai-provider | Azure OpenAI provider | rho streams answers and tool calls from Azure OpenAI `/responses`. Two auth modes: API key, and Entra token with audience `https://cognitiveservices.azure.com/`. | `rho-provider-azure` | `sprint-1` | Two auth modes are built in: API key and Entra token. Adding a new auth mode requires a change in `rho-provider-azure`. |
| F-model-registry | Model registry | rho maintains a list of available models per provider. The caller selects a model by ID. | `rho-config` | `planned` | A third party adds models via the config file. Alternatively, pass a `ModelDescriptor` slice at startup. |
| F-custom-provider-extension | Custom provider extension | A third party ships a crate that implements `Provider` and lists it as a cargo dependency. rho uses it without modification. | `n/a (caller crate)`, `rho-provider-testkit` | `sprint-2` | The `Provider` trait is the full extension point. No other mechanism is needed. |

---

## Tools

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-tool-trait | Tool trait | Any struct that implements `Tool` is callable by the agent. Every tool declares a JSON schema for its inputs and a `ToolKind` via `kind()`. | `rho-core` | `sprint-1` | Implement `Tool` in any crate and register it with the `ToolSet`. No fork required. |
| F-file-read-tool | File read tool | The agent reads a file at a given path. The path is confined to the session root. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `read`. |
| F-file-write-tool | File write tool | The agent writes a file at a given path. The path is confined to the session root. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `write`. |
| F-file-edit-tool | File edit tool | The agent replaces an exact text region in a file. The tool fails if the region is not found. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `edit`. |
| F-directory-list-tool | Directory list tool | The agent lists the files in a directory. Dotfiles and common noise paths are hidden by default. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `list`. |
| F-glob-tool | Glob tool | The agent searches for files by glob pattern within the session root. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `glob`. |
| F-grep-tool | Grep tool | The agent searches file contents by regex within the session root. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `grep`. |
| F-bash-tool | Bash tool | The agent runs a shell command with a timeout. Output streams back to the model. The command runs in the session root. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `bash`. Callers can wrap the built-in impl to add a permission gate. |
| F-path-confinement | Path confinement | Every built-in file tool resolves the path against the session root. Any path that escapes the root is rejected. | `rho-tools` | `sprint-1` | Callers set the session root at startup. No third-party extension point for the confinement logic itself; this is a security boundary. |
| F-tool-approval-gate | Tool approval gate | The caller supplies an `ApprovalPolicy` at startup. The policy runs before each tool call. It returns allow or block. Two policies ship: read-only and allow-all. | `rho-core` | `sprint-1` | Any caller supplies a different policy. No fork required. |
| F-ask-approval-policy | Ask approval policy | rho asks before a mutating tool call, and waits for an answer. A timeout is a denial. A closed channel is a denial. | `rho-core` | `planned` | Implement `ApprovalPolicy` for a different interaction. The Ask policy is one impl among several. See `SPEC-approval`. |
| F-approval-mode-resolution | Approval mode resolution | rho defaults to Ask where a human or a client can answer. It defaults to read-only where nobody can answer. The resolved mode is stated at startup. | `rho-core`, `rho-cli` | `planned` | A config value or a flag narrows the resolved mode. Only an explicit user value widens it. See `SPEC-approval`. |
| F-permission-over-acp | Permission over ACP | rho asks an ACP client with `session/request_permission`. A client that does not declare the capability gets read-only, never allow-all. | `rho-acp` | `planned` | The ACP protocol is the extension point. Any client renders its own prompt. |
| F-tui-approval-prompt | TUI approval prompt | The TUI shows an approval prompt for a mutating tool call. The prompt never blocks the input editor. It never blocks the event stream. | `rho-tui` | `planned` | Implement a different `ApprovalPolicy`, or a richer interface through F-approval-ui-extensions. See `SPEC-approval`. |
| F-approval-ui-extensions | Approval UI extensions | Richer approval interfaces beyond a simple TUI prompt are supported. These include remote approval, policy rules, and per-tool overrides. | `rho-tui` | `planned` | Implement a custom `ApprovalGate` closure and pass it at startup (requires F-tool-approval-gate). |
| F-todo-tool | Todo tool | A structured todo list the agent uses to record tasks, confidence scores, and completion status. The agent checks confidence before marking done. | `rho-tools` | `planned` | Replace by registering a different `Tool` impl under the name `todo`. |
| F-bash-os-sandbox | Bash OS sandbox | The agent runs `bash` under an OS sandbox. `confined` limits writes to the session root and the scratch directory. `strict` also denies the network. When confinement is asked for and no OS backend exists, `bash` refuses the command. | `rho-tools` | `sprint-2` | Set `SessionConfig::sandbox` or pass `--sandbox <mode>`. The macOS `sandbox-exec` path is behind one function, so a replacement is small. See `SPEC-bash-sandbox`. |
| F-bash-streams-to-the-store | Bash streams to the result store | `bash` writes its output into the result store as it runs, and keeps only a bounded window in memory. So the model can reach a whole long command output, not the first 100,000 bytes. Today `bash` caps its own output first, so a store holds no more than that. | `rho-tools` | `planned` | The store is already a trait. `bash` gains a writer, and the memory bound of D-bash-line-cap stays. See D-bash-cap-limits-the-store. |
| F-tool-description-contract | Tool description contract | Every built-in tool description states what the tool is for, when to use it, and when not to. A `ToolKind` and a schema alone do not stop a wrong call. | `rho-tools` | `planned` | The text is part of each `Tool` impl. A third-party tool follows the same shape, and a CI guard checks that a built-in description has both clauses. |
| F-ask-user-tool | Ask-the-user tool | The model asks the user one blocking question and waits for the answer. A frontend that cannot ask gets a refusal, so the model states the blocker instead of guessing. | `rho-tools` | `planned` | The asking channel is a trait, so the TUI, ACP, and a headless caller each answer in their own way. |
| F-ranked-file-search | Ranked file search | The agent searches for a concept when it does not know the exact symbol. rho scores files by keyword and returns a ranked list with a sample line. There is no index and no model call. | `rho-tools` | `planned` | Replace by registering a different `Tool` under the name `search`. N-04 still refuses an embedding model. |
| F-image-input | Image input | The user attaches an image, and a model that reads images natively receives it in the same request. rho accepts PNG, JPEG, GIF, and WebP, each under a size cap. | `rho-core`, `rho-tui` | `planned` | A `Provider` declares whether it accepts an image. A provider that does not is routed to F-image-text-adapter. |
| F-image-text-adapter | Image text adapter | A model that cannot read an image still gets the evidence. rho sends the image to a second model the caller configured, and returns its text. rho hard-codes no model id. | `rho-core` | `planned` | The adapter is a trait. With no adapter configured, rho states that the image cannot be read. See D-no-fixed-helper-model. |
| F-undo-tracked-change | Undo a tracked change | rho reverses the most recent file change a tool made. Repeating it walks back through older changes. It never touches git history or a change made outside rho. | `rho-core` | `planned` | The change log is read from the session file, so any frontend offers the same undo. |
| F-permission-rules | Permission rules | The user saves allow and deny rules, matched by wildcard against the tool and its target. Deny beats allow, and rule order never decides the outcome. `F-scoped-command-guardrails` owns the matcher this rule set feeds. | `rho-core`, `rho-config` | `planned` | Rules are config data, so a new rule needs no code. The matcher sits behind `ApprovalPolicy`, so a caller replaces it. See D-deny-beats-allow. |
| F-session-grants | Session grants | An answer of "yes, and do not ask again" grants the displayed scope for this session only. rho writes no grant to a config file, and a resume restores none. | `rho-core` | `planned` | No extension point. A grant that outlived its session would be a permission the user forgot giving. See D-resume-never-widens. |

---

## Hooks and plugins

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-hook-trait-tier-1 | Hook trait (Tier 1) | A struct implements the `Hook` trait and runs at lifecycle hook points. Sprint 1 delivers the trait, its ordering guarantee, and the `before_tool_call` and `after_tool_result` hook points. | `rho-core` | `sprint-1` | Implement `Hook` in any crate and pass an instance to the agent loop. |
| F-lifecycle-hook-points | Lifecycle hook points | Hooks fire at these lifecycle points: session start, before provider request, after provider response, and before tool call. They also fire after tool result, at turn end, and at session end. | `rho-core` | `planned` | The set of hook points is fixed per sprint. New points require an interface change. |
| F-out-of-process-plugin-tier-2 | Out-of-process plugin (Tier 2) | A subprocess in any language connects over stdio JSON-RPC. It lists its tools and serves tool calls. A crashed plugin does not take down the session. | `rho-plugin` | `sprint-1` | Write a plugin in any language. The JSON-RPC protocol is the extension point. |
| F-plugin-schema-cache | Plugin schema cache | rho caches the tool schemas advertised by each plugin on disk. At startup, schemas appear in the first provider request without waiting for the plugin process to connect. | `rho-plugin` | `planned` | Third-party plugins do not need to change. The cache is transparent. |
| F-slash-commands | Slash commands | The user types `/command` in the TUI or ACP client. An in-tree handler or a Tier-1 hook responds. The TUI part is built. `/` opens the list. Typing filters it. The arrows, tab, and a left click select a row. `/help` and `/quit` run. `/model` and `/sessions` each report that they are not built yet, and the list marks them. `F-tui-guide` built `/guide`. A command that does nothing reads as a defect. | `rho-core` | `partial` | A `CommandHandler` impl registers a new slash command without forking. The handler trait is not built yet. So a new command still needs an entry in `rho-tui::slash_commands`. |
| F-skills-filesystem | Skills (filesystem) | rho discovers `SKILL.md` files in configured directories. Skill descriptions appear in the system prompt. The agent loads the full file on demand. | `rho-skills` | `sprint-2` | Add a directory to the `skill_paths` config key. No code change required. |
| F-prompt-templates | Prompt templates | The user invokes a `.md` file in a configured directory as a template. rho expands it before sending. | `rho-core` | `planned` | Add a directory to the `prompt_paths` config key. No code change required. |

---

## Sessions and persistence

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-append-only-session-log | Append-only session log | Every message, tool call, tool result, and event is appended to a JSONL file on disk. The file is never rewritten. The codec is one seam: `serde_json` by default, and `sonic-rs` behind the `fast-json` feature. | `rho-core` | `sprint-2` | A third party reads the file directly; the format is stable and documented. Compile with `--features fast-json` for the faster codec. See `ADR-jsonl-codec`. |
| F-session-resume | Session resume | The user passes a session file path and rho continues the conversation from the last message. | `rho-core` | `sprint-2` | No external extension point. Callers choose the session file path. |
| F-session-branching | Session branching | The user navigates to an earlier turn and continues from that point. rho creates a new branch in the same file. The original branch is not deleted. | `rho-core` | `sprint-2` | No external extension point in sprint 1. |
| F-ephemeral-mode | Ephemeral mode | The user opts out of session persistence. No file is written. | `rho-core` | `sprint-2` | Callers set `session_path = None` at startup. |
| F-pi-session-import | Pi session import | The `rho-session-import-pi` crate converts a pi session file to rho's JSONL format. The conversion is one-way. The original pi file is not changed. | `rho-session-import-pi` | `sprint-2` | No extension point. The converter is a standalone crate. |
| F-session-close | Session close | A close ends a session cleanly and writes the last record. A second close changes nothing, because close is idempotent. | `rho-core` | `sprint-2` | Frontends call close. `rho-acp` maps `session/close` to it. |
| F-session-cancel-without-close | Session cancel without close | A cancel stops the running turn and keeps the session open. The file holds no half-written tool pairing. | `rho-core` | `sprint-2` | Any holder of the `CancelToken` cancels. `rho-acp` maps `session/cancel` to it. |
| F-session-store | Session store | Session files live under `~/.rho/sessions/<project-key>/`. The key comes from the git common directory, so every worktree of one repository shares one pool. rho parses the `.git` entry itself, and spawns no git process. | `rho-core` | `spec` | A caller injects the store root and the project key. `SessionStore` stays a struct, so a different backend is a fork. See `SPEC-session-store-wiring`. |
| F-session-list | Session list | rho lists sessions with a short summary each. A row is built from a bounded head read and a bounded tail read, so no whole file is decoded. A file rho cannot read is one row marked unreadable, never a failed list. | `rho-core` | `sprint-2` | `rho-acp` maps `session/list` to it. A caller reads the header record directly. |
| F-session-title | Session title | A session titles itself from the first line of the first prompt. `rho sessions name` and `/name` write an explicit title. The newest title wins. No model call, so a title costs nothing. | `rho-core` | `spec` | The title is a leaf record, so another frontend writes one the same way. See `D-a-session-title-costs-nothing`. |
| F-session-crash-continue | Crash continue | On start rho finds a session in this project with no close record, and offers to continue it once. A closed session is never offered. | `rho-cli` | `spec` | `SessionStore::newest_open` is the seam. Any frontend offers the same. |
| F-session-delete | Session delete | A delete removes one session file. The command states what happens to a branch inside that file. | `rho-core` | `sprint-2` | `rho-acp` maps `session/delete` to it. |
| F-session-fork | Session fork | A fork copies a session to a new id from a chosen record. The original file is not changed. | `rho-core` | `sprint-2` | A caller picks the record to fork from. F-session-branching branches inside one file instead. |

---

## Context engineering

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-stable-prefix-for-kv-cache | Stable prefix for KV cache | The system prompt and tool list are sent in the first request. They never change mid-session. The provider KV cache stays warm across turns. | `rho-core` | `sprint-1` | No extension point. This is a fixed protocol discipline. |
| F-full-tool-list-at-turn-one | Full tool list at turn one | rho advertises the complete tool list in the first provider request. It does not wait for plugins to connect. Plugin schemas load from cache. | `rho-core` | `planned` | Callers filter the tool list by passing a `ToolSet` with only the tools they want. |
| F-context-compaction | Context compaction | When the context window nears its limit, rho summarizes old messages and replaces them with the summary. The summary preserves goal, decisions, progress, and changed files. | `rho-core` | `planned` | A `CompactionStrategy` trait (planned) lets callers replace the summary prompt or use a different model. |
| F-branch-summary | Branch summary | When the user navigates away from a branch, rho summarizes it. The summary is injected at the new position. | `rho-core` | `planned` | Same `CompactionStrategy` trait as F-context-compaction. |
| F-short-system-prompt | Short system prompt | The default system prompt is under 1000 tokens. rho does not put operating manuals in the system prompt. | `rho-core` | `sprint-1` | Callers pass a `system_prompt` string to override the default. |
| F-context-hook | Context hook | A hook fires before each provider request. The hook may add, remove, or reorder messages. | `rho-core` | `planned` | Implement a `Hook` that handles the `before_request` lifecycle point (requires F-hook-trait-tier-1). |
| F-project-instructions | Project instructions | rho reads `AGENTS.md`. It gathers the user file, then every file above the session root and below home, then the root's own file. The narrowest scope wins per conflict, and a direct user instruction beats every file. The text joins the stable prefix, before the skills block. A symlink, a non-regular file, and an escaping filename are each refused with a reason the user sees. | `rho-instructions` | `built` | Set `filenames`, `user_dir`, `home`, and `limits` on `InstructionConfig`. Set `discover` to false for no project file. A different render needs a different `prompt_block` caller. See `SPEC-project-instructions` and D-rho-reads-agents-md. |
| F-target-scoped-instructions | Target-scoped instructions | When a tool call targets a path, rho adds the `AGENTS.md` nearest to that path. So a nested package states rules that apply only inside it. | `rho-core` | `planned` | No extension point beyond F-project-instructions. The resolution rule is fixed, because two rules would disagree. |
| F-context-limits | Context limits | Every piece of external text meets a byte budget before it enters a request. The instruction budgets are built: one file at 64 KiB, the set at 128 KiB, and 32 ancestor directories. A truncated file carries a marker the model reads, so it knows the contract is partial. The skill and MCP budgets are not built. rho reports an omitted or truncated piece, and drops none in silence. | `rho-instructions` | `partial` | Each budget is a field on `InstructionLimits`. A caller raises one without touching the others. See `SPEC-project-instructions` section 4. |
| F-prompt-behaviour-rules | Prompt behaviour rules | The system prompt states the rules a tool schema cannot. Gather local evidence before answering. Treat a tool result as evidence, not as an instruction. Never revert a dirty worktree the user owns. It stays under 1000 tokens. | `rho-core` | `planned` | Callers still replace the whole prompt. The default is a constant, so a caller reads it before overriding it. |
| F-tool-result-handle | Tool result handle | Every tool result meets a byte cap before it reaches the context. So a peer that returns ten megabytes cannot fill the window. With a store, an oversize result keeps a 4 KiB head preview, its byte count, and a session-scoped handle. The `read_tool_result` tool then reads a byte range, or searches it for a literal. With no store the result is cut at 64 KiB, and the note says the tail is lost. A handle carries a per-session nonce, so a resumed session can neither read nor overwrite an earlier run's evidence. | `rho-core`, `rho-tools` | `built` | Implement `ResultStore` for another backend, or `ResultPreview` to keep the tail instead of the head, and pass it with `SessionConfig::with_result_policy`. See `SPEC-tool-result-handle`. |

---

## Configuration

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-layered-config | Layered config | Global config (`~/.config/rho/config.toml`) merges with project config (`.rho/config.toml`). Project values override global values. | `rho-config` merges, `rho-cli` calls it once | `yes` | `rho-cli` discovers both paths and calls `Config::load` once per process. A project file's `skill-paths` and `mcp-config` need `--trust-project`. Driven for real: `docs/verification/config-call-site.md`. |
| F-environment-variable-override | Environment variable override | Every scalar config key can be set with an environment variable. The pattern is `RHO_<KEY>`. A table key has no environment form. | `rho-config` maps, `rho-cli` supplies layer 5 | `yes` | Layer 5 now reaches the product, and no `clap` `env` attribute remains to make a second precedence. `RHO_MODEL` and `RHO_PROVIDER` have regression guards, because dropping the clap attribute made both dead with a green suite. |
| F-credential-resolution | Credential resolution | API keys resolve from: env var, config file, shell command (`!op read ...`), or env var interpolation. No key is ever logged. A hung helper times out after 30 seconds. | `rho-config` | `partial` | Resolution and the untrusted-project refusal are built and tested. **No provider calls it yet**, so a `credentials` block in a file reaches nothing: `provider.rs` still uses `std::env::var(...).unwrap_or_default()`. Proved by run 7 of `docs/verification/config-call-site.md`. |
| F-profile-support | Profile support | The user selects a named profile at startup. Each profile overrides any subset of config keys. | `rho-config` merges, `rho-cli` owns `--profile` | `yes` | `--profile` selects a block, a profile key beats a plain file key, and an undefined profile is an error. |

---

## Frontends — TUI

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-reasoning-trace-and-replay | Reasoning trace and replay | A model's reasoning is two block kinds. A trace is for the reader and never reaches a provider, and the type enforces that. A replay block carries one opaque payload, tagged with the provider and the model that wrote it. A payload from another provider or another model is dropped. | `rho-core` | `delivered` | A new provider puts its own wire shape inside the payload. Shared code never reads inside it, so no new provider changes `rho-core`. `ProviderState::for_owner` holds the owner rule in one place. |
| F-bedrock-reasoning-replay | Bedrock reasoning replay | rho captures the signature Bedrock sends with a reasoning block. It replays the text and the signature unmodified on the next request. The AWS API requires that inside a tool loop. Encrypted reasoning rides as base64 and replays as a blob. | `rho-provider-bedrock` | `delivered` | A corrupted signature was rejected live with a 400, which proves the block travels. See `docs/verification/reasoning-replay.md`. |
| F-reasoning-payload-bounds | Reasoning payload bounds | A payload over the record cap is dropped whole, and the drop is reported. A truncated token is useless, and an unreadable record breaks a whole session. A payload never reaches a log at any level, and it is stored verbatim, because a rewritten payload cannot replay. | `rho-core` | `delivered` | `redact_block` and `cap_block` each name every reasoning block, so no wildcard hides a new block kind. |
| F-openrouter-reasoning-effort | OpenRouter reasoning effort | The chosen level reaches OpenRouter as `reasoning.effort`. `off` sends `enabled: false`. `xhigh` maps to `high`, because the host has no `xhigh` and rho must not invent a value it rejects. | `rho-provider-openrouter` | `delivered` | The mapping lives in one function. A live A/B proved the field does the work: `high` printed reasoning, and no flag printed none. |
| F-replay-scope | Replay scope | Only the current tool loop replays its reasoning. A block from before the last prompt stays in the transcript and never travels again. The prompt is append-only, so an old block would cost its bytes on every later turn. | `rho-provider-bedrock` | `delivered` | The growth was worked out from the code path, not measured. See `D-replay-only-the-current-loop`. |
| F-reasoning-effort | Reasoning effort | The user sets how hard the model thinks. The levels are `off`, `low`, `medium`, `high`, and `xhigh`. The sources are `--reasoning-effort`, `RHO_REASONING_EFFORT`, and the `reasoning-effort` key. Unset means the provider's own default, so rho sends no field. A bad level stops the run and names its source. | `rho-core`, `rho-config`, `rho-cli` | `delivered` | `ReasoningEffort` is one user-facing word. A provider crate maps it to its own wire shape, so a new provider needs no change to shared code. See `docs/verification/reasoning-effort.md`. |
| F-bedrock-asks-for-thinking | Bedrock asks for thinking | rho sends `thinking` in `additionalModelRequestFields` for a Claude model at version 3.7 or above, with the budget of the chosen level. Without the ask, Claude wrote `<thinking>` tags into the answer. A model that cannot think is asked for nothing. A thinking request also drops the temperature. It raises `max_tokens` above the budget. | `rho-provider-bedrock` | `delivered` | `model_supports_thinking` is one function with a table test of real model ids. It fails closed on an id it cannot read. |
| F-headless-reasoning-output | Headless reasoning output | `rho run` prints the answer on stdout and the reasoning on stderr, so a pipe stays clean. A leading `<thinking>` tag becomes reasoning on this path too, which it did not before. Reasoning prints in `full` and `live` only. | `rho-cli` | `delivered` | `reasoning_is_shown` states the rule in one place, and a source guard proves the loop calls the splitter. |
| F-minimal-tui | Minimal TUI | The user runs `rho` and sees a transcript, a streaming answer, and thinking blocks. Tool rows, an input editor, and a status line are also present. | `rho-tui` | `sprint-1` | `rho-tui` is an optional crate. A third party ships a different TUI or omits TUI entirely. |
| F-pure-function-render | Pure-function render | TUI state is a pure function of events. Tests drive the renderer with a test backend, not a real terminal. | `rho-tui` | `sprint-1` | No external extension point. This is an internal design constraint. |
| F-non-blocking-input | Non-blocking input | The input editor never blocks on model work. The user types while the model streams. Ctrl-C cancels the turn. A second Ctrl-C exits. | `rho-tui` | `sprint-1` | No external extension point. |
| F-themes | Themes | The user sets a color theme in the config file. The TUI applies the theme to all rendered output. | `rho-tui` | `planned` | Add a theme file to the `theme_paths` config key. No code change required. |
| F-keybindings | Keybindings | The user rebinds any TUI key action in the config file. | `rho-tui` | `planned` | Add or override bindings in the config file. No code change required. |
| F-custom-tool-renderer | Custom tool renderer | A third party registers a custom render function for a named tool. Tool rows display custom content. | `rho-tui` | `planned` | Implement a `ToolRenderer` trait and register it with `rho-tui` at startup. |
| F-duration-ladder | Duration ladder | A session, a turn, a tool call, and a thinking block each report an elapsed time. The ladder rounds once at the top, so a carry cannot escape a tier. | `rho-tui` | `partial` | The ladder is a fixed format, proven over 10835518 values. No production code writes a row duration yet, so only a fixture shows one. `SPEC-tui-inline-and-composer` moves the write into the reducer. |
| F-duration-slot | Reserved duration slot | Every duration renders in a seven-column right-aligned slot, so a live tick never reflows the text beside it. | `rho-tui` | `partial` | The width is the widest rung of the ladder. The slot draws, and the span that fills it is not written yet outside a fixture. |
| F-paste-collapsing | Paste collapsing | A paste over one thousand characters shows as one chip, and the full text still reaches the model. A repeat of the same size gets a numeric suffix. | `rho-tui` | `sprint-4` | No extension point. The threshold is a constant in the spec. A paste now reaches the composer, which holds the full text aside. |
| F-paste-burst | Paste burst detection | On a terminal with no bracketed paste, a burst of key events collapses into one paste. So a pasted question mark cannot open the help. | `rho-tui` | `partial` | The detector is built and tested. The composer takes a paste now, and no caller routes a key burst through the detector yet. |
| F-attachments | Image attachments | An image attaches as a bounded chip. An oversize image is refused with the limit stated, and a path outside the session root is refused. | `rho-tui` | `partial` | `attach_image` is built and tested, and confinement is a security boundary. No key reaches it yet, so the chip cannot appear. |
| F-concise-mode | Concise mode | A tool call and a thinking block collapse to one line each, and expand on a key. The mode is opt-in. | `rho-tui` | `partial` | The fold model is built and no key folds a row. The caret is not drawn either, because a caret promises a key. Both return together. |
| F-working-motion | Working motion | One motion marks a working state. A raised-cosine band sweeps the working word, as a pure function of a tick, so a test asserts a frame. | `rho-tui` | `sprint-3` | An extension replaces the working word. The sweep reads no clock, by design. |
| F-theme-roles | Theme roles | Six roles carry the interface. Each resolves to a 256-colour value, a 16-colour fallback, and a no-colour modifier. | `rho-tui` | `sprint-3` | Style by role, never by a raw colour, so a user theme keeps working. See `SPEC-tui-plugins`. |
| F-generated-help | Generated help | The help screen is generated from the binding table, so the keys and the help cannot drift apart. | `rho-tui` | `sprint-3` | Add a binding, and the help row follows. No second list to maintain. |
| F-tui-guide | The two minute tour | `/guide` opens a paged panel: three pages, moved by the arrows or the space bar, closed by Esc. Every key and command it prints comes from the binding table and the slash list, so the tour cannot drift. Page one names the live model and provider. The command list now marks an unbuilt command, and a guard forbids a starter hint that names one. | `rho-tui` | `sprint-4` | No extension point. The pages are product copy inside `rho-tui`, and another frontend ships its own. See `SPEC-tui-guide` and `D-the-guide-is-a-paged-panel`. |
| F-tui-plugin-view | Transcript plugin view | A third party contributes transcript rows through a Tier-1 trait. It returns data, never a frame, and it cannot read the transcript without a grant. | `rho-tui` | `planned` | Implement the view trait. See `SPEC-tui-plugins`. |
| F-inline-band | Inline band | rho draws a fixed band at the bottom of the terminal. The band holds the live rows, the composer, and the footer. rho never enters the alternate screen, so the output above the band stays. | `rho-tui` | `sprint-4` | No extension point. See `D-inline-viewport-not-alternate-screen`. |
| F-freeze-upward | Freeze a row upward | A row that can never change again moves into the terminal's own scrollback. So the wheel, a drag-select, and the terminal search all work on the transcript. | `rho-tui` | `sprint-4` | No extension point. A row freezes only when it is final, and only in order. See `D-a-frozen-row-never-repaints`. |
| F-optional-mouse | Optional mouse capture | rho leaves the mouse to the terminal, so drag-select keeps working. One config key turns capture on for the wheel and the clickable list. | `rho-tui`, `rho-config` | `sprint-4` | Pass `--mouse`, or set `RHO_TUI_MOUSE`. The `tui-mouse` file key parses and waits for the config call site. |
| F-alternate-screen | Alternate screen | rho opens the alternate screen at startup and owns the whole terminal. It restores every mode on every exit path, including a panic, a `SIGTERM`, and a `SIGHUP`. | `rho-tui` | `sprint-4` | No extension point. See `D-alternate-screen-after-all` and `SPEC-tui-alternate-screen`. |
| F-owned-scroll | Owned scroll state | rho owns the transcript scroll, because the alternate screen has no scrollback. The wheel, `pageup`, `pagedown`, `home`, and `end` move the view, and the offset clamps where it changes. | `rho-tui` | `sprint-4` | No extension point. See `SPEC-tui-alternate-screen` section 3. |
| F-markdown-colour | Markdown as colour | A heading, a fence, a code line, a quote, a list marker, and a rule each take a colour. Inline bold, italic, and code take a style too. Every marker is removed. A table draws with aligned columns and a bold header. Syntax highlighting is out of scope. | `rho-tui` | `sprint-4` | A row is a `StyledLine` of styled runs, and `put` owns the width. The subset is closed by design: a new element edits the scanner. See `D-markdown-line-level-first` and `SPEC-tui-markdown`. |
| F-block-text-shape | Block text keeps its shape | An assistant answer keeps its line breaks, its blank lines, and the indent of each line. A list stays a list, and a code fence stays code. Every escape sequence is still dropped. | `rho-tui`, `rho-redact` | `sprint-4` | `sanitize_block` keeps a newline and drops an escape. `sanitize_line` stays for a one-row row. No flag turns the filter off. See `D-block-text-keeps-its-shape`. |
| F-startup-notice | Startup notice | A startup notice draws in the transcript, wrapped, with the `!` glyph and the `Warn` role. rho used to print a notice and then open the alternate screen over it, so the user never read one. | `rho-tui`, `rho-cli` | `sprint-4` | A caller hands notices to `App::with_notices`. A row is `Row::Notice`, so a new frontend must answer for it. See `D-a-notice-reaches-the-transcript`. |
| F-mouse-capture | Mouse capture | rho captures the mouse by default. The alternate screen has no scrollback, so the wheel is the only way to scroll. `--no-mouse` gives the mouse back to the terminal. | `rho-tui`, `rho-config` | `sprint-4` | Pass `--no-mouse`, or set `RHO_TUI_MOUSE`. Every target terminal keeps a modifier bypass for drag-select. See `D-the-wheel-needs-capture`. |
| F-composer-editing | Composer editing | The draft is a multi-row composer with paste chips and a cursor. It answers the newline keys and the readline motions. | `rho-tui` | `sprint-4` | No extension point. A chip is one unit for the cursor. See `SPEC-tui-scroll-copy-composer`. |
| F-draft-history | Draft history | The user recalls a submitted draft with the arrows, and searches the session history with a key. | `rho-tui` | `sprint-4` | No extension point. The history lives for the session only. |
| F-external-editor | External editor | The user edits the draft, or reads a selection, in `$EDITOR`. A failed editor run never discards the draft. | `rho-tui` | `sprint-4` | The editor comes from `$VISUAL`, then `$EDITOR`, then `vi`. |
| F-time-to-first-frame | Time to first frame | rho renders the TUI before the first token arrives. The time to first frame is measured and recorded in `docs/benchmarks.md`. | `rho-tui` | `sprint-1` | No external extension point. This is a quality gate, not a feature. |

---

## Frontends — ACP

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-acp-frontend | ACP frontend | A client connects over stdio JSON-RPC and drives rho headlessly. Any UI can embed rho without a terminal. | `rho-acp` | `planned` | `rho-acp` is an optional crate. Any process speaks the protocol. The protocol is documented. |
| F-prompt-command | Prompt command | The client sends `session/prompt`. The agent sends `session/update` notifications during the turn. The agent responds to `session/prompt` with a `stopReason` when the turn ends. | `rho-acp` | `planned` | The protocol is the extension point. Any language can implement a client. |
| F-steer-command | Steer command | The client sends a steering message while the agent is running. The message delivers after the current tool calls finish. | `rho-acp` | `planned` | Same protocol extension point as F-prompt-command. |
| F-abort-command | Abort command | The client sends a `session/cancel` notification. rho cancels the current turn and responds to `session/prompt` with `stopReason: cancelled`. | `rho-acp` | `planned` | Same protocol extension point as F-prompt-command. |
| F-session-commands-over-acp | Session commands over ACP | The client creates new sessions, switches sessions, and forks sessions over the protocol. | `rho-acp` | `planned` | Same protocol extension point as F-prompt-command. |
| F-extension-ui-sub-protocol | Extension UI sub-protocol | An ACP client responds to `extension_ui_request` events for select, confirm, and input dialogs from hooks. | `rho-acp` | `planned` | Clients that do not implement the sub-protocol receive a default value after a timeout. |

---

## Frontends — JSONL

`docs/specs/20260819-102749-SPEC-jsonl-frontend.md` owns these rows. This protocol lands before
ACP, because it is small. ACP stays the interop target and arrives as a bridge. See
decision D-jsonl-before-acp.

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-jsonl-frontend | JSONL frontend | A client drives rho headlessly over stdin and stdout, one JSON object per line. Any process that reads and writes lines can embed rho. | `rho-jsonl` | `planned` | `rho-jsonl` is an optional crate. Any language implements a client. The protocol is documented. |
| F-jsonl-prompt | JSONL prompt command | The client sends a `prompt` command. The agent streams events. The run ends with a settled event. | `rho-jsonl` | `planned` | The protocol is the extension point. Any language can implement a client. |
| F-jsonl-steer | JSONL steer command | The client sends a `steer` command while the agent runs. The message delivers after the current tool calls finish. | `rho-jsonl` | `planned` | Same protocol extension point as F-jsonl-prompt. |
| F-jsonl-abort | JSONL abort command | The client sends an `abort` command. rho cancels the current turn and settles with a cancelled stop reason. | `rho-jsonl` | `planned` | Same protocol extension point as F-jsonl-prompt. |
| F-jsonl-session-commands | JSONL session commands | The client reads state, switches models, starts a session, and lists messages and commands over the protocol. | `rho-jsonl` | `planned` | Same protocol extension point as F-jsonl-prompt. |
| F-jsonl-dialog-sub-protocol | JSONL dialog sub-protocol | The agent asks the client for a select, a confirm, an input, or a notify. A dialog blocks until the client answers. The agent side owns the timeout. | `rho-jsonl` | `planned` | A client that answers no dialog receives the default value after the timeout. |

---

## Extensions — tier 2

An extension adds tools, hooks, or both. It is a normal crate behind a cargo feature, so
a user who does not want it does not compile it. See `docs/extending.md` for the three
tiers and for the design notes behind each row here.

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-scoped-command-guardrails | Scoped command guardrails | The user allows `bash(git log *)` and denies `bash(rm *)`. A denied command never runs. Deny beats allow. | `new: rho-guard` | `planned` | Config states the patterns. A caller can add its own `Hook` for a rule the patterns cannot express. |
| F-default-destructive-shape-deny-list | Default destructive-shape deny list | rho refuses a known-destructive command without any configuration. The list covers `rm -rf`, `dd`, `mkfs`, a piped installer, `sudo`, a force push, and a fork bomb. | `new: rho-guard` | `planned` | The list is data, so a caller replaces or extends it. |
| F-narrower-write-confinement | Narrower write confinement | The user confines `write` and `edit` to a glob such as `src/**`, tighter than the session root. | `new: rho-guard` | `planned` | Config states the globs. |
| F-budget-caps | Budget caps | A session stops at a token, money, or wall-clock limit. So a runaway loop cannot spend without a bound. | `new: rho-guard` | `planned` | A caller sets the caps. A `Hook` enforces them. |
| F-web-fetch | Web fetch | The agent fetches a URL and reads it as text. The body is capped. A redirect into a private address is refused. | `new: rho-web` | `planned` | Replace by registering a different `Tool` under the name `web_fetch`. |
| F-web-search | Web search | The agent searches the web through a configured provider. The key is a `Secret`. | `new: rho-web` | `planned` | The provider is a trait, so a caller adds a search back end without forking. |
| F-browser-control | Browser control | The agent drives a real browser. It declares `ToolKind::Execute`, because it runs a program and can act on a logged-in session. | `new: rho-web` | `planned` | Replace by registering a different `Tool` under the name `browser`. |
| F-external-content-marking | External-content marking | Fetched text is marked as external, so a later rule can treat it as untrusted. Fetched text never widens a permission. | `new: rho-web` | `planned` | No extension point. This is a security boundary. |
| F-task-list | Task list | The model records its goals and their state, and the list survives a turn. | `new: rho-todo` | `planned` | Replace by registering a different `Tool` under the name `todo`. |
| F-task-confidence-scoring | Task confidence scoring | The model rates its confidence when a task is assigned and again when it is done. A large jump triggers a re-check. | `new: rho-todo` | `planned` | The threshold is configuration. |
| F-auto-continue-on-unfinished-work | Auto-continue on unfinished work | A turn that ends with unfinished tasks sends the model back to work. A transient failure retries, a permanent one stops. | `new: rho-todo` | `planned` | Needs a turn cap and a budget from F-budget-caps, so it cannot spend in silence. |
| F-durable-memory | Durable memory | A note store the agent writes only when the user asks it to remember. A note is never injected into every request. The agent reads a note back through the tool. | `new: rho-memory` | `planned` | The store is a trait. A caller points it at a file, a database, or nothing at all. |
| F-language-server-tools | Language-server tools | Diagnostics, a definition lookup, and a rename, from a language server. | `new: rho-lsp` | `considered` | One server per language, behind a trait. |
| F-cost-meter | Cost meter | A running token and money count, with a per-session cap. | `new: rho-cost` | `considered` | A `Hook` reads usage and enforces the cap. |

## Hook model gaps

`docs/extending.md` compares rho's hook model with pi's. These rows track the gaps.

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-terminate-on-block | Terminate on block | A hook stops the whole run, not only one tool call. So a guardrail can end a session that keeps trying a refused action. | `rho-core` | `planned` | `HookOutcome` gains a variant. |
| F-model-request-and-response-hooks | Model request and response hooks | A hook sees the request before it is sent and the response as it arrives. So an extension can meter cost or redact a prompt. | `rho-core` | `planned` | New `Hook` trait methods. Must not break the stable prompt prefix. |

A hook point and a slash command are one feature each, so this table no longer
restates them. See `F-lifecycle-hook-points` and `F-slash-commands` above.

## Subagents

`docs/specs/20260818-000223-SPEC-subagents.md` owns these rows. A subagent is another session on the same runtime, so a fan-out is cheap.

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-subagent-spawning | Subagent spawning | The model delegates work to a child. The child runs in a fresh conversation and returns only a summary. | `rho-tools` | `sprint-2` | Register a different `Tool` under the name `spawn_agent`. |
| F-policy-composition | Policy composition | A child runs under `BothPolicies`, so it can only be more restrictive than its parent. Escalation is unrepresentable. | `rho-core` | `sprint-2` | No extension point. This is a security boundary. See decision D-child-confined-by-composition. |
| F-tool-set-intersection | Tool-set intersection | A child's tool set is the parent's set filtered by the child's list. A name the parent lacks is dropped and reported. | `rho-core` | `sprint-2` | No extension point. This is a security boundary. |
| F-inheritance-rules | Inheritance rules | A child inherits the provider and the root, and may narrow the model and the sandbox. The root is never overridable. The sandbox may only narrow. | `rho-core` | `sprint-2` | No extension point. This is a security boundary. |
| F-four-subagent-limits | Four subagent limits | A depth cap, a per-parent cap, a process-wide cap, and a child timeout bound a fan-out. Each refusal names the limit. | `rho-core` | `sprint-2` | A caller sets `SubagentLimits`. |
| F-cycle-guard | Cycle guard | The spawn walk carries a visited set. A cycle in the parent chain is refused rather than looped. | `rho-core` | `sprint-2` | No extension point. |
| F-salvage-and-retry-cap | Salvage and retry cap | A child that dies without a report yields a failed result. A re-delegated task stops at the retry cap. | `rho-core` | `sprint-2` | A caller uses `RetryLedger`. |
| F-agent-events | Agent events | The parent stream shows a child through three events: spawned, progressed, and finished. | `rho-core` | `sprint-2` | New `AgentEvent` variants. A frontend renders them. |
| F-agent-definitions | Agent definitions | An agent is a markdown file with frontmatter. A project definition is withheld until the project is trusted, including one reached through a symlink from a user directory. A file that does not load is reported with its path, its reason, and its repair. | `rho-skills` | `built` | Author a definition file. The loader is shared with skills. A frontend may render `RejectedDefinition` itself. See decisions D-a-rejected-definition-is-reported and D-an-agent-symlink-cannot-smuggle-trust. |
| F-background-subagent | Background subagent | The parent starts a child and returns at once. It polls with `agent_status`, and a finished child stays reportable after its handle is gone. | `rho-tools` | `built` | Pass `background: true` to `spawn_agent`. |
| F-agent-status | Agent status | The parent asks what a child is doing, or what it did. It reports turns, tokens, queued steers, the outcome, the summary, and the transcript path. | `rho-tools` | `built` | Register a different `Tool` under the name `agent_status`. |
| F-child-transcript | Child transcript | Every child streams a JSONL transcript to a per-user temp directory, and the parent is told the path. A child that never finished still leaves what it wrote. | `rho-core` | `built` | Read `AgentReport.transcript`, or implement a different writer. |
| F-steer-subagent | Steer a subagent | A host sends a running child a new instruction. The child reads it at its next turn boundary, so it never lands inside a provider request. A model can call `steer_agent` only once a child can outlive a turn. | `rho-tools` | `built` | Register a different `Tool` under the name `steer_agent`, or hold the registry and call `LiveAgent::steer`. |
| F-cancel-one-subagent | Cancel one subagent | The model stops one child. Its siblings and the parent keep running. | `rho-tools` | `built` | Register a different `Tool` under the name `cancel_agent`. |
| F-live-agent-handle | Live agent handle | A running child is addressable. A caller lists live children, reads one child's progress, and cancels one child without touching its siblings. | `rho-core` | `built` | `AgentRegistry::live_under`, `descendant`, `status`, and `cancel_descendant`. Each takes the calling node, because the unscoped views are private. A frontend renders the list. |
| F-agent-tool-call-budget | Agent tool-call budget | A run stops at a tool-call budget. A turn cap counts provider round trips, so it cannot bound a turn that asks for forty tools. | `rho-core` | `built` | `--max-agent-tool-calls`, or `SessionConfig::with_max_tool_calls`. |
| F-agent-fan-out | Agent fan-out | `spawn_agents` runs several children at once in one tool call. A refused task is a per-task result, and results report in request order. | `rho-tools` | `sprint-2` | Register a different `Tool` under the name `spawn_agents`. See decision D-fan-out-is-one-tool-call. |
| F-tool-list-keywords | Tool list keywords | A definition writes `tools: all` or `tools: *` to inherit every tool the parent holds, and `tools: none` to hold none. A keyword must stand alone, and a mixed line keeps the named tools and warns. The list takes a comma line or a YAML sequence, and a value of any other type refuses the file. | `rho-skills` | `built` | Author a definition file. See decisions D-a-tool-keyword-stands-alone and D-a-tool-list-accepts-a-yaml-sequence. |

`docs/specs/20260820-115320-SPEC-subagent-slots-handles-grace.md` owns the three rows below.
All three are proposed, and none is built.

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-agent-slot-queue | Agent slot queue | A spawn over a concurrency cap queues instead of refusing. It returns an id at once, and the child starts when a slot frees. A queued child can be polled, steered, and cancelled. `spawn_agent` and `spawn_agents` both use it, so a fan-out wider than the cap runs every task. | `rho-core`, `rho-tools` | `built` | `AgentNode::spawn_child` stays the immediate form, so a caller bypasses the queue. `--max-queued-per-parent` bounds the line, and `--queue-wait-secs` bounds how long one child waits in it. |
| F-agent-handles | Agent handles | A model addresses a child by a derived name, such as `explore-2`. It can also set its own name with the `alias` argument. The id stays the identity, and a name resolves only inside the caller's own descendants. `agent_status` with no id lists the children. | `rho-core`, `rho-tools` | `built` | `AgentRegistry::set_alias` names a child. `AgentRef` accepts an id or a name, so the old integer shape keeps working. |
| F-agent-grace-turns | Agent grace turns | rho warns a child a fixed number of turns before its turn cap, so the child writes its summary. The warning uses the steering queue, and it never displaces a user message. A child transcript records the delivery. | `rho-core` | `built` | `SessionConfig::with_grace_turns`, and `--agent-grace-turns`. Zero disables it, and a definition cannot set it. |

`docs/specs/20260820-115310-SPEC-subagent-worktree-isolation.md` owns the two rows below. Both
are proposed, and neither is built.

| F-subagent-workspace-isolation | Subagent workspace isolation | A child works in its own tree, so a fan-out that writes files is safe. Only a trusted caller grants isolation. A definition and the model may refuse it, and neither may demand it. | `rho-core` | `planned` | Implement the `Workspace` trait and install it in `SpawnEnv`. `rho-tools` ships the git one. |
| F-child-work-kept-on-a-branch | Child work kept on a branch | A child's changes are committed to a named branch when it stops, including after a cancel or a timeout. An unchanged tree leaves no branch, and a failed commit leaves the tree on disk. | `rho-tools` | `planned` | Implement `Workspace::reclaim` differently. Read `AgentReport.branch` and `AgentReport.isolation_root`. |
| F-isolation-orphan-recovery | Isolation orphan recovery | A worktree and its branch carry the agent, the id, and a UTC timestamp. So a crash leftover is unique and findable. A later run lists an orphan and never deletes it. | `rho-tools` | `planned` | Not pluggable. The naming is a recovery contract a human relies on. |

`docs/specs/20260819-102750-SPEC-agent-tasks.md` owns the four rows below. A child carries a
task, and rho verifies the result. See decision D-a-child-does-not-grade-itself.

| F-agent-task | Agent task | A child carries a goal, its declared artifacts, and its acceptance checks, not a bare prompt. The `spawn_agent` tool takes `artifacts`. | `rho-core` | `built` | Build an `AgentTask`, or pass `artifacts` to `spawn_agent`. |
| F-artifact-spec | Artifact spec | A deliverable rho can check: a file, a command that exits zero, or a named kind. A new kind is a new variant or a registered checker. A file path obeys `confine`. | `rho-core` | `built` | Register an `ArtifactChecker` for a named kind. |
| F-acceptance-gate | Acceptance gate | rho verifies the artifacts and runs the checks after the child stops. A child cannot certify its own work, because no public constructor builds a verdict. A failed gate reports `Rejected`, never `Done`. | `rho-core` | `built` | Implement the `Gate` trait. A `CommandRunner` supplies the sandbox. |
| F-unverified-child-claims | Unverified child claims | The child reports its open questions and what it did not check. These stay separate from the gate verdict, and they are never proof. | `rho-core` | `built` | No extension point. This is a security boundary. |

## MCP client

`docs/specs/20260817-215003-SPEC-mcp.md` owns these rows. An MCP server is a peer, not a trusted part
of rho. So every row below states its trust boundary.

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-mcp-client | MCP client | rho connects to an MCP server over stdio, lists its tools, and calls one. The server's tools join the tool set. | `rho-mcp` | `sprint-2` | Add a server to the config. No code change is required. |
| F-tool-name-namespacing | Tool name namespacing | A server tool appears under a server prefix. Two servers cannot collide. A server cannot shadow a built-in tool. | `rho-mcp` | `sprint-2` | No extension point. This is a security boundary. |
| F-schema-cache | Schema cache | rho caches each server's tool schemas on disk. So the first provider request holds the full tool list without a connect. | `rho-mcp` | `sprint-2` | The cache is transparent. A server needs no change. |
| F-server-trust-policy | Server trust policy | A server declares nothing about its own risk. rho treats every server tool as mutating. So a peer cannot bypass a read-only policy. | `rho-mcp` | `sprint-2` | No extension point. This is a security boundary. |
| F-server-limits | Server limits | A slow or noisy server meets four bounds. They are a connect timeout, a call timeout, a response size cap, and a line length cap. | `rho-mcp` | `sprint-2` | A caller sets the limits. |
| F-server-output-sanitation | Server output sanitation | rho sanitizes server text before the text reaches the model or the terminal. A control sequence cannot repaint the screen. | `rho-mcp` | `sprint-2` | No extension point. This is a security boundary. |
| F-http-and-sse-transports | HTTP and SSE transports | rho connects to a remote MCP server over HTTP with SSE, not only to a local subprocess. | `rho-mcp` | `planned` | The transport is a trait, so a caller adds one without a fork. |
| F-lazy-mcp-tool-discovery | Lazy MCP tool discovery | rho advertises a search tool and a select tool instead of every server tool. The model finds a tool, then loads one schema. So twenty servers do not fill the context window. | `rho-mcp` | `planned` | A caller that wants every schema at turn one keeps the current behaviour with a config key. This trades context for one round trip. |

## Observability and telemetry

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-structured-tracing | Structured tracing | Every agent action emits a `tracing` span with structured fields. A third party attaches any `tracing` subscriber. | `rho-core` | `planned` | Attach a `tracing::Subscriber` at startup. No rho code change required. |
| F-token-and-cost-accounting | Token and cost accounting | Every provider response reports input tokens, output tokens, cache read tokens, cache write tokens, and the cost the provider charged. The cost is measured, never estimated. Session totals are F-session-statistics. | `rho-core` | `sprint-2` | A hook (F-hook-trait-tier-1) fires after each response and receives the usage struct. |
| F-session-statistics | Session statistics | The caller reads session totals: message count, token totals, cost, context usage percent. | `rho-core` | `planned` | No external extension point. The statistics are read from the session state. |
| F-no-secrets-in-logs | No secrets in logs | The credential resolution path redacts key values before passing them to `tracing`. Redaction is done by construction, not by a filter. One crate owns redaction. | `rho-redact` | `sprint-2` | No extension point. This is a security constraint. |
| F-doctor-command | Doctor command | `rho doctor` checks local state and prints what is wrong. For a problem it can repair, it prints the exact command to run. | `rho-cli` | `planned` | Each check is a small function in a list, so a caller adds one check without touching the others. |

---

## Footprint and performance

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-low-resident-memory-per-session | Low resident memory per session | The goal is resident memory per session well below pi (76.5 MB, jcode.sh, August 2026). The rho number is `to be measured`; see `docs/benchmarks.md`. | `rho-core` | `sprint-1` | Callers compile only the crates they need. Feature flags exclude unused providers and frontends. |
| F-fast-cold-start | Fast cold start | The goal is time-to-first-input well below pi (596 ms, jcode.sh, August 2026). The rho number is `to be measured`; see `docs/benchmarks.md`. | `rho-cli` | `sprint-1` | No Rust runtime to start. No script engine to start. The main binary does not discover or compile extensions at startup. |
| F-release-build-size | Release build size | The release binary uses `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, and `strip = "symbols"`. The resulting binary size is `to be measured`; see `docs/benchmarks.md`. | `rho-cli` | `sprint-1` | Callers compile only the features they need with `--no-default-features`. |
| F-separate-feature-flags-for-providers | Separate feature flags for providers | Each provider is a cargo feature. A build with only one provider does not pay the compile time or binary size of the others. | `rho-cli` | `sprint-1` | Callers select features with `--features`. No code change required. |
| F-startup-latency-budget | Startup latency budget | Every noninteractive command has a wall-clock budget that CI checks. A command over its budget fails the build. The report prints a process baseline beside each result. | `n/a (CI config)` | `planned` | The budgets are data in one file. A caller adds a command and its limit in the same place. See D-startup-budget-is-a-ci-guard. |
| F-wasm-target | WebAssembly target | `rho-core` compiles to `wasm32`, with the network and the filesystem supplied by the host. A browser or a sandbox then embeds the agent loop. | `rho-core` | `considered` | The host supplies a transport trait and a storage trait. N-01 refuses WASM plugins, which is a different question. |

---

## Distribution

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-single-binary | Single binary | `cargo install rho` produces one self-contained binary with no runtime dependency. | `rho-cli` | `planned` | No extension point. The binary is the distribution unit. |
| F-library-first-api | Library-first API | `rho-core`, `rho-tools`, and `rho-plugin` are usable as library crates without the CLI or TUI. | `rho-core` | `sprint-1` | Third parties add `rho-core` to their `Cargo.toml` and build a custom frontend. |
| F-website | Website | `getrho.dev` explains what rho is, shows the install command, and links to docs and GitHub. | `n/a (site)` | `sprint-1` | The website is a separate Astro project. Contributors edit `web/`. |
| F-ci-pipeline | CI pipeline | Every pull request runs `cargo fmt --all --check` and `cargo clippy -D warnings`. It also runs `cargo test --workspace` and `cargo build` on Linux and macOS. | `n/a (CI config)` | `sprint-1` | No extension point. CI is fixed. |
