# D-dead-surface-is-a-defect-class — a capability with no caller needs a CI guard

**Question:** writing the user guide found five features that exist in code and reach no user.
Each one passed every gate. Is this a run of bad luck, or a class we can check?

**Answer: a class.** Every instance has the same shape. A capability crate offers a function,
the binary never calls it, and the feature is silently absent. A test suite cannot see it,
because the code under test works. Only a user, or a doc writer who drives the product,
finds it.

The five, with the missing call in each:

| Capability | The dead call | What a user sees |
| --- | --- | --- |
| MCP tools | `McpSchemaCache::save` has no caller | Every run says the tools arrive next session. They never do. |
| Reduced motion | `motion_enabled` is never consulted | The sweep cannot be turned off. An accessibility need. |
| Named credentials | `Config::resolve_credential` has no caller | A `[credentials]` block does nothing. |
| Task progress | The task row ignores its `progress` field | A long build shows no percentage. |
| Subagents with `--no-skills` | One switch governs two capabilities | `spawn_agent` disappears, in silence. |

**Decision:** treat dead surface as a defect class, and add a CI guard for it. The guard
lists every `pub fn` in a capability crate that no code outside its own module and tests
calls. A hit is either a defect to wire or an item to delete.

A first pass of that check, written in Python over `crates/*/src`, reports six functions with
no call site anywhere: `default_search`, `with_retry`, `into_task`, `task_schema_fields`,
`sanitize_message`, and `is_scroll_pinned`. Two of those are the sanitizer wrapper and the
progress helper behind the table above.

**Why a guard and not a review rule.** AGENTS.md step 8 already asks for a check of the whole
surface, and five defects still shipped. A rule that a human applies is a rule that a tired
human skips. Every other guard in `bench/` exists because a review missed the thing it now
catches.

**What the guard must not do.** It must not fail on a function a third party calls, because
rho ships libraries for exactly that. So the guard needs an allowlist file, in the shape of
`bench/deleted-tests.txt`: one line per intentionally public and internally uncalled item,
with the reason. An empty reason is a failure.

**Rules out:** treating each of the five as an isolated bug, which is how the first four were
recorded before the fifth arrived. Deleting an uncalled function on sight, because a
library's public API has outside callers. Trusting a test suite to find this, since every one
of the five had passing tests over working code.

**Not decided here:** whether the guard runs over every crate or only over `rho-cli`'s
dependencies. `rho-core` and `rho-provider-testkit` exist to be called from outside, so they
may need the allowlist more than a guard.
