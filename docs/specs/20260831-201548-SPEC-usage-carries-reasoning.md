# SPEC-usage-carries-reasoning — one reasoning token count, measured or absent

Status: draft, for review before any implementation.
Prior art: `SPEC-reasoning-across-providers`, and `D-measured-cost-and-cache`.

## 0. The problem, in the user's words

The user wants to see thinking tokens and the thinking duration in the terminal.

The duration already shows. `rho-tui` times it and draws `∴ thought for 2.4s`. The token
count does not exist. `rho_core::Usage` counts input, output, and cache tokens only. This
spec adds the token count and defines the contract for it.

## 1. The sides

| Side | Owner | Must agree on |
| --- | --- | --- |
| The data model | `rho-core` | the new field, and its optional shape |
| The wire, read | each provider crate | which field carries the count, and when to set `None` |
| The persisted usage record | `rho-core` | what an old reader does with the new field |
| The screen | `rho-tui` | how the count draws in each reasoning mode |
| The provider contract | `rho-provider-testkit` | what the harness can assert |

Contract kinds touched: the data model, the wire format, the persisted format, the error
taxonomy, and the extension surface.

## 2. The data model

Add one field to `rho_core::Usage` in `crates/rho-core/src/usage.rs`.

```rust
/// Reasoning tokens the provider billed for this turn, when it reports them.
///
/// `None` means the provider reported no count. It never means zero. A provider that
/// measures zero sets `Some(0)`. The UI shows a count only for `Some`, so an absent
/// number never renders as `0`. Measured, never estimated.
/// See D-reasoning-tokens-are-optional-never-zero.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub reasoning_tokens: Option<u64>,
```

`Usage::add` sums the field, and a missing count stays missing, like `cost_usd`.

```rust
self.reasoning_tokens = match (self.reasoning_tokens, other.reasoning_tokens) {
    (Some(a), Some(b)) => Some(a + b),
    (Some(a), None) => Some(a),
    (None, Some(b)) => Some(b),
    (None, None) => None,
};
```

**Why `Option<u64>`, not `u64`.** A `u64` with `#[serde(default)]` reads a missing key as
`0`. An old session file, and a provider that reports nothing, would both show `0`
reasoning tokens. That is a claimed measurement nobody made. `AGENTS.md` forbids an
estimate. So the field is an `Option`.

## 3. Tokens versus duration

The two numbers come from two places.

- **The token count** is a provider fact. It rides on `Usage.reasoning_tokens`. A provider
  reads it from its own wire usage block. rho never derives it from text.
- **The duration** is a client measurement. `rho-tui` times it from the thinking row start
  to the answer start. It needs no provider support. It is not a `Usage` field.
  See D-reasoning-duration-is-client-measured.

**Which provider can supply the count.** The wire exposes it for two providers. Each field
name needs a live probe before implementation, as `D-measured-cost-and-cache` required for
cache counts.

- `rho-provider-openrouter`: `usage.completion_tokens_details.reasoning_tokens`.
- `rho-provider-azure`: `usage.output_tokens_details.reasoning_tokens`.
- `rho-provider-bedrock`: Bedrock `ConverseStream` reports no separate reasoning count. It
  leaves `reasoning_tokens = None`. The duration still shows, because the client times it.

When a provider cannot supply the count, the field holds `None`. The UI shows the duration
only. It never shows `0`.

## 4. The error taxonomy and the fail-open check

There is no new error enum. The one hazard is a fail-open number.

- The provider crate owns the `None` versus `Some(0)` distinction.
- A provider that reads no field sets `None`.
- A provider that reads a measured zero sets `Some(0)`.
- Shared code and the testkit must never write `Some(0)` to stand for "unknown".

This is the family of `ToolKind::Other`, where a missing value read as a safe default. Here
a missing count must not read as a measured zero.

## 5. The old reader

An old rho reads a new `usage` record. `Usage` has no `deny_unknown_fields`, so serde
ignores the extra key. The old rho reads the record without error. It drops
`reasoning_tokens` on read. `fork` re-encodes each record, so `fork` erases the number on
disk.

This is acceptable. The number is derived provider data. The next turn produces it again.
It is never a boundary decision, so a silent drop here is safe. This differs from a dropped
policy field, which would be a defect. See `crates/rho-core/src/session/mod.rs` `Entry`.

## 6. The extension point

A fourth provider reports the count with no edit to shared code. It sets
`Usage.reasoning_tokens` in the `StreamEvent::Usage(Usage)` it emits. `Usage` is a plain
struct, built by each provider. No shared `match` arm names a provider. A new provider is a
new crate that implements the `Provider` trait. It needs a new impl, not an edit to a
shared arm.

## 7. What the TUI renders

`rho-tui` holds the count for the current turn.

```rust
/// The reasoning token count of the current turn, when a provider reports one.
pub reasoning_tokens: Option<u64>,
```

The state sets it on `StreamEvent::Usage(usage)`. The thinking summary row reads it.

```rust
let summary = match (format_duration(row_duration(state, index)), state.reasoning_tokens) {
    (Some(span), Some(n)) => format!("{GLYPH_THINKING} thought for {span} · {n} reasoning tokens"),
    (Some(span), None) => format!("{GLYPH_THINKING} thought for {span}"),
    (None, _) => format!("{GLYPH_THINKING} thinking"),
};
```

Per reasoning mode, from `rho_core::ReasoningDisplay`:

- `Off`: draw nothing. No summary, and no count.
- `Summary`: draw the summary row. Append the count when `Some`.
- `Full`: draw the summary row, then the dimmed text.
- `Live`: draw the text while it streams. On collapse, draw the summary row with the count.

When the count is absent but the duration is known, the row shows the duration only. It
never shows `0 reasoning tokens`. A measured `Some(0)` shows `0 reasoning tokens`.

## 8. The provider contract test

The testkit adds one check to `crates/rho-provider-testkit/src/contract.rs`.

```rust
/// A provider that reports no reasoning count leaves the field `None`, never `Some(0)`.
pub async fn provider_contract_reasoning_tokens_absent_is_not_zero(harness: &dyn ProviderHarness) {
    let run = harness.run(Script::Text).await;
    let events = collect(run).await;
    for event in &events {
        if let StreamEvent::Usage(usage) = event {
            assert_ne!(
                usage.reasoning_tokens,
                Some(0),
                "a provider with no reasoning count must set None, not Some(0)"
            );
        }
    }
}
```

`run_all` calls this check too.

**What the contract can and cannot assert.** The `Script::Text` run has no reasoning. So
every provider must leave `reasoning_tokens = None` for it. The check proves the provider
does not invent a zero. The contract cannot assert a positive count, because a provider
that does not report one is still correct. The duration is a client measurement, so the
contract does not assert it. `rho-tui` tests assert the duration and the render.

## Test cases

### `rho-core` (`crates/rho-core/src/usage.rs`)

- `reasoning_tokens_defaults_to_none` — a `Usage` built with `..Default::default()` has
  `reasoning_tokens == None`.
- `an_absent_reasoning_count_does_not_serialize` — `None` drops the key, by
  `skip_serializing_if`.
- `a_measured_zero_round_trips_and_differs_from_absent` — `Some(0)` serializes, reads back
  `Some(0)`, and is not equal to `None`.
- `add_sums_reasoning_tokens_and_keeps_a_missing_one_missing` — `Some(a)` plus `None` is
  `Some(a)`, and `None` plus `None` is `None`.
- `an_unknown_extra_field_on_usage_is_ignored_on_read` — a `usage` JSON with a future key
  deserializes without error, proving the additive shape is safe for an old reader.

### `rho-provider-testkit` (`crates/rho-provider-testkit/src/contract.rs`)

- `provider_contract_reasoning_tokens_absent_is_not_zero` — a no-reasoning run leaves the
  field `None`, never `Some(0)`.

### Each provider crate

- `openrouter_reads_reasoning_tokens_from_completion_tokens_details` — a recorded usage
  fixture sets `Some(n)`; a fixture with no detail sets `None`. Field name pending a live
  probe.
- `azure_reads_reasoning_tokens_from_output_tokens_details` — the same, for Azure. Field
  name pending a live probe.
- `bedrock_leaves_reasoning_tokens_none` — a Bedrock metadata fixture sets `None`, because
  the wire has no such field.

### `rho-tui` (`crates/rho-tui/src/render.rs`, `state.rs`)

- `the_summary_row_appends_the_reasoning_token_count` — `Some(512)` draws
  `· 512 reasoning tokens`.
- `an_absent_count_shows_only_the_duration` — `None` draws `∴ thought for 2.4s`, with no
  token text.
- `a_measured_zero_shows_zero_reasoning_tokens` — `Some(0)` draws `· 0 reasoning tokens`.
- `off_mode_draws_no_token_count` — `Off` draws nothing.
- `live_mode_shows_the_count_on_collapse` — after the answer starts, the summary row carries
  the count.

## Out of scope

- The dollar cost of reasoning tokens. `D-three-reasoning-costs-stay-open` keeps that open.
- A persisted reasoning duration. The duration stays a live client measurement.
- A per-block token attribution. This spec carries one per-turn total.
- Any change to `Entry` to add `deny_unknown_fields` or a catch-all map. That is a separate
  defect.
- A token count estimated from text length or character count. `AGENTS.md` forbids it.
- Wiring a caller for `run_all`. Its dead-surface state is a separate task.

## Amendments after the contract review, binding

A contract review read this spec before any code. These amendments answer it. Where an
amendment and the text above disagree, the amendment wins.

### 1. No cumulative reasoning total reaches the screen

The review found a way for a measured number to become wrong rather than absent.

`Usage::add` sums the field, so a session total is easy to build. But a session file drops an
unknown field on read, and `fork` re-encodes each record, so a fork erases
`reasoning_tokens` from the copy. A cumulative total shown after a fork would therefore
shrink, and a shrinking measured number is worse than no number.

So rho shows a reasoning token count **per turn only**. No header, footer, or session summary
shows a cumulative reasoning total. If a later feature wants one, it recomputes from the turns
of the live run and never from a file it read back.

Test: `no_cumulative_reasoning_total_is_rendered`.

### 2. The testkit check needs a caller, or it is theatre

`rho_provider_testkit::contract::run_all` has no caller today, and
`bench/check-dead-surface.py` reports it. Adding a check to a harness nobody runs proves
nothing.

So this feature either gives `run_all` its first caller, in a test that every provider crate
runs, or it does not claim the check. The spec chooses the first. A provider crate gains a test
that calls `run_all`, so the new check runs in the workspace suite.

Test: `every_provider_runs_the_contract_harness`.

### 3. The wire path is verified on both providers, and it is nested

This amendment replaces an earlier one that marked OpenRouter and Azure unproven. A live probe
settled it, so the claim is now measured rather than deferred.

Both providers carry the count at the **same nested path**, and neither carries it at the top
level of `usage`:

```
usage.completion_tokens_details.reasoning_tokens
```

| provider | model or deployment | value seen |
| --- | --- | --- |
| OpenRouter | `openai/gpt-5` | 64 |
| Azure | deployment `gpt-5.5`, api-version `2025-04-01-preview` | 13 |
| Bedrock | `us.anthropic.claude-haiku-4-5` | the field is absent |

So one parser serves both, because both speak the OpenAI completion shape. The parser reads
`completion_tokens_details.reasoning_tokens` and treats a missing object or a missing key as
`None`. It never reads a top-level `reasoning_tokens`, because neither provider sends one.

Bedrock stays `None`, and that is a measured absence rather than an assumption.

Two further fields appeared in the same probe. They are out of scope here, and they are
recorded so a later feature does not have to probe again. OpenRouter sends
`usage.cost_details` beside the `cost` this project already reads. Azure sends
`usage.latency_checkpoint`, with a server-side time to first token.

Tests: `openrouter_reads_the_nested_reasoning_count`,
`azure_reads_the_nested_reasoning_count`, and
`a_missing_completion_tokens_details_reads_as_none`.

## Amendment: the request meant the reasoning text, and that already ships

The person who asked for "thinking and reasoning tokens" meant the **reasoning text on
screen**, and not a token count. This spec answered the wrong question, so the record must say
so plainly.

### What already works, driven live

A drive against Bedrock with `--reasoning-effort high` exercised all four display modes. Every
one behaved as its help text promises:

| mode | what landed on screen |
| --- | --- |
| `summary`, the default | one row, `∴ thought for 0.8s` |
| `full` | the same row, plus the whole reasoning text |
| `live` | the text streamed during the turn, then collapsed to `∴ thought for 5.3s` |
| `off` | no reasoning row at all |

So the reasoning text and its duration are both delivered today. `F-reasoning-display` is
correct, and no new work is needed for the text itself.

### The real gap is reach, not rendering

Three things keep a user from the text:

1. The default is `summary`, so a new user sees a duration and never the text.
2. There is no way to change the mode inside the session. It needs a restart with a flag.
3. Nothing in the interface says the modes exist. The tour and the key help do not name them.

The second gap belongs to `SPEC-choose-a-model-and-configure-a-run`, which already specifies a
`/reasoning-display` command. The first is a one-line default and a product choice. The third is
a guide and tour change.

### The token count is parked, and not built

The count is real. A probe measured it at
`usage.completion_tokens_details.reasoning_tokens`: 64 on OpenRouter, 13 on Azure, and absent on
Bedrock. See `docs/verification/provider-reasoning-probe.md`.

But nobody asked for it. This project builds only what a failing test needs, so the count stays
unbuilt and this spec stays a draft. The measurement is kept because it cost a probe, and a
later feature should not have to repeat it.
