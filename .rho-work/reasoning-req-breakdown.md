# Requirement Breakdown: SPEC-reasoning-across-providers

Format: **Stated** (your exact words from spec + committed code), **Inferred** (what the spec logically requires but doesn't state as a must), **Unknown** (choices the implementation must make or that conflict).

---

## R1 — Split the data model into a trace and a replay block

**Stated:**
- From `AGENTS.md` step 3: "The data model is a contract too." Contract first, before code.
- From spec section 1: "The data model" side is owned by `rho-core` and "must agree on the block kinds, and which one reaches a provider."
- From spec section 2 (jcode): `ContentBlock::ReasoningTrace { text }` is "history only, never sent" and `ContentBlock::AnthropicThinking { thinking, signature }` is "replay, sent back." jcode "splits the block in two," which is "the better idea."
- From spec section 4: "A trace is for a human. A replay block is for the provider."
- From task R1: `ReasoningTrace { text }` — for the reader, and it never reaches a provider; `ReasoningReplay { text, signature: Option<String> }` — echoed back when the wire asks.
- From task R1: `signature` stays `Option<String>`. An earlier draft used `""` for none, and a review named that a fail-open default.
- From task R1: Add `thought_signature: Option<String>` to `ToolCall`, so a Gemini crate needs no edit to shared code later.

**Inferred:**
- The split must be backward compatible in the persisted format (R4 handles the migration).
- Every provider's `format_messages` arm must change from `Thinking { .. }` to explicitly handle both `ReasoningTrace` (drop it) and `ReasoningReplay` (apply the wire rule from R2).
- "Never reaches a provider" means `ReasoningTrace` must be filtered before any `format_messages` call.
- The exhaustive match principle (AGENTS.md step 8) requires that no provider has a catch-all arm; every block variant must be explicitly named.

**Unknown:**
- None stated. The split is dictated by jcode's approach, which the spec endorses as "the better idea."

---

## R2 — Add the replay policy as data

**Stated:**
- From spec section 3: "The table governs the OpenAI-compatible family only, where the variation really is field names."
- From spec section 4 (the data model): "Replay is a per-endpoint property."
- From task R2: `ReasoningWire { read_fields, replay, rejects_unknown_fields, ask_to_enable }`.
- From task R2: `ReasoningReplay` with `Never`, `SignedWhileSameModel`, `TextOnToolCall`, `SignatureOnToolCall`.
- From spec section 2 (jcode): "Guessing fails in both directions: Kimi and DeepSeek **require** the replayed field, and Mistral answers 422 `Extra inputs are not permitted` without it."
- From spec section 2 (jcode): pi's rule for replay is "keyed on whether the same model answers: redacted same model (keep), redacted other model (drop), has a signature same model (keep even when text empty), empty text (drop), other model (convert to plain text)."

**Inferred:**
- `read_fields` must encode the priority order: pi's source says "the first non-empty one," and jcode issue 322 proves the order matters.
- Each `ReplayPolicy` variant (Never, SignedWhileSameModel, TextOnToolCall, SignatureOnToolCall) governs whether a `ReasoningReplay` block is serialised into the request at all.
- `rejects_unknown_fields` implies that if a provider's schema is strict (like Mistral), the `ReasoningReplay` block must be omitted entirely, not present with a `None` field.
- `ask_to_enable` is a flag per endpoint that gates whether rho sends a thinking budget or thinking request in the prompt.
- `rho-provider-openrouter` must read from the table and apply the policy; no provider-specific branching beyond reading the wire.

**Unknown:**
- What is the exact wire format for `read_fields`? (e.g., a tuple of field names, a JSON path, a function pointer?)
- How are multiple provider hosts within the OpenAI-compatible family distinguished in the table? (e.g., by model prefix, by a separate config key?)
- Should `ask_to_enable` per endpoint be statically configured, or should rho probe the endpoint's capabilities on first use?

---

## R3 — Ask Bedrock for thinking

**Stated:**
- From spec section 0, defect 1: "With Claude Haiku on Bedrock, rho never asks for extended thinking, so the model writes `<thinking>...</thinking>` inside ordinary text."
- From task R3: "`rho-provider-bedrock/src/lib.rs` currently drops reasoning in a named arm and says replay is phase 2."
- From task R3 proof: "`ask_to_enable_adds_the_thinking_request`."
- From spec section 3 "One" (reading): "Reading is per-provider code. A provider is its own crate in rho, and parsing its wire format is that crate's job."

**Inferred:**
- "Ask for extended thinking" means including a field in the request body (e.g., `"thinking": { "type": "enabled", "budget_tokens": N }`).
- The budget N is not part of this requirement; the spec says it is out of scope.
- This fix applies only to Bedrock. Other providers either already receive thinking by default or have their own request shape.
- The named arm that currently drops reasoning must be changed to build a `ReasoningReplay` block (when R2 is done) or a temporary stub, so the text is not lost.

**Unknown:**
- Should rho ask for thinking on every Bedrock call, or only when the user has not disabled reasoning display (i.e., when `ReasoningDisplay != Off`)?
- What budget value should rho request? The spec defers this, but the request body must choose a number.

---

## R4 — Make the persisted format work in both directions

**Stated:**
- From spec section 4, "The persisted format, in both directions":
  - "A new rho reads `"type": "thinking"` as `ReasoningTrace`, and it never replays a stale signature from an older session."
  - "An old rho must still load a new file, so the new variants serialise under the old `"type": "thinking"` tag with an added `replay` key."
  - "A missing `replay` key reads as `false`, which is the safe direction."
  - "A truncated final line is dropped, and the session still opens."

**Inferred:**
- The serialisation format uses `"type": "thinking"` as the tag for backward compatibility. New `ReasoningTrace` and `ReasoningReplay` variants must both deserialise from that tag.
- Distinguishing trace from replay in the old format is done by the presence of a `replay` key: if `"replay": true`, it is a `ReasoningReplay`; if missing or `false`, it is a `ReasoningTrace`.
- An old rho (before this change) sees `ReasoningReplay { text, signature: Some(_), replay: true }` and must treat it as the old `Thinking { text, signature }`, losing the replay metadata but preserving the text.
- Session file loading must be resilient to truncation, line-by-line.

**Unknown:**
- Should `ReasoningTrace` blocks written by the new rho be tagged with a new `"type": "trace"`, or stay as `"type": "thinking"` with `"replay": false`? (The spec says "serialise under the old tag," but does not say whether trace is distinguishable for future readers.)
- How is the `signature` field handled when deserialising an old block? Does it carry forward, or is it discarded because a trace should never be replayed?

---

## R5 — Keep an orphaned tool call whole

**Stated:**
- From spec section 6, rule 5: "An orphaned tool call gets a synthetic error result, so the signature chain stays whole."
- From spec section 2 (pi and jcode): pi "inserts a synthetic tool result for an orphaned tool call, and its comment says this 'preserves thinking signatures and satisfies API requirements'."
- From task R5 proof: "`an_orphaned_tool_call_gets_a_synthetic_result`."

**Inferred:**
- "Orphaned" means a `ToolCall` block with no following `ToolResult` block in the transcript.
- "Synthetic error result" means a `ToolResult` block with an error message, not a successful execution result.
- This must be inserted before `format_messages` sends the message to the provider, so the provider sees a complete tool-call/result pair.
- The error message should be generic and not leak implementation details (e.g., "Tool execution was incomplete").

**Unknown:**
- At what stage is the synthetic result injected? (e.g., in the reducer, in the provider's format_messages, or in the session builder?)
- What is the exact error message text?
- Should this only apply to tool calls that precede reasoning blocks, or to all orphaned tool calls?

---

## R6 — Stop the CLI swallowing a bad reasoning mode

**Stated:**
- From committed code (`rho-cli/src/cli.rs:428`): `resolve_reasoning` calls `.ok()` on the FromStr result, so `rho --reasoning loud` silently becomes `summary`.
- From committed code (`rho-config/tests/reasoning.rs:51`): `a_bad_reasoning_value_fails_closed` asserts that a config file sets a bad value and gets an error.
- From task R6: "The spec requires `an_unknown_reasoning_mode_is_refused`, and says 'it does not fall back in silence'. The committed code and its test `a_bad_flag_value_falls_back_to_summary` assert the opposite."
- From prior session (ask_user reply): "Refuse" means exactly that: no fallback, no warning-and-continue. rho prints an error naming the bad value and the valid modes, and exits non-zero. Nothing is drawn."

**Inferred:**
- The decision has been made (from the prior session): **refuse** mode, not fallback.
- Both `--reasoning` and `RHO_TUI_REASONING` must refuse unknown modes.
- The error message must name the bad value and list the valid modes.
- The exit code must be non-zero, indicating an error.
- The test `a_bad_flag_value_falls_back_to_summary` must be deleted or renamed to assert refusal instead.

**Unknown:**
- None. The decision is stated in the prior session.

---

## R7 — Make the config file reach the screen

**Stated:**
- From task R7: "`rho-config` parses `tui-reasoning`, validates it, and stores it in `Config::reasoning` at `rho-config/src/lib.rs:175`. Nothing ever reads that field."
- From task R7: "`resolve_reasoning` re-reads the environment and never consults the loaded config, so a config-file setting is parsed and then thrown away."
- From task R7 proof: "`the_flag_beats_the_config_and_the_environment`, plus a test that a config file alone changes the drawn mode."
- From spec section 2 (pi): "pi decides at send time," which implies configuration is read at runtime and applied to the display.

**Inferred:**
- The config precedence is: CLI flag > environment variable > config file > default.
- `resolve_reasoning` must be called with the loaded `Config` object, not just the flag and env vars.
- The config value is only read if the flag is absent and the environment variable is absent.

**Unknown:**
- None stated. The wiring is clear.

---

## R8 — Reconcile the spec's test names with the tree

**Stated:**
- From task R8: "The spec names 38 tests. 14 match the tree by name, and 9 more exist under a different name, so the spec and the code disagree about what is proven."
- From task R8 table: specific mappings (e.g., `a_mixed_case_tag_is_stripped` in spec vs `a_mixed_case_tag_opens_a_block` in tree).
- From task R8: "Pick one name per test, and make the spec and the tree agree. Add the two absent tests."

**Inferred:**
- Every test in the spec must have a corresponding test in the tree with the same name.
- If a test does not exist, it must be written.
- Renaming existing tests in the tree to match the spec is acceptable.

**Unknown:**
- Which name should be canonical—the spec's name or the tree's name? (The instruction says "make the spec and tree agree," but does not say who moves.)
- For the two absent tests (`a_stripped_tag_does_not_change_the_request` and `no_text_is_emitted_before_the_decision`), which file should contain them? (The spec does not say.)

---

## R9 — Prove the saved work catches its own defects

**Stated:**
- From task R9: "Step 7, and it was never run on any of this. The commit says so. For each of the 14 landed tests, break the implementation on purpose and watch the test fail."
- From task R9: "Copy the file to `/tmp` first. **Never restore with `git checkout`**, because it destroys every uncommitted change in that file. See `D-jcode-bash-lessons`."
- From task R9: "The suspect ones are the display tests, because a renderer test can assert a row exists while the mode logic is inverted."

**Inferred:**
- Every test must fail when the code it is testing is broken.
- A test that passes against broken code must be fixed or deleted.
- The display tests (in `rho-tui/src/render.rs`) are high-risk and need extra attention.

**Unknown:**
- How many ways should each test be broken? (e.g., invert a boolean, delete a line, change a constant?)
- Should R9 be applied to the 14 existing tests, or also to the new tests added in R1–R7?

---

## R10 — Drive it for real, then write it down

**Stated:**
- From task R10: "Step 11, and there is no `docs/verification/` file for reasoning. No fixture can catch this class of defect, because a fixture describes a response and these defects are in the **request**."
- From task R10: "Build `cargo build --release -p rho-cli`, then run a real prompt on Bedrock with a Claude model, and confirm no `<thinking>` tag reaches the screen as the answer."
- From task R10: "Run a **two-call tool loop**, because Bedrock worked for one tool call and returned 400 for two."
- From task R10: "Exercise every provider the change touches. OpenRouter and Bedrock are both in the diff."
- From task R10: "Ask the model to print a literal `<thinking>` tag, and confirm it prints."
- From task R10: "Write the commands and the real output into `docs/verification/`."

**Inferred:**
- The verification must use real providers (Bedrock, OpenRouter), not mocked responses.
- At least one test case must exercise a multi-turn tool-call loop (not just one tool call).
- The verification must prove both the fix (no thinking tag as answer) and the fallback case (a literal tag in model output does print).

**Unknown:**
- Which Claude model on Bedrock should be used? (e.g., Claude 3.5 Opus, Claude 3 Haiku?)
- How long should the tool loop be? (The spec says "two-call" but does not define what counts as one call.)
- Should the verification include providers not in the diff? (e.g., Gemini, if its reasoning reading is part of R1/R2?)

---

## R11 — Update the docs last

**Stated:**
- From task R11: "Step 13. `docs/features.md` has one reasoning mention, on the OpenRouter row, and it claims 'Reasoning tokens pass through'. Add the rows this work creates, with the owning crate and the extension point."
- From task R11: "Record R6's decision in `.rho-work/decisions/`."
- From task R11: "Delete any claim the tree cannot prove."
- From AGENTS.md step 13: "Amend the spec when the implementation diverged. The spec is the contract, so it follows the code or the code follows it. Never leave them disagreeing."

**Inferred:**
- The extension point for each feature must be named in `docs/features.md` (e.g., "Gemini reasoning is a new `rho-provider-gemini` crate").
- Any claim in the README or docs that the implementation does not support must be removed or updated.
- The decision file for R6 (refuse vs. fallback) must be written to `.rho-work/decisions/` with a date and slug.

**Unknown:**
- Which rows in `docs/features.md` should be added or updated? (e.g., one row per provider, or one row for the whole feature?)
- Should the spec itself be updated if the implementation diverges, or only the docs?

---

## Summary: What Needs a Decision

Only **R3** and **R6-ish** have open questions:

1. **R3**: Budget for Bedrock thinking request, and whether to gate on `ReasoningDisplay != Off`.
2. **R6**: Already decided (refuse). No action needed beyond implementing the refusal.
3. **R8**: Who owns the name—spec or tree? (Recommendation: spec is the contract, so rename tree to match.)
4. **R10**: Which Claude model, which providers beyond Bedrock/OpenRouter?

All other requirements are stated or clearly inferable from the spec and committed code.
