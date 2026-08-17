# rho comparison with prior art

This document explains what rho takes from each source and what it deliberately drops.

---

## From pi

pi is a TypeScript coding agent. Source: `/opt/homebrew/lib/node_modules/@earendil-works/pi-coding-agent/docs/`.

| Idea | Do we take it? | rho feature ID | Why or why not |
|------|----------------|----------------|----------------|
| Extension model: tools, hooks, slash commands, themes from user-space packages | Yes, adapted | F-40, F-41, F-44, F-83, F-84 | This is the best idea in pi. rho implements it with compiled Rust traits (Tier 1) and out-of-process JSON-RPC plugins (Tier 2) instead of TypeScript modules. |
| Skills: `SKILL.md` files discovered from directories, loaded on demand | Yes | F-45 | Skills are language-neutral files. rho discovers them the same way. No Node runtime required. |
| Prompt templates: user-authored `.md` files in a config directory | Yes | F-46 | Plain files, no runtime dependency. Direct port. |
| Append-only session JSONL log | Yes | F-50, F-51 | Stable, inspectable, easy to tail. rho uses the same append-only model. The format itself is rho's own; see decision D-001. |
| Session branching and tree navigation | Yes | F-52 | Valuable for exploration. Port the concept. The record format is rho-specific, not pi's. |
| Headless mode over stdio JSON-RPC | Yes | F-90–F-95 | The owner's app (`makit`) drives agents headlessly. This is the primary use case. rho names this frontend ACP, and it speaks the real Agent Client Protocol. |
| Layered config (global + project) | Yes | F-70, F-71 | Simple and familiar. rho uses TOML instead of JSON. |
| Credential resolution: env var, file, shell command, interpolation | Yes | F-72 | Shell command resolution (`!op read ...`) is a real user need. Direct port of the idea, not the code. |
| Context compaction: summarize old messages when context is near full | Yes | F-62 | Context windows are finite. The trigger and summary format are directly inspired by pi. |
| Branch summarization on tree navigation | Yes | F-63 | Context from abandoned branches is lost without this. |
| Provider and model registry extensible without a code change | Yes | F-14, F-15 | Config-file model registration is simpler than recompiling for every new model. |
| Short system prompt | Yes | F-64 | A long system prompt wastes tokens on every turn. rho adopts the same discipline. |
| Custom compaction via extension hook | Yes (planned) | F-62 | Users may need a domain-specific summary. The hook is the right abstraction. |
| Node.js runtime | No | — | This is the primary cost driver. pi at ~76.5 MB per session and ~596 ms cold start (jcode.sh benchmarks, August 2026) is too expensive for 50 concurrent sessions. |
| Single bundled npm package | No | — | The single-package model makes selective use impossible. rho uses cargo features for composability. |
| TypeScript extension compilation at load time (jiti) | No | — | Compile-at-load is a startup cost. Rust compiled extensions pay nothing at runtime. Out-of-process plugins connect after the first token. |
| Built-in OAuth login flows | No | — | OAuth adds complexity. rho resolves credentials from env vars, files, and shell commands. OAuth is a plugin concern. |
| Subscription-based providers (ChatGPT Plus, Claude Pro) | No | — | rho targets API key and credential-chain auth. Subscription auth requires OAuth reverse-engineering. Not a sprint-1 need. |
| TUI custom component system (`@earendil-works/pi-tui`) | Partially | F-85 | A custom renderer per tool is valuable. rho will expose a `ToolRenderer` trait. The full TUI component API is not ported. |
| Session export to HTML | No | — | Useful but not a sprint-1 need. Add as a plugin later. |
| Session sharing via GitHub gist | No | — | Out of scope. Not a sprint-1 need. |

---

## From jcode

jcode is a Rust agent harness. Published benchmarks at https://jcode.sh (sampled August 2026).

| Idea | Do we take it? | rho feature ID | Why or why not |
|------|----------------|----------------|----------------|
| Extreme resource discipline: measure PSS, time to first frame, time to first input | Yes | F-110, F-111, F-112, F-86 | This is the core design constraint. No number is claimed without a measurement. |
| Append-only context: stable prefix, never insert dynamic text into a sent prefix | Yes | F-60, F-61, F-64 | KV cache warmth is a real latency win. This rule costs nothing to follow. |
| Advertise the full tool list in the first request | Yes | F-61 | Avoids a schema round-trip on every new tool. Required by the stable prefix rule. |
| Plugin schema cache: schemas available at turn one even before plugin connects | Yes (planned) | F-43 | Removes plugin startup from the critical path. |
| Background tasks as first-class: list, tail, cancel, wait | Yes (planned) | F-07 | `wait` with a progress checkpoint removes polling loops. Important for long commands. |
| Auto-continue on incomplete work | Yes (planned) | F-06 | Reduces user babysitting. |
| Retry transient errors, stop on permanent ones | Yes | F-05 | Standard practice. jcode's explicit rule is worth codifying. |
| Short system prompt (~670 tokens in jcode) | Yes | F-64 | rho targets under 1000 tokens. |
| Todo tool with confidence scores and forced re-check | Yes (planned) | F-30 | Confidence at assignment and completion catches false completions. |
| Per-session PSS reporting | Yes | F-110, `docs/benchmarks.md` | The measurement script (`bench/footprint.sh`) reports PSS. |
| Publish benchmark numbers | Yes | `docs/benchmarks.md` | All numbers are attributed and dated. rho does not claim a win without a test. |
| ~10.4 MB extra resident memory per session (jcode.sh, August 2026) | Reference only | F-110 | This is jcode's number, not rho's. rho's number is `to be measured`. |
| ~49 ms time to first input (jcode.sh, August 2026) | Reference only | F-111 | This is jcode's number, not rho's. rho's number is `to be measured`. |
| Self-modifying source mode | No | — | Out of scope for sprint 1. Too risky without a full audit of the trust model. |
| Embedded semantic memory with local embeddings | No | — | Adds a large dependency (embedding model). A plugin can provide this instead. |
| Desktop app | No | — | Not a sprint-1 need. `rho-tui` and `rho-acp` cover the UI surface. |

---

## From agentsdk.build design school

agentsdk.build is a retired project. Its design principles are documented in the project brief.

| Idea | Do we take it? | rho feature ID | Why or why not |
|------|----------------|----------------|----------------|
| Library first, application second. The SDK is the product; the CLI is a thin consumer. | Yes | F-121, F-10, F-20 | `rho-core` is a pure library. `rho-cli` is thin. Third parties use `rho-core` directly. |
| Separate the harness from the compute. The agent loop must not assume it runs in the same process as the tools. | Yes | F-42, F-43 | `rho-plugin` implements out-of-process tools over stdio JSON-RPC. The loop dispatches by name, not by function pointer. |
| Typed, replaceable components rather than configuration flags. | Yes | F-10, F-20, F-40 | `Provider`, `Tool`, and `Hook` are traits, not enum variants or config strings. Replacing a component means passing a different impl. |
| Agent loop as a composable piece, not a framework | Yes | F-01 | `AgentLoop` is a struct the caller constructs. It does not call a global registry. |

## What reading jcode's source changed

The table above compares designs. This section records a concrete outcome, so the claim
that reading prior art pays is checkable rather than asserted.

Reading jcode's `edit` tool gave rho three features it lacked: `replace_all`, near-miss
diagnostics on a failed match, and familiar argument aliases. It also showed one defect
to avoid: an empty `old_string` with `replace_all` rewrites the whole file, because an
empty pattern matches at every character boundary.

See `docs/specs/SPEC-03-tool-interface.md` section 6a, and decision D-027.
