# rho comparison with prior art

This document explains what rho takes from each source and what it deliberately drops.

---

## From pi

pi is a TypeScript coding agent. Source: `/opt/homebrew/lib/node_modules/@earendil-works/pi-coding-agent/docs/`.

| Idea | Do we take it? | rho feature ID | Why or why not |
|------|----------------|----------------|----------------|
| Extension model: tools, hooks, slash commands, themes from user-space packages | Yes, adapted | F-hook-trait-tier-1, F-lifecycle-hook-points, F-slash-commands, F-themes, F-keybindings | This is the best idea in pi. rho implements it with compiled Rust traits (Tier 1) and out-of-process JSON-RPC plugins (Tier 2) instead of TypeScript modules. |
| Skills: `SKILL.md` files discovered from directories, loaded on demand | Yes | F-skills-filesystem | Skills are language-neutral files. rho discovers them the same way. No Node runtime required. |
| Prompt templates: user-authored `.md` files in a config directory | Yes | F-prompt-templates | Plain files, no runtime dependency. Direct port. |
| Append-only session JSONL log | Yes | F-append-only-session-log, F-session-resume | Stable, inspectable, easy to tail. rho uses the same append-only model. The format itself is rho's own; see decision D-own-session-format. |
| Session branching and tree navigation | Yes | F-session-branching | Valuable for exploration. Port the concept. The record format is rho-specific, not pi's. |
| Headless mode over stdio JSON-RPC | Yes | F-acp-frontend–F-extension-ui-sub-protocol | The owner's app (`makit`) drives agents headlessly. This is the primary use case. rho names this frontend ACP, and it speaks the real Agent Client Protocol. |
| Layered config (global + project) | Yes | F-layered-config, F-environment-variable-override | Simple and familiar. rho uses TOML instead of JSON. |
| Credential resolution: env var, file, shell command, interpolation | Yes | F-credential-resolution | Shell command resolution (`!op read ...`) is a real user need. Direct port of the idea, not the code. |
| Context compaction: summarize old messages when context is near full | Yes | F-context-compaction | Context windows are finite. The trigger and summary format are directly inspired by pi. |
| Branch summarization on tree navigation | Yes | F-branch-summary | Context from abandoned branches is lost without this. |
| Provider and model registry extensible without a code change | Yes | F-model-registry, F-custom-provider-extension | Config-file model registration is simpler than recompiling for every new model. |
| Short system prompt | Yes | F-short-system-prompt | A long system prompt wastes tokens on every turn. rho adopts the same discipline. |
| Custom compaction via extension hook | Yes (planned) | F-context-compaction | Users may need a domain-specific summary. The hook is the right abstraction. |
| Node.js runtime | No | — | This is the primary cost driver. pi at ~76.5 MB per session and ~596 ms cold start (jcode.sh benchmarks, August 2026) is too expensive for 50 concurrent sessions. |
| Single bundled npm package | No | — | The single-package model makes selective use impossible. rho uses cargo features for composability. |
| TypeScript extension compilation at load time (jiti) | No | — | Compile-at-load is a startup cost. Rust compiled extensions pay nothing at runtime. Out-of-process plugins connect after the first token. |
| Built-in OAuth login flows | No | — | OAuth adds complexity. rho resolves credentials from env vars, files, and shell commands. OAuth is a plugin concern. |
| Subscription-based providers (ChatGPT Plus, Claude Pro) | No | — | rho targets API key and credential-chain auth. Subscription auth requires OAuth reverse-engineering. Not a sprint-1 need. |
| TUI custom component system (`@earendil-works/pi-tui`) | Partially | F-custom-tool-renderer | A custom renderer per tool is valuable. rho will expose a `ToolRenderer` trait. The full TUI component API is not ported. |
| Session export to HTML | No | — | Useful but not a sprint-1 need. Add as a plugin later. |
| Session sharing via GitHub gist | No | — | Out of scope. Not a sprint-1 need. |

---

## From jcode

jcode is a Rust agent harness. Published benchmarks at https://jcode.sh (sampled August 2026).

| Idea | Do we take it? | rho feature ID | Why or why not |
|------|----------------|----------------|----------------|
| Extreme resource discipline: measure PSS, time to first frame, time to first input | Yes | F-low-resident-memory-per-session, F-fast-cold-start, F-release-build-size, F-time-to-first-frame | This is the core design constraint. No number is claimed without a measurement. |
| Append-only context: stable prefix, never insert dynamic text into a sent prefix | Yes | F-stable-prefix-for-kv-cache, F-full-tool-list-at-turn-one, F-short-system-prompt | KV cache warmth is a real latency win. This rule costs nothing to follow. |
| Advertise the full tool list in the first request | Yes | F-full-tool-list-at-turn-one | Avoids a schema round-trip on every new tool. Required by the stable prefix rule. |
| Plugin schema cache: schemas available at turn one even before plugin connects | Yes (planned) | F-plugin-schema-cache | Removes plugin startup from the critical path. |
| Background tasks as first-class: list, tail, cancel, wait | Yes (planned) | F-background-tasks | `wait` with a progress checkpoint removes polling loops. Important for long commands. |
| Auto-continue on incomplete work | Yes (planned) | F-auto-continue | Reduces user babysitting. |
| Retry transient errors, stop on permanent ones | Yes | F-auto-retry | Standard practice. jcode's explicit rule is worth codifying. |
| Short system prompt (~670 tokens in jcode) | Yes | F-short-system-prompt | rho targets under 1000 tokens. |
| Todo tool with confidence scores and forced re-check | Yes (planned) | F-todo-tool | Confidence at assignment and completion catches false completions. |
| Per-session PSS reporting | Yes | F-low-resident-memory-per-session, `docs/benchmarks.md` | The measurement script (`bench/footprint.sh`) reports PSS. |
| Publish benchmark numbers | Yes | `docs/benchmarks.md` | All numbers are attributed and dated. rho does not claim a win without a test. |
| ~10.4 MB extra resident memory per session (jcode.sh, August 2026) | Reference only | F-low-resident-memory-per-session | This is jcode's number, not rho's. rho's number is `to be measured`. |
| ~49 ms time to first input (jcode.sh, August 2026) | Reference only | F-fast-cold-start | This is jcode's number, not rho's. rho's number is `to be measured`. |
| Self-modifying source mode | No | — | Out of scope for sprint 1. Too risky without a full audit of the trust model. |
| Embedded semantic memory with local embeddings | No | — | Adds a large dependency (embedding model). A plugin can provide this instead. |
| Desktop app | No | — | Not a sprint-1 need. `rho-tui` and `rho-acp` cover the UI surface. |

---

## From agentsdk.build design school

agentsdk.build is a retired project. Its design principles are documented in the project brief.

| Idea | Do we take it? | rho feature ID | Why or why not |
|------|----------------|----------------|----------------|
| Library first, application second. The SDK is the product; the CLI is a thin consumer. | Yes | F-library-first-api, F-provider-trait, F-tool-trait | `rho-core` is a pure library. `rho-cli` is thin. Third parties use `rho-core` directly. |
| Separate the harness from the compute. The agent loop must not assume it runs in the same process as the tools. | Yes | F-out-of-process-plugin-tier-2, F-plugin-schema-cache | `rho-plugin` implements out-of-process tools over stdio JSON-RPC. The loop dispatches by name, not by function pointer. |
| Typed, replaceable components rather than configuration flags. | Yes | F-provider-trait, F-tool-trait, F-hook-trait-tier-1 | `Provider`, `Tool`, and `Hook` are traits, not enum variants or config strings. Replacing a component means passing a different impl. |
| Agent loop as a composable piece, not a framework | Yes | F-agent-loop | `AgentLoop` is a struct the caller constructs. It does not call a global registry. |

## What reading jcode's source changed

The table above compares designs. This section records a concrete outcome, so the claim
that reading prior art pays is checkable rather than asserted.

Reading jcode's `edit` tool gave rho three features it lacked: `replace_all`, near-miss
diagnostics on a failed match, and familiar argument aliases. It also showed one defect
to avoid: an empty `old_string` with `replace_all` rewrites the whole file, because an
empty pattern matches at every character boundary.

See `docs/specs/20260817-164906-SPEC-tool-interface.md` section 6a, and decision D-jcode-edit-lessons.

---

## What reading pi's source changed

The spec said pi "spawns a separate `pi` process" per child. That claim was wrong.

Reading the source proved it. `agent-runner.ts` line 921 calls `createAgentSession` inside
`runInChildSessionContext`. `child-context.ts` lines 1–15 show that is an `AsyncLocalStorage`
flag, not a process boundary. `agent-manager.ts` line 237 defines `spawn` as a method on
a manager class. It is not an OS spawn. `agent-manager.ts` line 441 stores the result in
`record.result`. The result travels through a struct field, not a wire. `nested-tools.ts`
line 189 checks depth with an integer compare. The only `child_process` import in the whole
extension is `worktree.ts`, which runs git.

pi chose the same in-process architecture rho chose. The original spec used the false claim
to argue a structural advantage that does not exist. The real advantage is measured density:
50 live sessions in 24.98 MiB, about 247 KB each (`docs/benchmarks.md`). That number stands.

The lesson: the spec argued from a remembered design. Reading the actual source took minutes
and found the error. AGENTS.md step 1 says to read the real source, not your memory of it.
This section is the proof that step 1 pays.
