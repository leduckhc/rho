# D-measured-cost-and-cache — Report the cache hit rate and the real cost, because a competitor only claims them


*Numbered 032 after a collision. A sibling agent claimed D-bash-os-sandbox for the bash sandbox while
this was being written, and both were appended to this file. See decision D-shared-working-tree: a shared
tree has no single owner, and an append is not a safe way to claim an identifier.*

jcode's site says its append-only context engineering keeps the provider prompt cache hot,
and it publishes no cache-hit-rate number. `SPEC-core-runtime` section 1 gives rho the same
discipline. So the opportunity is not to copy the claim. It is to **measure it**.

**What the controller found.** `Usage` already had `cache_read_tokens` and
`cache_write_tokens`. Bedrock filled them. **OpenRouter and Azure both hard-coded zero.**
So a user of two of the three providers could not see the saving that the whole
append-only rule exists to earn.

**The field names came from a live probe, not from memory.** That matters, because the
last two provider defects were both wrong guesses about a request shape.

| Provider | Path to the cache counts |
| --- | --- |
| OpenRouter | `usage.prompt_tokens_details.cached_tokens` and `cache_write_tokens` |
| Azure Responses | `usage.input_tokens_details.cached_tokens` and `cache_write_tokens` |
| Bedrock | `metadata.usage.cacheReadInputTokens` and `cacheWriteInputTokens` |

**Decision one.** All three providers report the cache counts, and `Usage::cache_hit_ratio`
turns them into a share. It returns `None` when there were no input tokens, so "no data"
cannot read as "no cache hits", and a caller cannot divide by zero.

**Decision two: report the cost the provider charged, never an estimate.** OpenRouter
returns `usage.cost`. A harness that multiplies tokens by a price table is wrong whenever a
price changes, a request falls back to another model, or a cached token is billed at a
discount. So `Usage::cost_usd` is an `Option`, it carries the charged amount where a
provider reports one, and it stays **absent** where none does. Absent is not zero, and
`Usage::add` keeps that distinction, because a total that silently reads as free is worse
than a total that admits it is unknown.

**A consequence worth stating.** `Usage` can no longer derive `Eq`, because a float has no
total equality. The derive line says so, to stop somebody adding it back.

**What is still unproven.** The controller tried to make Anthropic caching engage through
OpenRouter, with an explicit `cache_control` breakpoint on a 3609-token system prompt, and
saw `cached_tokens` stay at zero on both a cold and a warm call. That path is byok, so the
upstream key may not carry caching. **So rho now reports the number, and rho has not yet
demonstrated a non-zero hit rate end to end.** Automatically placing a cache breakpoint at
the end of the stable prefix is the obvious next step, and it is not done. Recorded as a
gap rather than implied.
