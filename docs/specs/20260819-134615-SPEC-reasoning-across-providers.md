# SPEC-reasoning-across-providers — one reasoning contract, and a table instead of branches

Status: draft, for review before any implementation.
Prior art: pi, jcode, and fx, all three read as source. See `docs/comparison.md`.

Amended 20260821. The replay payload is now one opaque, owner-tagged provider state. See
decision `D-reasoning-replay-is-opaque-provider-state`. Section 4 holds the change.

Amended 20260829. A refused replay is reported as data, and no longer only as a log line.
See decision `D-a-drop-report-is-data-not-a-log-line`. Section 6 holds the contract.

Amended 20260830. A tool result anchors the pending run instead of ending it. Rule 12 sent no
reasoning at all on every real request before this. See decision
`D-the-pending-run-includes-the-turn-a-tool-result-answers`.

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
`reasoning_text`, and it takes **the first non-empty one**. See
`pi-ai/dist/api/openai-completions.js:350`. Its comment names the reason, and the host:
one host returns two fields with the same content, so a naive reader doubles the text. The
comment names `chutes.ai` as that host.

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

| Endpoint | Rule | Where jcode says so |
| --- | --- | --- |
| Moonshot Kimi coding | **requires** `reasoning_content` on an assistant tool-call message | `openrouter-runtime/src/lib.rs:1441`, issue 322 |
| DeepSeek, direct OpenAI-compatible | **requires** the stored `reasoning_content` replayed | `openrouter_provider_impl.rs:67`, issue 815 |
| Mistral, strict OpenAI schema | **rejects** it with 422 `Extra inputs are not permitted` | `openrouter-runtime/src/lib.rs:1473`, issue 261 |

The issue numbers come from comments in jcode's own source, which is what rho read. Nobody
here has seen that tracker, and a docs audit was right to ask.

And jcode records a third lesson, from three crashes: reasoning arrives as a byte stream, and
slicing it at a non-character boundary panics and kills the process (issues 632, 633, 635).

**fx** stores no reasoning at all. It asks the endpoint to return an encrypted payload with
`include: ["reasoning.encrypted_content"]`, and it replays that payload from one opaque
`provider_state_json` on the message. See `fx-src/src/gateway/openai_codex.zig:99` and
`fx-src/src/core/shared/types.zig:873`.

fx gives rho two things, one to copy and one to avoid:

- **Copy the opaque payload.** A new host then needs no change to shared code. Section 4
  takes it.
- **Avoid the missing tag.** fx never records which provider wrote a payload, so a model or a
  provider switch can send a foreign blob. rho tags the payload with its owner, and rule 8
  drops a mismatch.

fx also gates the ask on a model capability, and it fails closed. `reasoningEffortSupported`
answers false for an effort the catalogue does not list, and the caller sets the field only
when that answer is true. See `fx-src/src/core/config/model_capabilities.zig:124` and `:201`.

**fx goes further than rho here, and its own test says so.** That file holds a test named
"capabilities never infer reasoning or Fast controls from model IDs". fx reads a gateway
catalogue instead. rho has no catalogue, and a Bedrock id does carry its version, so
`model_supports_thinking` reads the id. That is a weaker source of truth, and section 9 states
the cost: an inference profile ARN hides the model, so a capable model reads as incapable. rho
fails closed and reports it, which is the best an id can do.

A project file in fx may not set the model or the effort, so a cloned repository cannot raise
your spend.

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

**The table was specified and never built, so it is deleted.** An architecture review called
it a liability, and the evidence is in the tree: `Delta::first_reasoning` in
`rho-provider-openrouter` already reads the field names in order, with no table, and no
production code ever referenced `ReasoningWire` or `ReplayPolicy`. An unbuilt struct in a spec
is a promise a reviewer must keep checking and a shape the first real host may not fit.

What survives is what was learned, because that is what the next author needs:

- A host in this family disagrees only on the **field name**, so a reader takes the first
  non-empty of `reasoning_content`, `reasoning`, and `reasoning_text`.
- Kimi and DeepSeek **require** the stored field echoed back. Mistral **rejects** it with 422
  `Extra inputs are not permitted`. So replay is a per-host property, and guessing fails in
  both directions.
- rho therefore sends nothing on this family until a host arrives with a live proof, and it
  says so in a named arm. See `docs/verification/reasoning-effort.md`.

The replay policy that **is** built is the owner check of rule 8, in `ProviderState::for_owner`,
plus the loop scope of rule 13.

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

**An encrypted block replays without its text.** A provider that sends encrypted reasoning
gets its blob back, and the readable `text` beside it does not travel. So on that one path,
what the user reads and what the provider receives are different things. A security review
asked for the limit to be stated: a crafted file could pair benign text with a captured blob.
The blob is provider-encrypted, so it cannot be forged, and the paragraph below bounds how far
a session file is trusted at all.

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

**Nothing denies an unknown field, at any level.** The promise that an old rho loads a new
file rests on that, and a security review asked for it to be confirmed rather than assumed. It
holds for a header, a record, a message, and a block, and
`an_unknown_key_at_every_level_is_ignored` pins all four. A `deny_unknown_fields` anywhere in
this path would turn every later key into a failed load.

**A record read from a file is bounded as a whole, not only field by field.** The write path
caps the encoded record at `MAX_RECORD_BYTES`, and the read path caps each field. A record of
twenty thousand small blocks passes every field cap and still weighs megabytes, so the read path
checks the total too.

The check first tried to run only when the raw line already exceeded the cap. That was unsound,
and a probe measured why: `serde_json` writes `1e15` as `1000000000000000.0`, so a line packed
with floats in exponent form re-encodes 3.8 times larger, and a 60 kB record landed at 228 kB
inside the gate. The count is now exact for every record, and it stops at the cap, so measuring
one record costs no more than the cap.

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
4. Never send a `ReasoningTrace`. Send a `ReasoningReplay` only when the owner matches and
   the block is inside the current tool loop. See rules 8 and 12.
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
12. Only the current tool loop replays its reasoning. A block from before the last user
    prompt is history: the provider needs the thinking of the turns that carry the pending
    call, and nothing older. The prompt is append-only, so re-sending an old block costs its
    bytes on every later turn. A review worked that growth out from the code path as O(turns
    squared). It is arithmetic over the append-only rule, and not a measurement, because no
    bench builds a twenty-turn request yet.

    **A tool result anchors the pending run. It does not end it.** Take the trailing run of
    tool results, then take the maximal run of assistant turns that ends where it starts.
    Those are the turns the results answer. An earlier version ended the run at a tool result,
    and that sent nothing at all on every real request, because rho appends the tool results
    before it builds the request. See
    `D-the-pending-run-includes-the-turn-a-tool-result-answers`.
13. A request builder has a named arm for every content block. A stream parser may keep a
    wildcard, because a wire event set is open and a provider adds events without rho.
    `rho-provider-azure/src/lib.rs:600` is a request builder, so its `_ => {}` goes.

### How a drop is reported

Rule 8 says a drop is never silent. It first said that in a log line only, so the only test
that could prove the rule had to install a subscriber and read text. That seam broke. A
thread-local capture caught nothing under a parallel run, six times in four hundred runs. See
`D-a-callsite-caches-interest-globally` for the measurement.

So the report is **data**, and the log line is a courtesy. The request builder answers with
the payloads it refused.

```rust
/// Why one stored reasoning payload did not travel to the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayDropReason {
    /// The payload belongs to another provider, or to another model.
    AnotherOwner,
    /// The payload carries no signature, so Bedrock would refuse the whole turn.
    NoSignature,
    /// An encrypted payload did not decode from base64.
    UndecodableRedaction,
    /// The AWS SDK refused to build the block.
    ///
    /// No test reaches this arm today. `ReasoningTextBlock::builder` cannot fail once both
    /// required fields are set, and this code always sets both. It stays because the branch
    /// stays, and it fails closed.
    UnbuildableBlock,
}

impl ReplayDropReason {
    /// The one sentence a drop report says.
    pub fn report(self) -> &'static str;
}

/// One refused payload, named by where it sat and why it stayed behind.
#[derive(Debug, PartialEq, Eq)]
pub struct DroppedReplay {
    /// The index of the message the payload sat in.
    pub message_index: usize,
    /// Why the payload did not travel.
    pub reason: ReplayDropReason,
}

/// The messages a request carries, and the payloads that did not travel.
///
/// No `Default`, on purpose. A default would say "nothing was refused".
#[derive(Debug)]
pub struct BuiltMessages {
    pub messages: Vec<aws_sdk_bedrockruntime::types::Message>,
    pub dropped_replays: Vec<DroppedReplay>,
}

pub fn build_messages_for_model(messages: &[Message], model: &str) -> BuiltMessages;
```

The rules this shape keeps:

- **The wire does not change.** The caller reads `messages` and sends what it always sent.
- **No payload sits in the data.** `DroppedReplay` has no field for one, so rule 9 holds by
  construction.
- **The log and the data read one table**, `ReplayDropReason::report`. So a report cannot
  drift from the reason it names.
- **The enum stays exhaustive.** A new refusal breaks every reader on purpose. A catch-all
  arm is how `ToolKind::Other` approved every tool that forgot its kind.
- **Rule 12 is not a refusal.** A payload outside the current tool loop is history. It is not
  in `dropped_replays`, and it gets no log line, because a report on every turn is noise.

Only `rho-provider-bedrock` carries this shape today. Each other provider crate keeps its own
return type until the same need reaches it.

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
- `a_multibyte_reasoning_body_is_not_corrupted` — a multi-byte character split across
  two deltas does not panic and does not corrupt.

### The tags

- `a_leading_thinking_tag_becomes_reasoning` — `<thinking>a</thinking>b` yields reasoning
  `a` and text `b`.
- `a_leading_think_tag_becomes_reasoning` — the short name works too.
- `a_tag_in_the_middle_stays_text` — `here is a <thinking> tag` stays text in full. This is
  the rule that stops rho eating an answer about tags.
- `an_unclosed_tag_ends_at_the_message_end` — a truncated stream keeps its text.
- `a_stripped_tag_does_not_change_the_request` — what reaches the provider is unchanged.
  **Not built.** The splitter runs in a frontend, so a request cannot see it. U3 of
  `.rho-work/reasoning-expansion.md` holds the open choice.

### The replay

- `a_trace_never_reaches_a_provider` — every provider's request builder omits
  `ReasoningTrace`.
- `a_state_replays_for_the_same_owner` — Anthropic gets its signature back, inside `value`.
- `a_state_is_dropped_for_another_model` — a model change drops the payload, by rule 8.
- `a_state_is_dropped_for_another_provider` — the other half of rule 8, which fx does not
  check at all.
- `an_absent_state_replays_nothing` — a `None` payload sends no reasoning field.
- `an_azure_reasoning_item_round_trips_through_the_state` — the id, the summary, and the
  encrypted blob all survive one turn.
  **Not built.** Azure replays nothing yet, and rho has no account to drive.
- `a_tool_call_payload_reaches_the_transcript` — `ToolCallEnd` carries the payload, so a
  provider needs no edit to shared code.
- `a_tool_call_state_is_dropped_for_another_model` — rule 8 covers the tool-call path too.
  **Not built.** No provider binds a payload to a call yet. The block-level rule is proved
  by `a_state_is_dropped_for_another_model`.
- `a_value_over_the_record_cap_is_dropped_and_reported` — rule 10, all or nothing.
- `a_reasoning_text_over_the_string_cap_spills` — rule 10, the text half.
- `every_request_builder_has_an_explicit_arm` — no wildcard covers a reasoning block. It
  reads each arm of each match, and it drops comments first.
- `a_reasoning_text_over_the_string_cap_spills` and `the_payload_cap_is_inclusive` — the cap
  arms, proved by behaviour rather than by a name.
- `a_replayed_state_cannot_change_a_tool_call` — the payload rides along, and it never
  rewrites the request rho built.
  **Not built.**
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
  **Not built.** no host row exists, and section 3 now says why.
  so no 422.
- `a_tool_call_endpoint_receives_the_text` — the `TextOnToolCall` row attaches the text.
  **Not built.** no host row exists.
- `an_orphaned_tool_call_gets_a_synthetic_result` — the chain stays whole.
  **Not built.** the repair exists for a trailing call at resume only.
- `every_content_block_has_an_explicit_arm` — a compile-time exhaustive match, so no
  `_ => {}` can hide a new block. This is the test that would have caught defect three.

Added on 20260829, when the drop report became data. See
`D-a-drop-report-is-data-not-a-log-line`.

- `a_dropped_payload_is_reported` — rule 8, now asserted on `dropped_replays` and not on a
  log. It names the exact `ReplayDropReason` for each of the four refusals.
- `a_drop_names_the_message_it_sat_in` — `message_index` points at the right message, so a
  caller can say which turn lost its reasoning.
- `every_drop_reason_has_a_report` — every variant maps to a sentence, so the log line and the
  data cannot drift.
- `an_out_of_loop_payload_is_not_reported_as_a_drop` — rule 12 is history, not a refusal.
- `a_replayed_payload_is_not_reported_as_a_drop` — a good payload reports nothing, in the data
  and in the log.
- `a_drop_report_never_names_the_payload` — rule 9, asserted against the log **and** against
  the debug form of the data. A later field cannot leak a signature in silence.
- `the_capture_survives_a_callsite_reached_first_without_a_subscriber` — the guard for
  `D-a-callsite-caches-interest-globally`. It fails against a thread-local capture.

Added on 20260830, when a live probe found that no reasoning reached the wire. See
`D-the-pending-run-includes-the-turn-a-tool-result-answers`.

- `the_turn_a_tool_result_answers_replays_its_reasoning` — **the shape rho actually sends.**
  Every other scope test ended its list with an assistant turn, so none of them covered a real
  request.
- `parallel_tool_results_still_anchor_their_turn` — a run of results, not one, anchors the turn
  that made every call.
- `a_loop_that_ends_with_a_tool_result_still_sends_one_trace` — the no-growth invariant, now
  pinned on the shape that ships as well. The count stays one at 1, 5, 20, and 100 iterations.
- `a_turn_whose_call_was_already_answered_does_not_replay` — a closed chain stays history, so
  the wider scope did not become unbounded.
- `the_request_the_agent_loop_builds_carries_its_reasoning` — **the guard for the whole
  class.** It does not describe the shape. It asks `rho-core`'s agent loop for it, through a
  fake provider that records the request it receives on turn two. So a later change to how the
  loop orders or merges messages is seen with no edit to this test. It fails against the rule
  that shipped broken.
- `a_merged_pair_answered_by_a_tool_result_keeps_both_traces` — the two rules composed. A
  review found that an implementation replaying only the last turn passed both earlier tests,
  because one shape had a single-turn run and the other had no tool result.

### The display

- `the_summary_row_states_the_span` — `∴ thought for 2.4s`.
- `off_mode_draws_nothing` — no text, and no summary row. `Off` was deleted by one draft and
  restored by a review, and it still had no test.
- `full_mode_draws_the_text_dimmed` — the rows carry `Role::Muted`, never `Role::Text`.
- `live_mode_collapses_when_the_answer_starts` — the text goes, the summary stays.
- `the_default_mode_is_summary`.

### The tag rule under a stream

- `an_opening_tag_split_across_three_deltas` — the lead buffer holds until it decides.
- `a_mixed_case_tag_opens_a_block` — `<Thinking>` matches without case.
- `a_self_closing_tag_is_an_empty_block_and_keeps_the_rest` — `<thinking/>` yields an empty reasoning block.
- `an_unclosed_tag_before_a_tool_call_keeps_only_the_reasoning` — the answer is never swallowed.
- `a_turn_with_structured_reasoning_keeps_its_tags` — the case rho cannot separate. A turn
  **Not built.** `thinking.rs` states the opposite behaviour today, and the spec and the
  code must be reconciled before this lands.
  that already produced a reasoning block leaves the text alone, so a model asked to print a
  tag prints it.
- `no_text_is_emitted_before_the_decision` — a caller never sees text rho later reclassifies.
  **Not built.**

### The replay policy

- `never_replay_sends_nothing` — the `Never` row sends no reasoning field.
  **Not built.** no policy table exists.
- `a_signature_replays_on_the_matching_tool_call` — the `SignatureOnToolCall` row attaches
  **Not built.** no provider binds a payload to a call yet.
  `thought_signature`, so Gemini does not answer `Function call is missing a
  thought_signature`.
- `a_thinking_model_gets_the_thinking_request` — the Bedrock row asks for thinking, so the model
  returns a structured block instead of writing tags.

### The configuration

- `every_name_round_trips` — `off`, `summary`, `full`, and `live` each parse and print back.
  This is the tree's name for what an earlier draft called
  `the_reasoning_mode_parses_every_value`, and the tree calls it `every_name_round_trips`.
- `an_unknown_name_is_an_error` in `rho-core`, and `an_unknown_reasoning_mode_is_refused`
  plus `an_unknown_mode_in_the_environment_is_refused` in `rho-cli`. A wrong value never
  falls back in silence, at any source. See decision D-a-bad-reasoning-mode-is-refused.
- `a_bad_reasoning_value_fails_closed` in `rho-config` — the same rule for a file key.
- `the_flag_beats_the_config_and_the_environment` — the config leg needs a call site that
  reads a file, so this test moved to `SPEC-config-call-site`. `rho-cli` proves the two legs
  it has today with `the_flag_beats_the_config_and_the_environment`.

### The persisted format

- `an_old_thinking_block_imports_as_a_trace` — a session file from before this change loads,
  and its stale signature is not replayed.
- `a_new_block_is_readable_by_an_old_rho` — the new variants serialise under the old
  `"type": "thinking"` tag, so an old reader loads the file instead of failing it.
- `a_missing_replay_key_reads_as_false` — the safe direction, so no stale signature replays.
  **Not built.** covered by `a_replay_key_with_no_state_reads_as_a_trace`, which is the same rule with the name the tree uses.
- `a_truncated_final_line_does_not_stop_the_load` — a half-written block is dropped, and the
  **Not built.** the reader rule is proved by `a_bad_middle_record_does_not_discard_the_rest` in `rho-core`.
  session still opens.
- `the_reasoning_text_is_stored_in_every_mode` — switching to `full` mid-session shows the
  **Not built.**
  earlier reasoning.
- `a_state_round_trips_through_the_session_file` — the payload survives a resume unchanged.
- `a_replay_key_with_no_state_reads_as_a_trace` — the fail-closed direction.
- `a_new_block_is_readable_by_an_old_rho` — the new key does not fail an old load.
- `a_state_value_never_reaches_a_log` — rule 9, asserted against a captured log at `trace`.
  The capture proves itself first, per `D-log-capture-proves-itself`.
- `a_replay_record_with_no_state_is_reported` — rule 11, so a provider bug is visible.
- `an_imported_pi_signature_never_replays` — the importer writes a trace.
- `a_crafted_owner_is_not_authentication` — pins the stated limit: the tag stops an accident,
  **Not built.** the limit is stated in section 4 and in the decision. A test would assert an absence, so the claim lives in prose instead.
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

### What each provider does with the level

| Provider | What it sends |
| --- | --- |
| `rho-provider-bedrock` | `thinking` with a budget, for a model the table knows |
| `rho-provider-openrouter` | `reasoning.effort`, and `enabled: false` for `off` |
| `rho-provider-azure` | **nothing yet.** It reports once that the level had no effect |

Azure is a gap, not a silence. rho has no Azure account to drive, and guessing a field name is
how three providers rejected rho's requests in sprint 1. A report costs a user nothing and an
invented field costs a whole turn.

**Changing the level mid-session drops the provider cache.** The thinking request is part of the
request shape, so a new level changes the stable prefix. A performance review checked that
nothing else in this work moves the prefix: a replayed block is rebuilt byte for byte, so a
warm cache stays warm across turns.

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
