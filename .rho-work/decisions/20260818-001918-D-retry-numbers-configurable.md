# D-retry-numbers-configurable — Retry is configurable in its numbers and fixed in its rules


The owner asked whether retry should be configurable. The answer is both, split along one
line, and the split is the important part.

**Already true, and correct.** `RetryPolicy` in `rho-core` carries `max_attempts`,
`base_delay_ms`, and `max_delay_ms`. It grows exponentially with full jitter, and a server's
`Retry-After` hint wins over the computed delay but is still capped, so a mistaken or
hostile header cannot stall a session for an hour.

**Configurable, because a user's situation differs.** The numbers interact with money and
with wall-clock time. A long backoff on a paid API wastes a user's afternoon. A short one
against a tight quota makes a rate limit worse. A shared corporate endpoint and a personal
key deserve different values. So the three numbers belong to the caller.

**Not configurable, ever: which errors may be retried.** `ProviderError::is_retryable`
allows `Transport`, `RateLimited`, and `Server`. It refuses `Client`, `Decode`, and `Auth`.
That is a correctness rule, not a preference. Letting a user retry a 401 would burn their
rate limit on a wrong key and never succeed. A configuration option there would only let
somebody choose to be wrong.

**The gap this question exposed.** Retry is **not consistent across the three providers**.

| Provider | Retry today |
| --- | --- |
| OpenRouter | rho's `RetryPolicy`, through `send_with_retry` |
| Azure | rho's `RetryPolicy`, through `send_with_retry` |
| Bedrock | **none of rho's.** It silently inherits the AWS SDK's own retry, standard mode, three attempts |

So `RetryPolicy` is a setting that two providers honour and one ignores, and `rho-provider-bedrock`
has no `with_retry` at all. A user who tuned retry would get two behaviours and not know it.
That is the same shape as decisions D-secret-in-core and D-one-redaction-home: one concern implemented in more than one
place, drifting.

**Decision.** Bedrock must honour the same policy. The right fix is to **drive the SDK from
rho's policy** rather than to re-implement retry around it: set the SDK's `RetryConfig`
attempts and backoff from `RetryPolicy` when building the client. Fighting a client library's
own retry produces two nested loops and a multiplied delay, which is worse than either alone.

**Not yet done.** `rho-core` and the provider crates are held by a sibling agent right now.
Recorded here so the gap is a task rather than a surprise.
