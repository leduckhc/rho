# Expansion — reasoning across providers

Written 20260821 UTC, after reading pi, jcode, and fx as source. Branch
`feat/reasoning-across-providers`. Spec:
`docs/specs/20260819-134615-SPEC-reasoning-across-providers.md`.

This replaces the framing in `.rho-work/i15-credential-expansion.md`. The config work is a
side quest. The task is reasoning. Ids here are local to this file.

## TASK

Parse the reasoning tokens every provider sends, keep them in the message history, replay
them when the wire demands it, and draw them for the user on every frontend.

## STATED

- **S1.** "the name of this branch is reasoning across providers". *Your words.* The config
  and credential work is not the task.
- **S2.** "correctly parsing the reasoning/thinking tokens from llm providers". *Your words.*
- **S3.** "correctly handle them in rho (including message history, cli visualization of
  thinking, ...)". *Your words.* Three named surfaces: the wire, the history, and the CLI
  display. The "..." means the list is not closed, so replay and persistence count too.
- **S4.** "check how pi, jcode, and fx does it". *Your words.* All three are now read as
  source, and section "What the three do" below records what changed because of it.
- **S5.** "if the argument value is wrong, throw error". *Your earlier words.* Shipped in
  `2f807c4`, for the flag, the variable, and the file.

## What the three do, and what it changes here

Read as source, not recalled. Each row names a file.

| Question | pi | jcode | fx |
| --- | --- | --- | --- |
| Blocks | one `ThinkingContent` with `thinkingSignature` and `redacted` | **four** variants | none; reasoning is not stored |
| Replay | decided at send time, keyed on the same model | per provider, always when the block exists | an opaque `provider_state_json`, plus `include: ["reasoning.encrypted_content"]` |
| Strict host | not handled | Mistral detected by profile id or `mistral.ai` in the base URL | not handled |
| Orphan call | synthetic result `"No result provided"`, `isError` | `"[Session interrupted before tool execution completed]"`, and a warning about a race | none |
| Display | one flag, `hideThinkingBlock` | `Off`, `Full`, `Current` | a "thinking" status row only; the text is captured and dropped |
| Ask to think | `thinking: {type: enabled, budget_tokens}`, or `adaptive` with `output_config.effort` | `Adaptive` or `Enabled{budget_tokens}`, gated on `show_thinking` | `reasoning: {effort, summary: "auto"}`, gated on a model capability |

Four findings that the spec does not hold today:

1. **jcode carries a fourth block for OpenAI Responses:**
   `OpenAIReasoning { id, summary, encrypted_content, status }`
   (`jcode/crates/jcode-message-types/src/lib.rs:141`). rho's specced
   `ReasoningReplay { text, signature }` cannot carry an id or an encrypted blob. rho already
   **reads** that shape in `rho-provider-azure` (`lib.rs:418`,
   `response.reasoning_summary_text.delta`), so rho can read a thing it cannot replay.
2. **pi marks a redacted block with a `redacted` flag**
   (`pi-ai/dist/types.d.ts:251`). rho's spec has behaviour rule 6 about a redacted block, and
   no field to carry it.
3. **fx offers a third design**: one opaque provider-state blob per message, and no typed
   variant at all (`fx-src/src/core/shared/types.zig:909`). It costs nothing to add a host,
   and it gives up every compile-time check.
4. **jcode names the hazard in the orphan repair**: the repair writes a placeholder, the real
   result then lands, and "the conversation is permanently unsendable"
   (`jcode-provider-anthropic/src/lib.rs:28`).

## INFERRED

Mine, with a reason each. This is the first place to look for a wrong assumption.

### What the tree really has, and what that forces

- **I1.** Reading is mostly done, so the work left is the write path — because OpenRouter
  already reads three names in order (`lib.rs:294`), Bedrock reads `reasoningContent`
  (`lib.rs:189`), and Azure reads the summary deltas (`lib.rs:418`).
- **I2.** The headline defect is still open — because no Bedrock request asks for thinking. A
  grep for `additionalModelRequestFields` in `rho-provider-bedrock` finds nothing, so Claude
  still writes `<thinking>` into ordinary text. This is spec R3, and it is the first fix.
- **I3.** `rho run` still prints a tag as the answer — because `rho-cli/src/cli.rs:385`
  handles `TextDelta` only, and the splitter is wired into `rho-tui/src/state.rs:259` alone.
  Your words in S3 name the CLI, so a TUI-only fix does not close it.
- **I4.** `rho-acp` shows no reasoning at all — because it handles no thinking event. A
  frontend is a side of this contract, and it was left out.
- **I5.** Azure keeps two `_ => {}` arms (`lib.rs:472` and `600`) — because sprint 1 put
  replay out of scope. The spec forbids a catch-all, so the spec and the code disagree today.
- **I6.** The same-model rule cannot be built as written — because `Message` is
  `{ role, content }` (`rho-core/src/content.rs`) and carries no provider or model. pi keys
  its rule on `provider`, `api`, and `model` per assistant message. Without provenance,
  `SignedWhileSameModel` has nothing to compare.
- **I7.** The orphan repair is half built — because `session/mod.rs:846` repairs a **trailing**
  `ToolCall` at resume only. A mid-transcript orphan, and a turn cancelled while a call is
  open, are not covered.
- **I8.** The repair must run on a snapshot at request build, and never rewrite the stored
  transcript — because of jcode's race in finding 4 above.

### The rules the work must keep

- **I9.** A reasoning delta is sliced only on a character boundary — because jcode crashed
  three times there (issues 632, 633, 635), and a panic kills the session.
- **I10.** A new endpoint row is selected by the base URL or a profile id, not by a model name
  — because jcode detects Mistral by `mistral.ai` in the base URL
  (`openrouter-runtime/src/lib.rs:1472`).
- **I11.** The thinking request is gated on a model capability, and it fails closed — because
  fx sends `reasoning` only when the catalogue lists the effort
  (`model_capabilities.zig:95`), and an unsupported field is a 400 for the whole turn.
- **I12.** A repository file may not raise the thinking budget — because fx refuses `effort`
  and `model` in project config, so a clone cannot spend your money. rho has the trust gate
  for this already.
- **I13.** The reasoning text is stored in every display mode — because the spec says a user
  who switches to `full` mid-session must not find the earlier reasoning empty.

### Proof and paperwork

- **I14.** Step 7 runs on the 14 rescued tests before any new work lands — because commit
  `e08b190` vouched for none of them, and two tests in the config work turned out worthless.
- **I15.** Step 11 drives Bedrock with a **two-call** tool loop — because one call passed and
  two returned 400 in sprint 1, and no fixture catches a request-side defect.
- **I16.** The spec is amended where the tree won — because `thinking.rs` says it strips a
  leading tag "either way", while spec section 6 says a turn with structured reasoning keeps
  its tags. One of them must move.

## UNKNOWN

- **U1. The replay carrier for an OpenAI-shaped provider.** **Answered: (b).** A replay block
  carries one opaque, owner-tagged provider-state value. The provider crate that wrote it is
  the only reader. See `D-reasoning-replay-is-opaque-provider-state`, which also lists the five
  guards that stop an opaque value failing open.

- **U2. Model provenance in the transcript.** **Narrowed by U1.** The owner tag on the block
  names the provider and the model, so the same-model rule reads the tag and `Message` gains
  no field. What is left is smaller: does a model switch inside one session need to be visible
  in the transcript for any other reason? Pick one: **(a)** no, the block tag is enough;
  **(b)** yes, add the pair to `Message` as well, for a reader and for a resume.
  *Default if you stay quiet: (a). It keeps the persisted `Message` shape unchanged.*

- **U3. Where the tag rule runs, and what the provider then gets.** Pick one: **(a)** each
  frontend runs the splitter, so the request never changes, and `rho-cli` and `rho-acp` gain
  it; **(b)** the core runs it and stores a reasoning block, plus the original text for the
  wire, which stores the bytes twice; **(c)** the core runs it and the reclassified text stops
  being sent, which breaks the spec's promise that stripping never changes a request.
  *Default if you stay quiet: (a). It closes S3 for the CLI, and it keeps the request
  untouched. The cost is one splitter call per frontend, so a CI guard must name every
  frontend.*

## Out of scope

- The thinking budget per model. `ask_to_enable` says whether to ask, and how many tokens is
  its own measurement.
- A Gemini or Mistral crate. The `SignatureOnToolCall` policy and the `thought_signature`
  field exist so that crate needs no shared edit.
- Token accounting for reasoning. fx counts `reasoning_tokens` per model
  (`session_usage.zig:230`), and rho's footer work is separate.
- The config side quest: I15 of `.rho-work/reasoning-task.md`, and its U2 and U4.
