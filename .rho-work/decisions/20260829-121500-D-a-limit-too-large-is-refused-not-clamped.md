# D-a-limit-too-large-is-refused-not-clamped — the message names the key and its maximum

**Question:** `max_live_total` reaches `tokio::sync::Semaphore::new`, and tokio panics above
`MAX_PERMITS`. A config file therefore aborted the binary. Driven live:

```
$ printf '[subagents]\nmax-live-total = 18446744073709551615\n' > ~/.config/rho/config.toml
$ rho run "hi"
thread 'main' panicked at tokio-1.53.1/src/sync/batch_semaphore.rs:141:9:
a semaphore may not have more than MAX_PERMITS permits (2305843009213693951)
```

**Decision: rho refuses the value, and names the key and its maximum.** It does not clamp.

```
rho: the subagents.max-live-total value "18446744073709551615" is not valid: the maximum is 2305843009213693951
```

## Why a refusal and not a clamp

`D-your-settings-are-a-floor` argues against a value the user wrote and rho silently changed.
The whole subagent-limit design says a limit is the user's own statement, so answering a typo
with a different number and no word is the shape that decision rejects.

A clamp also hides the mistake. A user who wrote a number they misread keeps the wrong mental
model of their own configuration.

## Why the bound lives in `rho-core`

`rho-core` is the crate that calls `Semaphore::new`, so the bound is its own fact. Putting it in
`rho-config` would make the config crate know a runtime detail of a crate below it, and the flag
path would then need a second copy.

`SubagentLimits::check` is the one table. It has two callers: `Config::load` in `rho-config`,
which reports a `ConfigError::Value`, and `subagent_limits` in `rho-cli`, which reports the flag.

## The flag path had the same hole

Before the credential lane, `[subagents]` reached nothing, so only `--max-live-agents` could
panic the binary and nobody had met it. This lane made it reachable from a file, which is why
the fix lands here. Both paths are now checked, and each has its own test.

## Every numeric limit, not one field

`check` reads a table of all ten numeric limits, built from an exhaustive `let Self { .. }`
destructure with no `..` and no `_`. So a fifth limit fails the build until somebody decides its
bound.

Two limits become semaphore permits and carry `Some(MAX_COUNT)`. The other eight are compared
against a counter or a length, so they carry `None`.

**`None` is proved, not assumed.** `every_numeric_limit_is_bounded_or_provably_safe` drives the
extreme value of every unbounded field, `u32::MAX`, `usize::MAX`, and
`Duration::from_secs(u64::MAX)`, and asserts the check accepts them. So "this one needs no
bound" is a tested claim.

## Rules out

**A clamp.** See above.

**Bounding every field to `MAX_COUNT`.** Eight of them never reach a semaphore, and a bound
nobody needs is a refusal a user does not deserve.

**Validating in `rho-config` alone.** A flag reaches the same semaphore.

**Validating in `rho-cli` alone.** A file value should fail at load, before a session is built,
and with the config key in the message.

**Leaving the bound as a comment.** The panic was reachable for as long as the comment would
have been.
