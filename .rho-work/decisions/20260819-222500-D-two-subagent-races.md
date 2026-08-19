# D-two-subagent-races — Two proven timing attacks need architectural fixes

**Questions (security review):** Can a cancelled child read post-teardown environment? Can a steering message race into a provider request?

**Decisions:**

1. **Pre-cancel environment snapshot.** A child session inherits its parent's environment. When a child is cancelled, the session tears down but the `CancelToken` stays alive. A long-running `bash` tool that does not `select` on `cancel` can read environment variables from the post-teardown context. **Fix:** Before the child session starts, snapshot the env into a new `Env` struct that the child carries. If the child is cancelled, the snapshot is still readable but the live process is gone. This is safe even if a tool forgets to check `cancel.is_cancelled()`.

2. **Queue lock during provider requests.** The driver drains the steering queue at a turn boundary, but a model can call `steer_agent` mid-turn and ask for a tool call in the next turn. Both run concurrently. The queue drain happens after tool calls finish, so a steering message can interleave with the next provider request. **Fix:** Lock the queue during the provider request, so a steering message blocks until the request completes. The model will not read the message until the turn after it was queued, which is safe and predictable.

Both require implementation. Neither breaks the public API.

---

**Evidence.** Both were proved via mutation testing in a scratch crate against real `rho-core`, with the relevant checks broken on purpose.
