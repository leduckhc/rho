# D-anthropic-cache-fields-sum — two ephemeral fields, one cache count

Date: 20260901. Reference: `D-anthropic-cache-fields-sum`.
Spec: `docs/specs/20260901-192551-SPEC-anthropic-messages-provider.md`.

## The question

Anthropic reports cache hits in two fields: `cache_creation.ephemeral_5m_input_tokens`
and `cache_creation.ephemeral_1h_input_tokens`. rho has one field,
`Usage.cache_read_tokens`. What maps to what?

## The decision

**Sum the two ephemeral fields.** Default to zero when the parent object is absent.

```rust
usage.cache_read_tokens = anthropic_usage
    .cache_creation
    .as_ref()
    .map(|c| c.ephemeral_5m_input_tokens + c.ephemeral_1h_input_tokens)
    .unwrap_or(0);
```

## The reason

`Usage.cache_read_tokens` is the count of input tokens that were served from the cache.
Anthropic distinguishes two TTLs, but both are cache hits. The user cares about the total
hit rate, which `Usage::cache_hit_ratio` computes.

Splitting rho's one field into two would break every place that already reads it. Bedrock,
OpenRouter, and Azure all report one number. So the contract stays one field, and the
Anthropic provider sums.

Live probe confirmed the fields exist and carry non-zero values on a cached request.

## What it rules out

**Adding two new `Usage` fields for 5m and 1h.** That breaks the existing API and the cache
hit rate calculation. Bedrock and OpenRouter would fill neither.

**Picking one field and ignoring the other.** The ignored tokens would not count as cached,
so the hit rate would be wrong.

**Reporting zero when the fields are absent.** Already done. The `.unwrap_or(0)` handles it.

## The cost

One addition. The wire shapes are different, so the mapping code must exist. Summing is
minimal.
