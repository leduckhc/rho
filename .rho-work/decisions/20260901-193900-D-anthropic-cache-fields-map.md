# D-anthropic-cache-fields-map — straight through the top-level fields

Date: 20260901. Reference: `D-anthropic-cache-fields-map`.
Spec: `docs/specs/20260901-192551-SPEC-anthropic-messages-provider.md`.
Supersedes `D-anthropic-cache-fields-sum`, which mapped the wrong direction.

## The question

Anthropic reports cache activity in **four** fields, not two. rho has two, `cache_read_tokens`
and `cache_write_tokens`. What maps to what?

## The evidence

A live probe of the Messages API returned this shape:

```
usage:
  input_tokens:                            13
  output_tokens:                            8
  cache_creation_input_tokens:              0   ← total tokens WRITTEN to cache
  cache_read_input_tokens:                  0   ← total tokens READ from cache
  cache_creation:
    ephemeral_5m_input_tokens:              0   ← breakdown of writes, 5 minute TTL
    ephemeral_1h_input_tokens:              0   ← breakdown of writes, 1 hour TTL
```

The **ephemerals are a breakdown of the write total**. Their sum equals
`cache_creation_input_tokens`. Neither of them is a read count. The read count sits at the
top level as `cache_read_input_tokens`, separate from every other field.

## The defect the earlier decision shipped

`D-anthropic-cache-fields-sum` said:

> `usage.cache_read_tokens = ephemeral_5m_input_tokens + ephemeral_1h_input_tokens`

That assigns the **sum of the write breakdown** to the **read field**. Two errors at once:

- It confuses the direction. Writes are counted as reads. A read rate becomes a write rate.
- It ignores the top-level totals that already carry the numbers, and reads a breakdown that
  exists only when the parent object is present.

A test that only asserted the field was populated would have missed both.

## The decision

**Straight through the top-level fields, no sum:**

```rust
usage.cache_read_tokens  = anthropic_usage.cache_read_input_tokens;
usage.cache_write_tokens = anthropic_usage.cache_creation_input_tokens;
```

Both fields default to zero when absent. The ephemeral breakdown is out of scope, because
rho does not distinguish TTLs.

## Why this is right

The rho-core `Usage` struct has two cache fields, `cache_read_tokens` and
`cache_write_tokens`. Anthropic reports two top-level totals, `cache_read_input_tokens` and
`cache_creation_input_tokens`. The names align, the semantics align, and the mapping is
one-to-one.

## What it rules out

- **No sum of ephemerals.** The breakdown is not the count. A change of Anthropic's TTL
  policy must not shift the mapping.
- **No assignment of a write count to a read field.** rho's own hit-ratio helper reads the
  two fields; a swap makes every ratio wrong.
- **No reliance on the parent object.** The top-level fields are always present when caching
  is active. A missing `cache_creation` object is not "no writes".

## Test cases

- `cache_read_tokens_reads_the_top_level_read_field` — a response with
  `cache_read_input_tokens = 5` produces `Usage.cache_read_tokens = 5`.
- `cache_write_tokens_reads_the_top_level_creation_field` — a response with
  `cache_creation_input_tokens = 7` produces `Usage.cache_write_tokens = 7`.
- `an_absent_cache_field_reads_as_zero` — a response with no cache fields at all keeps both
  Usage fields at zero.
- `the_ephemeral_breakdown_is_not_read` — a response whose top-level fields disagree with
  the sum of ephemerals keeps the top-level number, and the test comments say why.
