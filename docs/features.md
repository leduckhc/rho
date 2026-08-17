# rho feature catalogue

This document is the contract for all later stages. The architect writes specs against these IDs. Developers implement against those specs. The website copy comes from the outcomes listed here.

**Status values:**
- `sprint-1` — ships in sprint 1.
- `planned` — on the roadmap after sprint 1.
- `considered` — not decided; requires a design spike first.

**Extension point** describes how a third party replaces or extends the feature without forking rho.

---

## Core runtime

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-01 | Agent loop | The user sends a prompt and receives a streamed answer with tool calls and results, all in one turn. | `rho-core` | `sprint-1` | Callers supply a `Provider` impl and a `ToolSet`. The loop itself is not pluggable in sprint 1. Planned: a `LoopPolicy` trait so callers can replace retry and stop logic. |
| F-02 | Event stream | Every agent action (token, tool call, tool result, error, done) appears as a typed event on a channel. No action is hidden from the caller. | `rho-core` | `sprint-1` | `rho-acp` and `rho-tui` both consume the same stream. A third-party frontend subscribes to the same channel. |
| F-03 | Turn model | One turn is one LLM response plus all tool calls and results that the response triggers. Turns repeat until the stop reason is `stop` or `error`. | `rho-core` | `sprint-1` | No external extension point in sprint 1. Planned: hooks fire before and after each turn (see F-20). |
| F-04 | Cancellation | Pressing Ctrl-C cancels the current turn. In-flight HTTP requests are aborted. No task leaks. | `rho-core` | `sprint-1` | Any holder of the `CancellationToken` can cancel. A frontend cancels by dropping its token. |
| F-05 | Auto-retry | On a 429 or 5xx response, rho retries with exponential backoff and jitter. It never retries a 4xx client error. | `rho-core` | `sprint-1` | A `RetryPolicy` trait (planned, not sprint-1) will let callers replace the backoff formula. Until then, the built-in policy is fixed. |
| F-06 | Auto-continue | When a turn ends with open todos, rho sends the model back to work without user input. | `rho-core` | `planned` | A `ContinuePolicy` trait will let callers control the auto-continue trigger condition. |
| F-07 | Background tasks | Long-running shell commands become named tasks the agent can list, tail, cancel, or wait on. The agent never writes polling loops. | `rho-core` | `planned` | Tools register tasks on a shared `TaskRegistry`. Any tool can create or query tasks. |

---

## Providers

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-10 | Provider trait | Any struct that implements `Provider` works in the agent loop. rho ships three providers. A caller adds a fourth without changing rho. | `rho-core` | `sprint-1` | Implement the `Provider` trait in any crate and pass an instance to the agent loop. No fork required. |
| F-11 | OpenRouter provider | rho streams answers and tool calls from any model on OpenRouter over `POST /api/v1/chat/completions` with SSE. Reasoning tokens pass through. | `rho-provider-openrouter` | `sprint-1` | Callers set the base URL to any OpenAI-compatible endpoint to use a different service. |
| F-12 | AWS Bedrock provider | rho streams answers and tool calls via Bedrock `ConverseStream`. SigV4 auth comes from the standard AWS credential chain: env vars, profile, SSO cache, IMDS. | `rho-provider-bedrock` | `sprint-1` | Callers supply a custom `CredentialProvider` (planned) to replace the standard chain. Until then, the chain is fixed. |
| F-13 | Azure OpenAI provider | rho streams answers and tool calls from Azure OpenAI `/responses`. Two auth modes: API key, and Entra token with audience `https://cognitiveservices.azure.com/`. | `rho-provider-azure` | `sprint-1` | Callers supply an `AzureCredential` enum variant; adding a new variant does not require forking. |
| F-14 | Model registry | rho maintains a list of available models per provider. The caller selects a model by ID. | `rho-config` | `planned` | A third party adds models by adding entries to the config file or by passing a `ModelDescriptor` slice at startup. |
| F-15 | Custom provider extension | A third party ships a crate that implements `Provider` and lists it as a cargo dependency. rho uses it without modification. | user crate | `planned` | The `Provider` trait is the full extension point. No other mechanism is needed. |

---

## Tools

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-20 | Tool trait | Any struct that implements `Tool` is callable by the agent. Every tool declares a JSON schema for its inputs. | `rho-core` | `sprint-1` | Implement `Tool` in any crate and register it with the `ToolSet`. No fork required. |
| F-21 | File read tool | The agent reads a file at a given path. The path is confined to the session root. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `read`. |
| F-22 | File write tool | The agent writes a file at a given path. The path is confined to the session root. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `write`. |
| F-23 | File edit tool | The agent replaces an exact text region in a file. The tool fails if the region is not found. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `edit`. |
| F-24 | Directory list tool | The agent lists the files in a directory. Dotfiles and common noise paths are hidden by default. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `list`. |
| F-25 | Glob tool | The agent searches for files by glob pattern within the session root. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `glob`. |
| F-26 | Grep tool | The agent searches file contents by regex within the session root. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `grep`. |
| F-27 | Bash tool | The agent runs a shell command with a timeout. Output streams back to the model. The command runs in the session root. | `rho-tools` | `sprint-1` | Replace by registering a different `Tool` impl under the name `bash`. Callers can wrap the built-in impl to add a permission gate. |
| F-28 | Path confinement | Every built-in file tool resolves the requested path against the session root and rejects any path that escapes it. | `rho-tools` | `sprint-1` | Callers set the session root at startup. No third-party extension point for the confinement logic itself; this is a security boundary. |
| F-29 | Tool approval gate | The caller registers an async callback that runs before each tool call. The callback may allow or block the call. | `rho-core` | `planned` | Any caller can supply an `ApprovalGate` closure at startup. |
| F-30 | Todo tool | A structured todo list the agent uses to record tasks, confidence scores, and completion status. The agent checks confidence before marking done. | `rho-tools` | `planned` | Replace by registering a different `Tool` impl under the name `todo`. |

---

## Hooks and plugins

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-40 | Hook trait (Tier 1) | A compiled Rust struct implements the `Hook` trait and fires synchronously at defined lifecycle points. Zero overhead. | `rho-core` | `planned` | Implement `Hook` in any crate and pass an instance to the agent loop. |
| F-41 | Lifecycle hook points | Hooks fire at: session start, before provider request, after provider response, before tool call, after tool result, turn end, session end. | `rho-core` | `planned` | The set of hook points is fixed per sprint. New points require an interface change. |
| F-42 | Out-of-process plugin (Tier 2) | A subprocess written in any language connects over stdio JSON-RPC, lists its tools, and serves tool calls. A crashed plugin does not take down the session. | `rho-plugin` | `sprint-1` | Write a plugin in any language. The JSON-RPC protocol is the extension point. |
| F-43 | Plugin schema cache | rho caches the tool schemas advertised by each plugin on disk. At startup, schemas appear in the first provider request without waiting for the plugin process to connect. | `rho-plugin` | `planned` | Third-party plugins do not need to change. The cache is transparent. |
| F-44 | Slash commands | The user types `/command` in the TUI or RPC client. An in-tree handler or a Tier-1 hook responds. | `rho-core` | `planned` | A `CommandHandler` impl registers a new slash command without forking. |
| F-45 | Skills (filesystem) | rho discovers `SKILL.md` files in configured directories. Skill descriptions appear in the system prompt. The agent loads the full file on demand. | `rho-core` | `planned` | Add a directory to the `skill_paths` config key. No code change required. |
| F-46 | Prompt templates | The user invokes a `.md` file in a configured directory as a template. rho expands it before sending. | `rho-core` | `planned` | Add a directory to the `prompt_paths` config key. No code change required. |

---

## Sessions and persistence

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-50 | Append-only session log | Every message, tool call, tool result, and event is appended to a JSONL file on disk. The file is never rewritten. | `rho-core` | `planned` | A third party reads the file directly; the format is stable and documented. |
| F-51 | Session resume | The user passes a session file path and rho continues the conversation from the last message. | `rho-core` | `planned` | No external extension point. Callers choose the session file path. |
| F-52 | Session branching | The user navigates to an earlier turn and continues from that point. rho creates a new branch in the same file. The original branch is not deleted. | `rho-core` | `planned` | No external extension point in sprint 1. |
| F-53 | Ephemeral mode | The user opts out of session persistence. No file is written. | `rho-core` | `planned` | Callers set `session_path = None` at startup. |

---

## Context engineering

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-60 | Stable prefix for KV cache | The system prompt and tool list are sent in the first request and never change mid-session. The provider KV cache stays warm across turns. | `rho-core` | `sprint-1` | No extension point. This is a fixed protocol discipline. |
| F-61 | Full tool list at turn one | rho advertises the complete tool list in the first provider request, not on demand. Plugin schemas load from cache if the plugin has not yet connected. | `rho-core` | `planned` | Callers filter the tool list by passing a `ToolSet` with only the tools they want. |
| F-62 | Context compaction | When the context window nears its limit, rho summarizes old messages and replaces them with the summary. The summary preserves goal, decisions, progress, and changed files. | `rho-core` | `planned` | A `CompactionStrategy` trait (planned) lets callers replace the summary prompt or use a different model. |
| F-63 | Branch summary | When the user navigates away from a branch, rho summarizes the abandoned branch and injects the summary at the new position. | `rho-core` | `planned` | Same `CompactionStrategy` trait as F-62. |
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
| F-80 | Minimal TUI | The user runs `rho` and sees a transcript, a streaming answer, thinking blocks, tool rows, an input editor, and a status line. | `rho-tui` | `sprint-1` | `rho-tui` is an optional crate. A third party ships a different TUI or omits TUI entirely. |
| F-81 | Pure-function render | TUI state is a pure function of events. Tests drive the renderer with a test backend, not a real terminal. | `rho-tui` | `sprint-1` | No external extension point. This is an internal design constraint. |
| F-82 | Non-blocking input | The input editor never blocks on model work. The user types while the model streams. Ctrl-C cancels the turn. A second Ctrl-C exits. | `rho-tui` | `sprint-1` | No external extension point. |
| F-83 | Themes | The user sets a color theme in the config file. The TUI applies the theme to all rendered output. | `rho-tui` | `planned` | Add a theme file to the `theme_paths` config key. No code change required. |
| F-84 | Keybindings | The user rebinds any TUI key action in the config file. | `rho-tui` | `planned` | Add or override bindings in the config file. No code change required. |
| F-85 | Custom tool renderer | A third party registers a custom render function for a named tool. Tool rows display custom content. | `rho-tui` | `planned` | Implement a `ToolRenderer` trait and register it with `rho-tui` at startup. |
| F-86 | Time to first frame | rho renders the TUI before the first token arrives. The time to first frame is measured and recorded in `docs/benchmarks.md`. | `rho-tui` | `sprint-1` | No external extension point. This is a quality gate, not a feature. |

---

## Frontends — ACP / RPC

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-90 | ACP frontend | A client connects over stdio JSON-RPC and drives rho headlessly. Any UI can embed rho without a terminal. | `rho-acp` | `planned` | `rho-acp` is an optional crate. Any process speaks the protocol. The protocol is documented. |
| F-91 | Prompt command | The client sends `{"type":"prompt","message":"..."}` and receives events until `agent_settled`. | `rho-acp` | `planned` | The protocol is the extension point. Any language can implement a client. |
| F-92 | Steer command | The client sends a steering message while the agent is running. The message delivers after the current tool calls finish. | `rho-acp` | `planned` | Same protocol extension point as F-91. |
| F-93 | Abort command | The client sends `{"type":"abort"}` and rho cancels the current turn immediately. | `rho-acp` | `planned` | Same protocol extension point as F-91. |
| F-94 | Session commands over ACP | The client creates new sessions, switches sessions, and forks sessions over the protocol. | `rho-acp` | `planned` | Same protocol extension point as F-91. |
| F-95 | Extension UI sub-protocol | An ACP client responds to `extension_ui_request` events for select, confirm, and input dialogs from hooks. | `rho-acp` | `planned` | Clients that do not implement the sub-protocol receive a default value after a timeout. |

---

## Observability and telemetry

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-100 | Structured tracing | Every agent action emits a `tracing` span with structured fields. A third party attaches any `tracing` subscriber. | `rho-core` | `planned` | Attach a `tracing::Subscriber` at startup. No rho code change required. |
| F-101 | Token and cost accounting | Every provider response records input tokens, output tokens, cache read tokens, and cost. Session totals accumulate. | `rho-core` | `planned` | A hook (F-40) fires after each response and receives the usage struct. |
| F-102 | Session statistics | The caller reads session totals: message count, token totals, cost, context usage percent. | `rho-core` | `planned` | No external extension point. The statistics are read from the session state. |
| F-103 | No secrets in logs | The credential resolution path redacts key values before passing them to `tracing`. Redaction is done by construction, not by a filter. | `rho-config` | `sprint-1` | No extension point. This is a security constraint. |

---

## Footprint and performance

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-110 | Low resident memory per session | Each session process uses significantly less resident memory than pi (76.5 MB, measured August 2026 per jcode.sh published benchmarks). The measured number for rho is `to be measured`; see `docs/benchmarks.md`. | `rho-core` | `sprint-1` | Callers compile only the crates they need. Feature flags exclude unused providers and frontends. |
| F-111 | Fast cold start | The process is ready to accept input in significantly less time than pi (596 ms, measured August 2026 per jcode.sh published benchmarks). The measured number for rho is `to be measured`; see `docs/benchmarks.md`. | `rho-cli` | `sprint-1` | No Rust runtime to start. No script engine to start. The main binary does not discover or compile extensions at startup. |
| F-112 | Release build size | The release binary uses `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, and `strip = "symbols"`. The resulting binary size is `to be measured`; see `docs/benchmarks.md`. | `rho-cli` | `sprint-1` | Callers compile only the features they need with `--no-default-features`. |
| F-113 | Separate feature flags for providers | Each provider is a cargo feature. A build with only one provider does not pay the compile time or binary size of the others. | `rho-cli` | `sprint-1` | Callers select features with `--features`. No code change required. |

---

## Distribution

| ID | Name | Outcome | Owning crate | Status | Extension point |
|----|------|---------|--------------|--------|-----------------|
| F-120 | Single binary | `cargo install rho` produces one self-contained binary with no runtime dependency. | `rho-cli` | `planned` | No extension point. The binary is the distribution unit. |
| F-121 | Library-first API | `rho-core`, `rho-tools`, and `rho-plugin` are usable as library crates without the CLI or TUI. | `rho-core` | `sprint-1` | Third parties add `rho-core` to their `Cargo.toml` and build a custom frontend. |
| F-122 | Website | `getrho.dev` explains what rho is, shows the install command, and links to docs and GitHub. | `web/` | `sprint-1` | The website is a separate Astro project. Contributors edit `web/`. |
| F-123 | CI pipeline | Every pull request runs `cargo fmt --all --check`, `cargo clippy -D warnings`, `cargo test --workspace`, and `cargo build` on Linux and macOS. | `.github/workflows/` | `sprint-1` | No extension point. CI is fixed. |
