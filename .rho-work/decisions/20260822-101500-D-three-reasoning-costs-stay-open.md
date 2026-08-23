# Three reasoning costs stay open, and each one names its trigger

Date: 20260822. Reference: `D-three-reasoning-costs-stay-open`.
From the performance review of the reasoning work.

## Why this file exists

A performance review found five costs. Two were fixed and measured: the replay now covers the
current tool loop only, and the reasoning row draws its tail instead of its whole text. See
`D-replay-only-the-current-loop` and `docs/benchmarks.md`.

Three were deferred. They were deferred in a conversation, which is not a record, so they are
written here. `AGENTS.md` allows a verified problem to wait, and it requires the wait to be
stated.

## 1. The transcript is deep-cloned once per turn

`Session::build_request` calls `context.messages().to_vec()`. Every block is cloned, including
every opaque `serde_json::Value`. That is O(blocks) clones per turn, and O(blocks squared) over
a session.

**Not fixed, because the fix changes a core type.** `Arc<Message>` in the context, or a request
that borrows `&[Message]`, both change the `Provider` contract and every provider crate with
it. That is its own spec.

**The trigger:** a measurement. No bench builds a multi-turn request yet, so the cost is
arithmetic, not a number. Write the bench first, and let it decide.

## 2. The reasoning trace has no live-memory bound

Section 5 of the spec says the text is stored in every display mode, so a user who switches to
`full` mid-session still sees the earlier reasoning. Nothing bounds that in memory. The caps in
`rho-core::session` measure the **persisted** record, not the live transcript.

The review named the family correctly: this project once shipped a memory cap that measured the
size of the kept output while the read buffer still grew. See `D-bash-line-cap`.

**Not fixed, because a bound here is a product decision.** Dropping a trace mid-session
contradicts section 5, and keeping it costs memory that grows with a long session. Somebody has
to choose, and the choice belongs to the owner.

**The trigger:** a long-session measurement, or the first report of memory growth. The compaction
work will meet this too, because compaction has to decide what a trace is worth.

## 3. Encrypted reasoning is base64-decoded on every turn

`replay_block` decodes the stored blob each time it replays. The review bounded it
analytically: at kilobyte payloads this is under a hundred microseconds even late in a session.

**Not fixed, because the cost is below the noise.** The review said to leave it, and it was
right. It is written here so nobody rediscovers it and calls it new.

## What this decision rules out

- No `Arc<Message>` or borrowed-request change without a bench and a spec.
- No cap on a live trace without an owner's ruling on section 5.
- No caching of a decoded blob until a measurement says it matters.
