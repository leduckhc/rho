You are a Codex CLI worker delegated by a Pi controller. This is a REVIEW-ONLY job. Do not edit any file. Do not commit.

Repository: /Users/le/Work/Vibe/rho-reasoning (rho, a Rust coding-agent harness).

Read /Users/le/Work/Vibe/rho-reasoning/AGENTS.md first. It is the contract this work must satisfy, especially step 3 (spec and contract before code), step 7 (break the implementation and watch the test fail), step 8 (check the whole surface, not the diff), and step 12 (turn each defect into a guard).

Review these four commits, which are the whole change:

    git log --oneline 20c00cf~1..HEAD
    20c00cf docs(reasoning): replay an opaque owner-tagged payload
    23c4355 docs(reasoning): close two review holes, and bound the replay payload
    08a86d6 feat(reasoning): ask the model to think, and let the user set how hard
    282eb15 feat(reasoning): replay a model's reasoning, and keep a trace that never travels

What the change does:

1. `ReasoningEffort` (off/low/medium/high/xhigh) in `crates/rho-core/src/reasoning.rs`, reaching `CompletionRequest.reasoning` through `SessionConfig`. Sources: `--reasoning-effort`, `RHO_REASONING_EFFORT`, and the `reasoning-effort` config key. A bad value is refused at its own source.
2. `crates/rho-provider-bedrock/src/lib.rs` asks Claude for extended thinking through `additionalModelRequestFields`, gated by `model_supports_thinking`, which parses a version out of the model id and fails closed. A thinking request raises `max_tokens` above the budget and drops the temperature.
3. `ContentBlock::Thinking` is split into `ReasoningTrace { text }` and `ReasoningReplay { text, state }`. `ProviderState { owner: ReasoningOwner { provider, model }, value: serde_json::Value }` is an opaque payload, and `ProviderState::for_owner` is the single owner check. `ToolCall` gained the same optional payload. See `crates/rho-core/src/content.rs`.
4. The persisted format goes through a private `DiskBlock` with `#[serde(from/into)]`, because two serde variants cannot share the `thinking` tag.
5. `crates/rho-cli/src/cli.rs`: `rho run` prints the answer on stdout and reasoning on stderr, and strips a leading `<thinking>` tag on that path too.
6. `ConfigError::Value` replaces four uses of `ConfigError::Parse` that passed the literal "the merged configuration" as a file path.

Defect history of this project, so assume another of the same family is present:

- `ToolKind::Other` was a fail-open enum variant, so a read-only policy approved any tool that forgot its kind.
- A `""` signature default was named a fail-open by review.
- Three `todo!()` bodies survived a stage that reported green, one of them a security boundary.
- Three providers rejected rho's requests in sprint 1: every fixture described a response, and every defect was in the request.
- A memory-cap test passed against the very bug it was written for.
- A credential trust gate protected nothing for a whole commit, because no call site resolved through it.
- In this very change, the SDK-to-wire translation dropped every signature in a wildcard arm while all unit tests passed.

Answer these questions specifically, with file paths and line numbers, in under 900 words:

1. Correctness of `model_supports_thinking`. It lowercases the id, splits on "anthropic.claude", then takes the first two numbers under 100 as major and minor, and returns true for 3.7 or above. Find an id it gets wrong in either direction. Real Bedrock ids, inference profiles, and cross-region prefixes all count.
2. The owner check. `ProviderState::for_owner(provider, model)` compares two strings. Find a path where a payload travels without that check, or where the check passes when it should not. Note that `build_messages` (no model) still exists next to `build_messages_for_model`, and that `Provider::id()` is not what supplies the string on the write side.
3. The `DiskBlock` conversion in `crates/rho-core/src/content.rs`. Check every case: a trace with a stale signature, `replay` true with no payload, `replay` false with a payload, an unknown extra key, and a `ToolCall` payload. Say what an older rho does with each shape.
4. Fail-open review. `build_thinking_fields` returns `Option`, `replay_block` returns `Option`, `cap_state` returns `Option`, and each `None` means "send nothing". Find a case where a `None` hides a real error that a user should see.
5. Untested public surface. List every public item this change adds that no test reaches. That list is where the bugs are.
6. The guards. `every_request_builder_has_an_explicit_arm` in `crates/rho-core/tests/reasoning_replay.rs` reads source text and strips comments. `the_thinking_fields_reach_the_request` and `the_headless_loop_splits_its_text` read the production half of a file. Break each guard in your head and say whether it still trips.
7. Anything in `docs/specs/20260819-134615-SPEC-reasoning-across-providers.md` or `docs/verification/reasoning-effort.md` or `docs/verification/reasoning-replay.md` that the code does not support. An overclaim in a doc is a defect here.

Return in your final answer:
- Status: DONE, DONE_WITH_CONCERNS, BLOCKED, or FAILED
- A numbered list of findings, each with severity (blocking, major, minor), file:line, and the smallest correct fix
- The list of public items with no test
- Anything you could not check, and why
