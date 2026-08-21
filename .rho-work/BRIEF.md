# rho — shared subagent brief

Read this file first. Then read **`AGENTS.md`**, which holds the development flow as a
checklist. Then read `agentic-workflow.yaml`, which holds your stage kind, its inputs, and
the exact slots you must return. Then do only your assigned stage.

`AGENTS.md` is the short version of everything below, and every step in it exists
because skipping it cost this project a real defect. Steps 5, 7, 11, and 13 are the ones
agents skip most: write the test first, prove the test catches the bug, drive the
feature for real, and reconcile the docs with the code.

Repo: `~/Work/Vibe/rho` — GitHub `leduckhc/rho` — site `getrho.dev` — MIT.

---

## 1. What rho is

A composable coding agent harness, written in Rust.

One sentence: **rho is the harness, unbundled.** The agent loop, the model
providers, the tools, and the user interfaces are separate crates. You take the
parts you want. You replace any part with your own crate, without forking rho.

## 2. Why it exists

The trigger is a measured cost problem. Existing harnesses cost a lot per
session:

| Harness | Extra resident memory per extra session | Time to first input |
| --- | --- | --- |
| jcode (Rust) | ~10.4 MB | ~49 ms |
| Codex CLI | ~21.6 MB | ~906 ms |
| pi (TypeScript) | ~76.5 MB | ~596 ms |
| Claude Code | ~213 MB | ~3513 ms |
| OpenCode | ~318 MB | — |

Source: <https://jcode.sh> published benchmarks, sampled August 2026.

The owner runs a multi-session app (`makit`) that spawns one agent process per
session. At 50 concurrent sessions a 100 MB per-session cost is fatal. So:

**Speed and memory are product features, not optimisations.** Never claim a
performance win without a measurement and the command that produced it.

## 3. What we take from prior art

### From pi (`/opt/homebrew/lib/node_modules/@earendil-works/pi-coding-agent`)

Read its docs before you design anything in the same area. Good parts to keep:

- `docs/extensions.md` — extension model: tools, hooks, slash commands, themes,
  all from user-space packages. This is the single best idea to carry over.
- `docs/skills.md` — filesystem skills discovered from directories, loaded on
  demand, not compiled in.
- `docs/prompt-templates.md` — user-authored prompt templates.
- `docs/sessions.md`, `docs/session-format.md` — append-only session log on disk.
- `docs/rpc.md` — a headless RPC mode, so any UI can drive the agent.
- `docs/custom-provider.md`, `docs/providers.md`, `docs/models.md` — provider and
  model registry that a user can extend without a code change.
- `docs/compaction.md` — context compaction strategy.
- `docs/themes.md`, `docs/keybindings.md`, `docs/tui.md` — TUI extensibility.
- `docs/settings.md`, `docs/environment-variables.md` — layered config.

What we do **not** copy from pi: a Node runtime, one large bundled package, and
a ~600 ms cold start.

### From jcode (<https://jcode.sh>)

- Extreme resource discipline. Measure per-session PSS, time to first frame,
  time to first input. Publish the numbers.
- **Append-only context engineering.** Keep the prompt prefix stable so the
  provider KV cache stays warm. Never insert dynamic text into an already-sent
  prefix. Advertise the full tool list in the first request.
- **Plugin schemas from an on-disk cache**, so out-of-process tools are
  advertised at turn one and never invalidate the cache when they connect late.
- **Background tasks as first-class**: a long command becomes a task the agent
  can list, tail, cancel, or wait on. `wait` blocks until a progress checkpoint,
  so the agent never writes `sleep` polling loops.
- **Auto-continue on incomplete work.** When a turn ends with open todos, poke
  the model back to work. Retry transient network errors, stop on permanent ones.
- A short system prompt. jcode's is ~670 tokens. Do not write an operating
  manual into the system prompt.
- Todo tool that records confidence at assignment and at completion, and forces
  a re-check when confidence jumps suspiciously.

What we consider too much for sprint 1: self-modifying source mode, embedded
semantic memory with local embeddings, a desktop app.

### From agentsdk.build (retired) and the same design school

- Library first, application second. The SDK is the product; the CLI is a thin
  consumer of it.
- Separate the harness from the compute. The agent loop must not assume it runs
  in the same process as the tools it calls.
- Typed, replaceable components rather than configuration flags.

## 4. Architecture decisions already made

These are settled. Do not re-open them. Record any disagreement as a note in
your report instead.

1. **Library-first core, thin frontends.** `rho-core` is a pure library with no
   HTTP and no terminal dependency. `rho-tui`, `rho-acp`, and `rho-cli` are thin
   and optional.
2. **Plugin mechanism, two tiers.**
   - Tier 1: in-tree Rust traits (`Provider`, `Tool`, `Hook`) for compiled
     extensions. Zero overhead.
   - Tier 2: out-of-process plugins over stdio JSON-RPC, any language.
   - **No WASM in sprint 1.** Record the reason in `ADR-plugin-mechanism`.
3. **Providers for sprint 1, in priority order**: OpenRouter, AWS Bedrock,
   Azure OpenAI. Each is its own crate behind a cargo feature.
   - OpenRouter: `POST /api/v1/chat/completions`, SSE streaming, tool calls,
     pass `reasoning` through.
   - Bedrock: `Converse` and `ConverseStream`, SigV4 from the standard AWS
     credential chain (env, profile, SSO cache, IMDS).
   - Azure: Azure OpenAI `/responses` API. Two auth modes: API key, and Entra
     token with audience **`https://cognitiveservices.azure.com/`**. That exact
     audience string is a known real-world bug source. Pin it in a test.
4. **Edition 2024, resolver 3, Rust stable.** Release profile uses `lto = "fat"`,
   `codegen-units = 1`, `panic = "abort"`, `strip = "symbols"`.
5. **Website**: static Astro site in `web/`, deployed to Cloudflare Pages.
6. Crates were created with `cargo new`. Add dependencies with `cargo add`.
   **Never hand-write a dependency line or invent a version number.**

## 5. Non-negotiable rules

- **This tree has more than one writer.** Sibling stage agents work here, and a separate
  agent keeps the repository private before release. That agent edits `AGENTS.md`,
  `docs/release-checklist.md`, `docs/index.md`, and `.github/workflows/` without
  committing. So prefer a small anchored edit over a whole-section rewrite, and never
  assume a file is as you left it. See decision D-shared-working-tree.

- **TDD.** A failing test lands before production logic. Red, green, refactor.
  If you are a `tester`, you must leave the suite red. If you are a
  `developer`, you must not edit a test to make it pass. If a test is wrong,
  say so in your report and stop.
- **SOLID.** If you cannot explain why your change respects each of the five
  principles, it probably violates one.
- **Never leave a verified bug unfixed.** Fix a confirmed bug even outside your
  task. If the fix is unsafe or too large now, say so explicitly. Never move on
  in silence.
- **Prose is ASD-STE100 Simplified Technical English.** Active voice. Simple
  tenses. One instruction per sentence. Sentences of 20 words or fewer. One
  word per meaning. No idioms. This applies to docs, comments, error messages,
  commit messages, and website copy. Code identifiers and commands stay verbatim.
- **No network in tests.** Use `wiremock` or a recorded fixture.
- **No secret in a log**, including at `trace` level. Redact by construction,
  not by a filter at the end.
- **Conventional commits.** Commit as you go. Small commits.
- Gate commands must pass before you report done. `AGENTS.md` `## Gate` lists them, and
  `agentic-workflow.yaml` `gate_sets.full` holds the same list. Read one of those two, and
  never a copy. Four files once defined this gate, and three were wrong.

## 6. Skills you may use

Skills are procedures on disk. Read the `SKILL.md` before you use one. Resolve
any relative path inside a skill against that skill's own directory.

There is no Rust-specific skill in this environment yet. If you invent a
repeatable Rust procedure for this repo, say so in your report so the controller
can save it as a project skill.

### Everyone

| Skill | Path | Use for |
| --- | --- | --- |
| `caveman` | `/Users/le/.agents/skills/caveman/SKILL.md` | Compress your own report. Keep technical accuracy. |
| `caveman-commit` | `/Users/le/.agents/skills/caveman-commit/SKILL.md` | Conventional commit messages, subject ≤50 chars. |
| `find-skills` | `/Users/le/.agents/skills/find-skills/SKILL.md` | Look for a skill you suspect exists. |

### architect

| Skill | Path | Use for |
| --- | --- | --- |
| `diagrams-as-code` | `/Users/le/.agents/skills/diagrams-as-code/SKILL.md` | Crate graph and data-flow diagrams as Python source, rendered to SVG. |
| `spike-agent-protocol-raw-jsonrpc` | `/Users/le/.pi/agent/projects-memory/makit/skills/spike-agent-protocol-raw-jsonrpc/SKILL.md` | Settle a wire-protocol question by driving the real binary over stdio JSON-RPC, before writing the spec. Use for the ACP frontend and the plugin protocol. |
| `firecrawl-scrape` | `/Users/le/.agents/skills/firecrawl-scrape/SKILL.md` | Pull an exact API reference page (Bedrock Converse, Azure Responses, OpenRouter) instead of guessing field names. |
| `firecrawl-deep-research` | `/Users/le/.agents/skills/firecrawl-deep-research/SKILL.md` | Only when a design question genuinely needs a cited multi-source report. |
| `ask-user` | `/Users/le/.pi/agent/npm/node_modules/pi-ask-user/skills/ask-user/SKILL.md` | Do **not** use. Report the open question to the controller instead. |

### documenter

| Skill | Path | Use for |
| --- | --- | --- |
| `diagrams-as-code` | `/Users/le/.agents/skills/diagrams-as-code/SKILL.md` | Architecture diagrams in `docs/`. |
| `firecrawl-scrape` | `/Users/le/.agents/skills/firecrawl-scrape/SKILL.md` | Quote a competitor's own published claim accurately. |
| `firecrawl-search` | `/Users/le/.agents/skills/firecrawl-search/SKILL.md` | Verify a factual claim before you write it. |
| `write-pull-request` | `/Users/le/.agents/skills/write-pull-request/SKILL.md` | PR titles and bodies. Also a good model for plain-English framing. |

### tester and qa

| Skill | Path | Use for |
| --- | --- | --- |
| `verify-by-driving` | `/Users/le/.agents/skills/verify-by-driving/SKILL.md` | **Primary skill for rho QA.** rho has no browser surface. Isolate state, then drive every command against a live instance. Record exit codes. Probe the failure paths. Turn each bug into a bounded regression test. |
| `acp-session-fixture-recorder` | `/Users/le/.pi/agent/projects-memory/makit/skills/acp-session-fixture-recorder/SKILL.md` | Record a real agent session to a JSON fixture for deterministic replay. Adapt the same idea to record provider SSE streams into test fixtures. |
| `spike-agent-protocol-raw-jsonrpc` | `/Users/le/.pi/agent/projects-memory/makit/skills/spike-agent-protocol-raw-jsonrpc/SKILL.md` | Capture real wire bytes before you assert on them. |
| `dart-add-unit-test` | `/Users/le/.worktrees/makit/feat-rho/.agents/skills/dart-add-unit-test/SKILL.md` | Language is wrong, but the **test organisation and naming discipline** transfers. Read only for structure. |

### developer

| Skill | Path | Use for |
| --- | --- | --- |
| `verify-by-driving` | `/Users/le/.agents/skills/verify-by-driving/SKILL.md` | Prove your change works by running it, not by reading it. |
| `firecrawl-scrape` | `/Users/le/.agents/skills/firecrawl-scrape/SKILL.md` | Read the exact provider API page. Never guess a JSON field name. |
| `caveman-commit` | `/Users/le/.agents/skills/caveman-commit/SKILL.md` | Commit messages. |

### reviewer

| Skill | Path | Use for |
| --- | --- | --- |
| `code-review-with-checklist` | `/Users/le/.agents/skills/engineering/code-review-with-checklist/SKILL.md` | 300+ checks across security, architecture, performance, testing. Primary review skill. |
| `caveman-review` | `/Users/le/.agents/skills/caveman-review/SKILL.md` | One line per finding: location, problem, fix. Use this output format. |
| `coderabbit-code-review` | `/Users/le/.agents/skills/coderabbit-code-review/SKILL.md` | Second opinion on a diff. |
| `open-code-review` | `/Users/le/.agents/skills/open-code-review/SKILL.md` | Third opinion via the `ocr` CLI, when a stage is high risk. |
| `macroscope-codereview` | `/Users/le/.agents/skills/macroscope-codereview/SKILL.md` | Branch-level review. |
| `pr-walkthrough` | `/Users/le/.agents/skills/pr-walkthrough/SKILL.md` | When the controller asks for an explainer, not just findings. |

### secops

| Skill | Path | Use for |
| --- | --- | --- |
| `code-review-with-checklist` | `/Users/le/.agents/skills/engineering/code-review-with-checklist/SKILL.md` | Security section of the checklist. |
| `verify-by-driving` | `/Users/le/.agents/skills/verify-by-driving/SKILL.md` | Prove a sandbox boundary holds by trying to escape it. |

### designer and website

| Skill | Path | Use for |
| --- | --- | --- |
| `design-brainstorm` | `/Users/le/.agents/skills/design-brainstorm/SKILL.md` | Generate several distinct directions and view them side by side. |
| `design-to-code` | `/Users/le/.agents/skills/design-to-code/SKILL.md` | Turn the chosen direction into real front-end code with an a11y and contrast audit. |
| `design-review` | `/Users/le/.agents/skills/design-review/SKILL.md` | Grade the built site. Detect AI-slop design. |
| `firecrawl-website-design-clone` | `/Users/le/.agents/skills/firecrawl-website-design-clone/SKILL.md` | Extract a design system from a reference site into `DESIGN.md`. |
| `agent-browser` | `/Users/le/.agents/skills/agent-browser/SKILL.md` | Drive a real browser. Screenshot. Verify the built site. |
| `qa-verify` | `/Users/le/.agents/skills/qa-verify/SKILL.md` | Browser QA of the site before you report done. |
| `firecrawl-seo-audit` | `/Users/le/.agents/skills/firecrawl-seo-audit/SKILL.md` | Metadata and heading review for `getrho.dev`. |

### devops

| Skill | Path | Use for |
| --- | --- | --- |
| `verify-by-driving` | `/Users/le/.agents/skills/verify-by-driving/SKILL.md` | Prove the benchmark script and the release build work. |
| `code-review-with-checklist` | `/Users/le/.agents/skills/engineering/code-review-with-checklist/SKILL.md` | CI and supply-chain checks. |

## 7. How you report back

End your run with exactly these sections. Keep it under 400 words.

```
## Stage
<stage id and name>

## DoD
<one line per DoD item from your stage in the track: MET or NOT MET, plus proof>

## Gate commands
<command, then exit code, then the last relevant line of output>

## Files
<paths you created or changed>

## Findings
<bugs you found, including ones outside your task. severity: blocker | major | minor>

## Open questions for the controller
<facts you needed and could not obtain>
```

Do not claim a DoD item is met without proof. The controller re-runs every gate
command and reads the diff. A false claim fails the stage.
