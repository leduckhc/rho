# SPEC-reasoning-across-providers — one reasoning contract, and a table instead of branches

Status: draft, for review before any implementation.
Prior art: pi, jcode, and fx, all three read as source. See `docs/comparison.md`.

Amended 20260821. The replay payload is now one opaque, owner-tagged provider state. See
decision `D-reasoning-replay-is-opaque-provider-state`. Section 4 holds the change.

## 0. The defects this fixes

Each one is measured, and each has a named test in section 8.

1. **rho shows `<thinking>` tags as the answer.** With Claude Haiku on Bedrock, rho never
   asks for extended thinking, so the model writes `<thinking>...</thinking>` inside ordinary
   text. rho draws that text as the answer, because to rho it is the answer.
2. **rho reads one field name.** `rho-provider-openrouter` reads `delta.reasoning` only. A
   model that uses `reasoning_content` or `reasoning_text` produces **no** reasoning in rho.
   Measured against the same JSON: rho saw an empty string where pi saw the text.
3. **rho drops a reasoning block when it builds a request.** The match arm is `_ => {}`. That
   is silent, and `AGENTS.md` calls a silent drop a defect. It is harmless only while rho
   never asks for thinking. It breaks a tool loop the moment rho does.
4. **rho has no way to show the reasoning text.** The renderer holds
   `Row::Thinking { text: _ }` and draws only `∴ thought for 2.4s`.

## 1. The sides

| Side | Owner | Must agree on |
| --- | --- | --- |
| The wire, read | each provider crate | which field names carry reasoning |
| The wire, write | each provider crate | whether the endpoint wants it echoed back |
| The data model | `rho-core` | the block kinds, and which one reaches a provider |
| The persisted transcript | `rho-core` | what a session file holds, and what an old reader does |
| The screen | `rho-tui` | how reasoning draws, and how the user turns it off |
| The configuration | `rho-config`, `rho-cli` | the display mode, and the thinking budget |

Contract kinds touched: the data model, the wire format, the persisted format, the
configuration, and the behaviour rules.

## 2. What pi does, what jcode does, and what fx does

All three were read as source, not recalled.

**pi** keeps one `thinking` block with a `thinkingSignature`, and decides at send time in
`transform-messages.js`. Its rule is keyed on whether the same model answers:

| Case | pi |
| --- | --- |
| redacted, same model | keep |
| redacted, other model | drop, because it is opaque and would error |
| has a signature, same model | keep, even when the text is empty |
| empty text | drop |
| other model | convert to plain text |

pi reads three field names in order, `reasoning_content`, `reasoning`, then
`reasoning_text`, and it takes **the first non-empty one**. Its comment names the reason:
one host returns two fields with the same content, so a naive reader doubles the text.

pi also inserts a synthetic tool result for an orphaned tool call, and its comment says this
"preserves thinking signatures and satisfies API requirements".

**jcode** splits the block in two, and this is the better idea:

```rust
ContentBlock::ReasoningTrace { text }                    // history only, never sent
ContentBlock::AnthropicThinking { thinking, signature }  // replay, sent back
ContentBlock::Reasoning { text }                         // replay, other providers
```

A trace is for a human. A replay block is for the provider. `push_reasoning_blocks` writes
the trace only when the replay block did not already capture the same readable text, so the
transcript never holds it twice.

jcode also proves that replay is a **per-endpoint** property, and that guessing fails in
both directions:

| Endpoint | Rule | jcode issue |
| --- | --- | --- |
| Moonshot Kimi coding | **requires** `reasoning_content` on an assistant tool-call message | 322 |
| DeepSeek, direct OpenAI-compatible | **requires** the stored `reasoning_content` replayed | 815 |
| Mistral, strict OpenAI schema | **rejects** it with 422 `Extra inputs are not permitted` | 261 |

And jcode records a third lesson, from three crashes: reasoning arrives as a byte stream, and
slicing it at a non-character boundary panics and kills the process (issues 632, 633, 635).

**fx** stores no reasoning at all. It asks the endpoint to return an encrypted payload with
`include: ["reasoning.encrypted_content"]`, and it replays that payload from one opaque
`provider_state_json` on the message. See `fx-src/src/gateway/openai_codex.zig:109` and
`fx-src/src/core/shared/types.zig:909`.

fx gives rho two things, one to copy and one to avoid:

- **Copy the opaque payload.** A new host then needs no change to shared code. Section 4
  takes it.
- **Avoid the missing tag.** fx never records which provider wrote a payload, so a model or a
  provider switch can send a foreign blob. rho tags the payload with its owner, and rule 8
  drops a mismatch.

fx also gates the ask on a model capability, and it fails closed when the catalogue does not
list the effort. See `fx-src/src/core/config/model_capabilities.zig:95`. A project file in fx
may not set the model or the effort, so a cloned repository cannot raise your spend.

## 3. Where rho goes further

Three additions. Each answers a defect that neither reference fixes.

**One. The field names are data. The structure is not, and the spec says so.**

An earlier draft of this section claimed that a table makes every new endpoint a row and
never a branch. **A review proved that false, and the proof is Gemini.** Gemini does not put
reasoning in a named field on a choice. It marks a part with `thought: true` inside
`candidates[].content.parts`, and it carries `thoughtSignature` **per part**. No field name
selects that, so extraction is structural and a row cannot express it.

So the claim is narrowed to what is true:

- **Reading is per-provider code.** A provider is its own crate in rho, and parsing its wire
  format is that crate's job. That is the extension point: a new provider is a **new crate**
  behind `trait Provider`, and no shared code changes. Gemini arrives that way.
- **The table governs the OpenAI-compatible family only**, where the variation really is
  field names. `rho-provider-openrouter` serves many hosts through one wire format, and
  those hosts disagree only on the name. That is the case a table fits.
- **The replay policy is data for every provider**, because the policy is a small closed set
  and the failures are hard in both directions.

```rust
/// How one OpenAI-compatible endpoint names its reasoning, and what it wants echoed back.
/// A new host in this family is a row. A provider with a different structure is a new crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReasoningWire {
    /// The delta field names to read, in order. The first non-empty one wins, because a
    /// host that sends two fields with the same text would otherwise double it.
    pub read_fields: &'static [&'static str],
    /// What the endpoint needs echoed back on the next request.
    pub replay: ReplayPolicy,
    /// True when the endpoint rejects a reasoning field it did not send. Mistral answers
    /// 422 `Extra inputs are not permitted`, so rho must send nothing.
    pub rejects_unknown_fields: bool,
    /// True when rho must ask for thinking before the model emits a structured block.
    /// Without the ask, Claude writes `<thinking>` tags into ordinary text.
    pub ask_to_enable: bool,
}

/// What an endpoint needs echoed back.
///
/// Named `ReplayPolicy`, and not `ReasoningReplay`, because `ContentBlock::ReasoningReplay` in
/// section 4 already owns that name. One name per meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayPolicy {
    /// Send nothing back. The trace stays in the transcript for the reader.
    Never,
    /// Send the payload back while the same owner answers. Anthropic requires its signature
    /// inside a tool loop, and it rejects the turn without it.
    SignedWhileSameModel,
    /// Send the readable text back on an assistant tool-call message. The Kimi coding
    /// endpoint rejects the message without it.
    TextOnToolCall,
    /// Send a signature back on the matching tool call, read from that call's
    /// `ProviderState`. Gemini rejects the request with
    /// `Function call is missing a thought_signature`, and jcode carries the same token.
    SignatureOnToolCall,
}
```

Every policy runs **after** the owner check of rule 8. A mismatched owner sends nothing,
whatever the policy says. So a policy decides the shape of a replay, and the owner decides
whether a replay may happen at all.

**Two. rho reads `<thinking>` tags.** Neither pi nor jcode does. It is the defect the owner
reported, so rho fixes it. The rule is narrow on purpose:

- rho strips a tag pair only from the **start** of an assistant message, and only when the
  opening tag is the first non-space text. A tag in the middle of an answer is prose about
  tags, and rho must not eat it.
- The accepted names are `<thinking>` and `<think>`.
- Text inside becomes `ThinkingDelta`. Text after the closing tag becomes `TextDelta`.
- An unclosed tag ends at the end of the message, because a truncated stream must not lose
  the answer.
- Stripping never changes what reaches the provider. It changes what rho draws.

**Three. Nothing is dropped in silence.** Every content block has an explicit arm in every
provider's request builder. A block that must not travel is dropped in a named arm with a
comment, never by `_ => {}`.

## 4. The data model

```rust
pub enum ContentBlock {
    Text { text: String },
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
        /// The replay payload some providers bind to this call. Gemini rejects a request
        /// with `Function call is missing a thought_signature` when it is absent.
        ///
        /// This was `thought_signature: Option<String>`. A review killed that: the stream
        /// event carried no such field, so Gemini would have had to edit shared code, and
        /// a bare string had no owner, so it replayed after a model switch with nothing
        /// checking it. One carrier, owner-tagged, on every replay path.
        state: Option<ProviderState>,
    },
    ToolResult { tool_call_id: String, content: Vec<ContentBlock>, is_error: bool },

    /// Readable reasoning, kept for the reader. **It never reaches a provider.**
    ReasoningTrace { text: String },

    /// Reasoning the provider needs echoed back.
    ReasoningReplay {
        /// The readable text, for the reader.
        ///
        /// pi converts this to plain text when another model answers. rho does **not**: the
        /// whole block is dropped, and the report of rule 8 says why. Passing one model's
        /// reasoning to another as an answer changes what the second model reads, and no
        /// test here could show that it helps.
        text: String,
        /// The opaque replay payload. `None` means there is nothing to replay.
        state: Option<ProviderState>,
    },
}

/// Which provider and which model produced a replay payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningOwner {
    /// The value of `Provider::id`, for example `bedrock`.
    pub provider: String,
    /// The model id of the request that produced the payload.
    pub model: String,
}

/// One provider's own replay payload. Shared code never reads inside `value`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderState {
    /// The pair that may read the value. See behaviour rule 8.
    pub owner: ReasoningOwner,
    /// The provider's private shape. It holds an Anthropic signature, an OpenAI reasoning
    /// item, or whatever a later host needs.
    pub value: serde_json::Value,
}
```

The provider builds the whole `ProviderState`, because only the provider knows its own id and
the model of the request. So the stream event carries it, and the typed signature goes:

```rust
pub enum StreamEvent {
    // ... other variants unchanged ...
    ThinkingEnd {
        index: u32,
        /// Replaces `signature: Option<String>`. A signature now travels inside `value`.
        state: Option<ProviderState>,
    },
    ToolCallEnd {
        index: u32,
        arguments: serde_json::Value,
        /// New. Without it a provider cannot bind a replay payload to its call, and the
        /// extension point of section 7 is a promise rho cannot keep.
        state: Option<ProviderState>,
    },
}
```

`Thinking { thinking, signature }` is replaced by these. The rename is the point: the old
name never said which blocks travel, and that is why one was dropped in silence.

### Why one opaque payload, and not a typed field per provider

An earlier draft carried `signature: Option<String>` and nothing else. rho already **reads**
an OpenAI-shaped reasoning item in `rho-provider-azure`, and that item is an id, a summary
list, an encrypted blob, and a status. A single string cannot hold it. So rho could read a
shape it could never send back.

jcode answers this with a fourth typed variant. That puts every host's wire shape into shared
code, and shared code then changes for each new host. fx answers it with one opaque value per
message, and a new host costs nothing. rho takes fx's shape, and adds the tag fx lacks.

**The signature is not a second carrier.** Two carriers for one job means two answers, and
that is the family of defect that killed the `""` signature default. An Anthropic signature is
now one key inside `value`, written and read by the crate that owns the wire.

**A tool call carries the same opaque payload.** An earlier draft kept a typed
`thought_signature: Option<String>` here. A review killed it twice over: no stream event
carried the string, so Gemini would have had to edit shared code, and a bare string had no
owner, so it replayed after a model switch with nothing checking it. One carrier, owner
tagged, on every replay path.

**The honest cost.** The compiler no longer checks a payload. So rule 8 below is the whole
guard, and it fails closed. A test for a mismatched owner is not optional.

### What `value` may hold, and what bounds it

A security review found the first draft of this part unsafe. Both bounds below come from it.

**`value` holds opaque provider bytes only.** A signature, an encrypted blob, an item id, or a
status. **No readable text.** The OpenAI item's `summary` is readable, so it goes in `text`,
where the reader sees it and where the session cap already applies. This is what makes the
redaction exemption of rule 9 safe: opaque bytes cannot be scanned for a secret, and they hold
none that rho put there.

**Both texts are capped like any assistant text.** `ReasoningTrace.text` and
`ReasoningReplay.text` pass through `cap_block`, with the same spill as `ContentBlock::Text`.
A `TextOnToolCall` endpoint then replays a capped text. That is the deliberate trade: a turn
with reasoning past the cap is already pathological, and the other road writes a session file
that cannot be read back.

**A `value` is all or nothing.** Over `MAX_RECORD_BYTES` it is dropped whole, and rho reports
it. A truncated opaque token is useless, and a truncated one that still looks valid is worse.

**No wildcard may cover these blocks.** `redact_block` and `cap_block` in
`rho-core/src/session/mod.rs` both end in `other => other.clone()` today, so a new variant
joins them in silence. Each needs a named arm for both reasoning blocks.

**The owner tag is accident protection, and not authentication.** It stops an honest mismatch
after a model switch. It stops nothing in a session file that somebody crafted, because the
tag sits beside the payload it describes. rho trusts a session file exactly as much as it
trusts the rest of that file, and no more. The decision states this too, so nobody reads the
tag as a signature.

### The persisted format, in both directions

A session file is a contract with every other version of rho, so both directions are stated.

**A new rho reads an old file.** `"type": "thinking"` maps to `ReasoningTrace`. A signature
from an older session is stale, and replaying it would fail, so it is not carried over.

**An old rho reads a new file.** This was missing, and `AGENTS.md` step 3 requires it. An old
rho uses a tagged enum with no `reasoning_trace` variant, so it **fails the whole load**. That
is unacceptable, so both new variants share the old `"type": "thinking"` record:

```json
{"type": "thinking", "thinking": "...", "replay": true,
 "state": {"owner": {"provider": "bedrock", "model": "anthropic.claude-haiku-4-5"},
           "value": {"signature": "..."}}}
```

An old rho reads the text and ignores `replay` and `state`. A new rho reads `replay` to choose
between `ReasoningTrace` and `ReasoningReplay`. A missing `replay` key reads as `false`, which
is the safe direction: it never replays a stale payload.

**`replay: true` with no `state` reads as a trace.** There is nothing to send, so the block
becomes readable history. This is fail-closed, and it also covers a file written by a rho that
crashed between the two keys.

**A `state` value is stored verbatim.** A redacted payload cannot replay, so nothing rewrites
it on the way to the file. Rule 9 states the matching log rule.

**An old `signature` key is read and dropped.** A file from before this change maps to
`ReasoningTrace`, and a stale signature never replays.

**An imported pi session holds traces only.** `rho-session-import-pi` cannot know which
`Provider::id` and model wrote pi's `thinkingSignature`, so it can build no honest owner. It
writes `ReasoningTrace`, and an imported signature never replays. A review found this, and the
alternative was a guessed owner, which rule 8 exists to refuse.

### How one tag carries two variants

The first draft of this section was **wrong**, and a compile proved it. Two enum variants
cannot share one serde tag. Both `#[serde(rename = "thinking")]` variants compile, and the
reader then returns the **first** one every time. A replay block came back as a trace, and
nothing said so. That is a silent drop, which `AGENTS.md` names as a defect. See decision
`D-two-variants-cannot-share-a-serde-tag`.

So the file shape is its own type, and the conversion happens at the boundary:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "DiskBlock", into = "DiskBlock")]
pub enum ContentBlock { /* the variants of section 4 */ }

/// The on-disk shape. One `thinking` record carries both reasoning variants.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DiskBlock {
    Thinking {
        thinking: String,
        #[serde(default)]
        replay: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<ProviderState>,
        /// Read from an old file, and never written.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    // ... one variant per block of section 4 ...
}
```

The conversion holds the rules: `replay: true` with a state gives `ReasoningReplay`, and every
other case gives `ReasoningTrace`. This shape was transcribed to a scratch crate outside the
repository, and its four round-trip tests pass. See `docs/verification/` once the code lands.

**A truncated line.** A session log is append-only and line-delimited. A reader drops a final
line that does not parse, and it says so once. A half-written reasoning block must never stop
a session from loading.

## 5. The display

```rust
/// How rho draws reasoning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReasoningDisplay {
    /// Draw nothing. No text, and no summary row.
    Off,
    /// Draw a one-row summary only, for example `∴ thought for 2.4s`.
    #[default]
    Summary,
    /// Draw the reasoning text, dimmed, and keep it in the transcript.
    Full,
    /// Draw the reasoning text while it streams, then collapse it to the summary row.
    Live,
}
```

**A correction to an earlier draft.** It called these "jcode's three modes, and its default".
That was wrong twice. jcode has `Off`, `Full`, and `Current`, and it defaults to `Off`. The
earlier draft dropped `Off` entirely, which removed the only way to hide reasoning, and a
review named that a capability regression. `Off` is back.

rho keeps four modes and defaults to `Summary`. That is a deliberate difference from jcode: a
turn that thought for nine seconds and says so is honest, while a turn that hides the nine
seconds looks stalled.

The reasoning text draws in `Role::Muted`, never in `Role::Text`. Dim is the whole point: it
is the model's private work, and it must never read as the answer.

**The text is always stored, in every mode.** A user who switches to `Full` in the middle of a
session must not find the earlier reasoning empty. The cost is bounded, because the transcript
is already held in memory and reasoning is a fraction of it. `Off` changes what draws, and
never what is kept.

**When `Live` collapses.** "The model commits an answer" is not an event, so the earlier
wording was untestable. The rule is now stated against real events: `Live` collapses on the
first `TextDelta` that follows the reasoning block, or on a `ToolCallStart`, whichever comes
first. Both are observable, so a test can assert the exact frame.

Config key `tui-reasoning`, values `off`, `summary`, `full`, `live`. CLI `--reasoning <mode>`.
An unknown value is an error at every source, and rho draws nothing. See decision
D-a-bad-reasoning-mode-is-refused.

## 6. The behaviour rules

1. Read the delta fields in order, and take the first non-empty one.
2. An empty delta adds nothing and starts no block.
3. Slice a reasoning delta only on a character boundary. jcode crashed three times here.
4. Never send a `ReasoningTrace`. Send a `ReasoningReplay` only when `ReasoningWire` says so.
5. An orphaned tool call gets a synthetic error result, so the signature chain stays whole.
6. A redacted block replays only while the same model answers, and it is dropped otherwise.
7. Strip a leading `<thinking>` or `<think>` pair, and only a leading one. The rules below
   say how, because the stream is incremental and a message-global rule cannot be applied one
   delta at a time.
8. A provider replays a `state` only when `owner.provider` equals its own `Provider::id`, and
   `owner.model` equals the model of the request. A mismatch drops the state in a named arm,
   and the request carries no reasoning payload. The drop is never silent, and never partial.
9. A `state` value never reaches a log, at any level, including `trace`. It is written to the
   session file verbatim, because a rewritten payload cannot replay.
10. A `value` over `MAX_RECORD_BYTES` is dropped whole, and rho reports it. A reasoning text
    over the string cap spills, exactly as an assistant text does.
11. rho reports a `replay: true` record that carries no `state`. It reads as a trace, and the
    report says so once, so a provider bug does not hide.
12. A request builder has a named arm for every content block. A stream parser may keep a
    wildcard, because a wire event set is open and a provider adds events without rho.
    `rho-provider-azure/src/lib.rs:600` is a request builder, so its `_ => {}` goes.

### The tag rule, stated for a stream

A review found that rule 7 could not be implemented as first written. The pipeline emits a
delta at a time, so "the first non-space text" is unknown until enough text has arrived.

- rho holds a **lead buffer** until it can decide. The buffer is at most the length of the
  longest accepted opening tag, so an opening tag split across three deltas still matches.
- Nothing is emitted from the buffer until the decision is made. So a caller never sees text
  that rho later reclassifies.
- The tag name matches **without case**, so `<Thinking>` is a tag.
- `<thinking/>` is a self-closing tag. It opens and closes an empty reasoning block, and it
  never opens a region.
- Once the first non-space text is known not to be a tag, the buffer flushes as text and rho
  stops looking for the rest of the message. A tag in the middle stays text.
- **An unclosed tag stops at the first `ToolCallStart`.** Otherwise a missing closing tag
  would swallow the whole answer as reasoning. At the end of a message an unclosed tag also
  stops, and the text is kept.
- **The case rho cannot separate.** A user may ask the model to print a literal `<thinking>`
  tag, and the answer then legitimately begins with one. rho cannot tell that from real
  reasoning, because the bytes are identical. rho chooses the safer failure: when the model
  emitted a structured reasoning block in the same turn, rho does **not** strip a tag from the
  text, because the reasoning already arrived by the proper path. Tag stripping applies only
  to a turn with no structured reasoning at all.

## 7. Out of scope

- **Choosing the budget numbers by measurement.** Section 9 states a ladder, and it is a
  starting point, not a measured optimum. A measurement gets its own bench and its own spec.
- **A reasoning search.** The transcript holds the trace, and the terminal searches it.
- **Google and Mistral provider crates.** rho has none yet. Mistral joins the
  OpenAI-compatible table as a row. **Google does not**, and section 3 says why: its reasoning
  is structural, so it arrives as a new crate behind `trait Provider`. The `SignatureOnToolCall`
  replay policy, the `ProviderState` on a `ToolCall`, and the `state` field on `ToolCallEnd`
  exist now, so that crate needs no change to shared code when it lands.
- **Token accounting for reasoning.** Anthropic reports it in
  `output_tokens_details.thinking_tokens`, and the footer work is separate.

## 8. Test cases

### Reading the wire

- `the_first_non_empty_reasoning_field_wins` — a delta with `reasoning_content` and
  `reasoning` holding the same text yields the text **once**.
- `reasoning_content_is_read` — the field pi added for llama.cpp produces a delta. rho reads
  nothing here today.
- `reasoning_text_is_read` — the third name produces a delta.
- `an_empty_reasoning_delta_starts_no_block` — no `ThinkingStart` for an empty string.
- `a_reasoning_delta_splits_on_a_character_boundary` — a multi-byte character split across
  two deltas does not panic and does not corrupt.

### The tags

- `a_leading_thinking_tag_becomes_reasoning` — `<thinking>a</thinking>b` yields reasoning
  `a` and text `b`.
- `a_leading_think_tag_becomes_reasoning` — the short name works too.
- `a_tag_in_the_middle_stays_text` — `here is a <thinking> tag` stays text in full. This is
  the rule that stops rho eating an answer about tags.
- `an_unclosed_tag_ends_at_the_message_end` — a truncated stream keeps its text.
- `a_stripped_tag_does_not_change_the_request` — what reaches the provider is unchanged.

### The replay

- `a_trace_never_reaches_a_provider` — every provider's request builder omits
  `ReasoningTrace`.
- `a_state_replays_for_the_same_owner` — Anthropic gets its signature back, inside `value`.
- `a_state_is_dropped_for_another_model` — a model change drops the payload, by rule 8.
- `a_state_is_dropped_for_another_provider` — the other half of rule 8, which fx does not
  check at all.
- `an_absent_state_replays_nothing` — a `None` payload sends no reasoning field.
- `an_azure_reasoning_item_round_trips_through_the_state` — the id, the summary, and the
  encrypted blob all survive one turn. This is the case a single string could not carry.
- `a_tool_call_state_is_captured_from_the_stream` — `ToolCallEnd` carries the payload, so a
  provider needs no edit to shared code.
- `a_tool_call_state_is_dropped_for_another_model` — rule 8 covers the tool-call path too,
  which the first draft left ungoverned.
- `a_value_over_the_record_cap_is_dropped_and_reported` — rule 10, all or nothing.
- `a_reasoning_text_over_the_string_cap_spills` — rule 10, the text half.
- `a_reasoning_block_has_a_named_redaction_arm` and `a_reasoning_block_has_a_named_cap_arm` —
  no wildcard covers a reasoning block.
- `a_replayed_state_cannot_change_a_tool_call` — the payload rides along, and it never
  rewrites the request rho built.
- `an_oversize_reasoning_line_does_not_fail_the_whole_resume` — one bad record drops, and the
  session still opens. **Not built.** No production caller writes a session file, so the
  resume path has no caller to test. See `D-no-caller-writes-a-session-file`.

Added while building it, each for a reason the list above did not hold:

- `the_stream_captures_the_signature` and `a_stream_with_no_signature_yields_no_state` — the
  reader kept the reasoning text and dropped the signature, so there was nothing to replay.
- `the_sdk_translation_carries_a_signature` and
  `the_sdk_translation_carries_redacted_reasoning` — the live SDK translation dropped every
  signature in a wildcard arm, while every unit test passed. The unit tests build the wire
  mirror directly, so only a test over the SDK types can see this.
- `a_state_with_no_signature_is_dropped` — an unsigned payload is not a signed block, and an
  empty signature is a 400.
- `every_request_builder_has_an_explicit_arm` — a source guard that reads each match over a
  content block, arm by arm. It also drops comments first, because one named arm quotes
  `_ => {}` to say what it avoids, and the first version of the guard failed the build for
  that prose.
- `a_payload_under_the_cap_is_written_verbatim` — the other side of rule 10.
- `an_imported_pi_signature_never_replays` — the importer writes a trace.
- `a_rejecting_endpoint_receives_no_reasoning_field` — the strict-schema row sends nothing,
  so no 422.
- `a_tool_call_endpoint_receives_the_text` — the `TextOnToolCall` row attaches the text.
- `an_orphaned_tool_call_gets_a_synthetic_result` — the chain stays whole.
- `every_content_block_has_an_explicit_arm` — a compile-time exhaustive match, so no
  `_ => {}` can hide a new block. This is the test that would have caught defect three.

### The display

- `the_summary_row_states_the_span` — `∴ thought for 2.4s`.
- `off_mode_draws_nothing` — no text, and no summary row. `Off` was deleted by one draft and
  restored by a review, and it still had no test.
- `full_mode_draws_the_text_dimmed` — the rows carry `Role::Muted`, never `Role::Text`.
- `live_mode_collapses_when_the_answer_starts` — the text goes, the summary stays.
- `the_default_mode_is_summary`.

### The tag rule under a stream

- `an_opening_tag_split_across_three_deltas_matches` — the lead buffer holds until it decides.
- `a_mixed_case_tag_is_stripped` — `<Thinking>` matches without case.
- `a_self_closing_tag_opens_no_region` — `<thinking/>` yields an empty reasoning block.
- `an_unclosed_tag_stops_at_a_tool_call` — the answer is never swallowed.
- `a_turn_with_structured_reasoning_keeps_its_tags` — the case rho cannot separate. A turn
  that already produced a reasoning block leaves the text alone, so a model asked to print a
  tag prints it.
- `no_text_is_emitted_before_the_decision` — a caller never sees text rho later reclassifies.

### The replay policy

- `never_replay_sends_nothing` — the `Never` row sends no reasoning field.
- `a_signature_replays_on_the_matching_tool_call` — the `SignatureOnToolCall` row attaches
  `thought_signature`, so Gemini does not answer `Function call is missing a
  thought_signature`.
- `ask_to_enable_adds_the_thinking_request` — the Bedrock row asks for thinking, so the model
  returns a structured block instead of writing tags.

### The configuration

- `every_name_round_trips` — `off`, `summary`, `full`, and `live` each parse and print back.
  This is the tree's name for what an earlier draft called
  `the_reasoning_mode_parses_every_value`.
- `an_unknown_name_is_an_error` in `rho-core`, and `an_unknown_reasoning_mode_is_refused`
  plus `an_unknown_mode_in_the_environment_is_refused` in `rho-cli`. A wrong value never
  falls back in silence, at any source. See decision D-a-bad-reasoning-mode-is-refused.
- `a_bad_reasoning_value_fails_closed` in `rho-config` — the same rule for a file key.
- `the_flag_beats_the_config_and_the_environment` — the config leg needs a call site that
  reads a file, so this test moved to `SPEC-config-call-site`. `rho-cli` proves the two legs
  it has today with `the_flag_wins_over_the_env_var`.

### The persisted format

- `an_old_thinking_block_imports_as_a_trace` — a session file from before this change loads,
  and its stale signature is not replayed.
- `a_new_block_is_readable_by_an_old_rho` — the new variants serialise under the old
  `"type": "thinking"` tag, so an old reader loads the file instead of failing it.
- `a_missing_replay_key_reads_as_false` — the safe direction, so no stale signature replays.
- `a_truncated_final_line_does_not_stop_the_load` — a half-written block is dropped, and the
  session still opens.
- `the_reasoning_text_is_stored_in_every_mode` — switching to `full` mid-session shows the
  earlier reasoning.
- `a_state_round_trips_through_the_session_file` — the payload survives a resume unchanged.
- `a_replay_key_with_no_state_reads_as_a_trace` — the fail-closed direction.
- `an_old_rho_ignores_the_state_key` — the new key does not fail an old load.
- `a_state_value_never_reaches_a_log` — rule 9, asserted against a captured log at `trace`.
  The capture proves itself first, per `D-log-capture-proves-itself`.
- `a_replay_record_with_no_state_is_reported` — rule 11, so a provider bug is visible.
- `an_imported_pi_signature_never_replays` — the importer writes a trace.
- `a_crafted_owner_is_not_authentication` — pins the stated limit: the tag stops an accident,
  not a crafted file.

## 9. The ask, and the effort level

This section was added on 20260821, because the owner asked the plain question: who decides
that a model thinks, and how hard?

**The capability belongs to the model, not to the provider.** Bedrock serves Claude models
that think and Titan models that do not. So one provider crate answers per model id, and it
fails closed. An unknown id asks for nothing, because a field the endpoint does not know is a
400 for the whole turn.

**One level, two wire shapes.** An OpenAI-compatible host takes a word. An Anthropic-style
host takes a token budget, and it rejects a request whose `max_tokens` is not above that
budget. So the level is the user's word, and each provider maps it.

```rust
/// How hard the model should think. This is the user's word, not a wire value.
///
/// `Option<ReasoningEffort>` carries "unset" everywhere. `None` means the provider's own
/// default, so rho sends no field at all and a host keeps its own behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReasoningEffort {
    /// Ask the model not to think. A host with a disable switch gets it. A host without one
    /// gets no field, because rho must not invent a value.
    Off,
    Low,
    Medium,
    High,
    XHigh,
}

impl ReasoningEffort {
    /// The config, flag, and environment name. `off`, `low`, `medium`, `high`, `xhigh`.
    pub fn as_str(self) -> &'static str;

    /// The Anthropic-style budget for this level. `None` for `Off`.
    ///
    /// The ladder starts at 1024, because Anthropic rejects a smaller budget. The numbers
    /// are a starting point and section 7 says so.
    pub fn budget_tokens(self) -> Option<u32>;
}
```

`CompletionRequest` gains `pub reasoning: Option<ReasoningEffort>`, so a provider sees the
level with the request and needs no second channel.

| Level | Budget | Word on an OpenAI-compatible host |
| --- | --- | --- |
| `off` | none | `none`, where the host has one |
| `low` | 1024 | `low` |
| `medium` | 4096 | `medium` |
| `high` | 16384 | `high` |
| `xhigh` | 32768 | `xhigh` |

### The three rules a provider keeps

1. **An unsupported model asks for nothing.** No field, no budget, no error.
2. **The budget leaves room for the answer.** A provider raises `max_tokens` above the budget
   when it asks for thinking. Anthropic rejects the request otherwise.
3. **Thinking drops a temperature.** Anthropic allows only the default temperature with
   extended thinking, so a provider that asks for thinking sends no temperature.

### The configuration

A new key, and one more flag. The display mode and the effort are two different things: one
changes what you see, and the other changes what the model does and what you pay.

| Source | Name |
| --- | --- |
| config file | `reasoning-effort` |
| environment | `RHO_REASONING_EFFORT` |
| flag | `--reasoning-effort <level>` |

An unknown value is refused at its own source, and the error names that source. This is the
same rule as the display mode, and the reason is `D-the-merge-cannot-name-a-values-source`.

### Test cases for section 9

- `every_effort_name_round_trips` — the five names parse and print back.
- `an_unknown_effort_name_is_an_error` — no silent fallback.
- `the_budget_ladder_only_grows` — each level's budget is above the one below it.
- `off_has_no_budget` — `Off` maps to no budget.
- `the_request_carries_the_configured_effort` — the level reaches `CompletionRequest`.
- `a_thinking_model_gets_the_thinking_request` — the Bedrock row asks, so the model returns a
  structured block instead of writing tags. This is defect 1 of section 0.
- `an_unsupported_model_asks_for_nothing` — rule 1, tested against real Bedrock model ids.
- `an_absent_effort_asks_for_nothing` — `None` sends no field.
- `the_budget_leaves_room_for_the_answer` — rule 2.
- `thinking_drops_a_temperature` — rule 3.
- `a_bad_reasoning_effort_value_fails_closed` — the file key, in `rho-config`.
- `an_unknown_effort_flag_is_refused` and `an_unknown_effort_variable_is_refused` — the other
  two sources, each named in its own error.
- `the_run_path_strips_a_leading_thinking_tag` — `rho run` had no splitter, so it printed a
  tag as the answer while the TUI did not.

### How the replay was proved live

A passing tool loop proves nothing on its own, because the loop also passed while rho sent no
reasoning at all. So the signature was corrupted on purpose and the run repeated:

| Request | Bedrock |
| --- | --- |
| the real signature | 200, and the loop finishes |
| `deliberately-wrong-signature` | **400**, the request is invalid |

A wrong signature can only break a request that carries it. See
`docs/verification/reasoning-replay.md`.
