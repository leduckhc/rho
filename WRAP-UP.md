# Subagents Feature — Wrap-up

**Status: DELIVERED**  
**Branch: feat/subagents (14 commits, fast-forward from main)**  
**Tests: 900 passing, 0 failed**  
**Gate: ✓ fmt ✓ clippy ✓ test ✓ check-ids ✓ check-spec-tests**

---

## What shipped

### Core spawning
- `spawn_agent` — delegate work to one named child, get back a summary
- `spawn_agents` — fan-out to multiple children (bounded per-parent concurrency)
- Live handles: `steer_agent`, `cancel_agent` (can steer/cancel specific child mid-run)
- Steering queue (bounded=32, delivered at turn boundary)
- Acceptance gate: verify artifacts before returning to parent

### Limits & budgets (all CLI-configurable)
- `--max-agent-depth` (default: 1 — one level only)
- `--max-children-per-parent` (default: 4 — concurrent, not sequential)
- `--max-live-agents` (default: 32 — process-wide safety)
- `--max-agent-tool-calls` (default: 64 — inside one turn)
- `--child-timeout-secs` (default: 600 — per child)
- `--max-child-retries` (default: 3 — retry cap per work key)

### Specs (delivered status, test-verified)
1. `SPEC-subagents` — the core architecture
2. `SPEC-agent-tasks` — the task contract (goal + artifacts + acceptance)
3. `SPEC-steering` — the queue and delivery model

---

## Five defects found & fixed via live Bedrock

| # | Defect | Evidence | Fix |
|---|--------|----------|-----|
| 1 | Model couldn't learn which agents exist | Schema took free text; descriptions were parsed but never sent → model guessed agent names | Schema `enum` of loaded names + purpose text in tool description |
| 2 | Child transcripts never written | `crates/rho-core/src/subagent.rs` never called `create_dir_all` → permission denied silently | `create_dir_all(parent)` before write |
| 3 | Child timeout cancelled parent | Child's cancel token was a clone of parent's → child timeout triggered parent's receiver drop | `CancelToken::child()` creates isolated token; parent can cancel child but not reverse |
| 4 | All limits hardcoded | `subagent_limits()` had no CLI flags → user had no way to tune | Added `--max-agent-depth`, `--max-children-per-parent`, `--max-live-agents`, `--max-agent-tool-calls` |
| 5 | Retry cap unplugged | `RetryLedger` existed but no tool checked it → poisoned work retried forever | Wired ledger into `spawn_agent`; refusal on cap reached |

Each fix was mutation-tested: reverted the code, confirmed the test failed for the right reason, restored.

---

## Security findings & fixes

| Severity | Finding | Root Cause | Fix |
|----------|---------|-----------|-----|
| HIGH | `steer_agent` had unscoped registry lookup | Could steer another session's child | Added `AgentRegistry::live_under()` scoped accessor; tool uses it |
| CRITICAL | `MessageQueued` event never emitted | Announce was deferred; never reached frontend | Moved announce into `MessageQueue::push()` itself |
| MEDIUM | Sandbox mode comparison too permissive | Used `>=` instead of `>` | Fixed: only narrow, never widen |
| MEDIUM | Per-parent child count had TOCTOU window | Checked atomically, incremented non-atomically | Fixed: CAS loop, atomic throughout |
| LOW | Race tests for limits didn't reliably fail | No barrier to force simultaneous attempts | Added spin barrier; test now reliable |

All findings proved with deterministic test breaks and barrier-based race tests.

---

## Known limitations

1. **Steering only works in TUI/ACP.** In `run` mode, no parent session stays live, so the model never receives `steer_agent` or `cancel_agent` in its tool list. This is architectural, not a bug. (Verified: example binary works in ACP.)

2. **Steering scope is one direction.** A parent can steer/cancel only its own children, not siblings or ancestors. Scoped lookup prevents cross-session attacks.

---

## Verification

### Run it yourself

```sh
# 1. Steering works end-to-end (3 runs identical)
unset AWS_PROFILE
cargo run --release -p rho-cli --example steer_subagent

# 2. Every documented behaviour (30 assertions)
./bench/demo-subagents.sh
```

### What the proof covers

- ✓ Single spawn + fan-out
- ✓ Limits bite (depth, per-parent, tool-calls, timeout)
- ✓ Approval policy blocks child spawning under read-only
- ✓ Tool intersection works (child sees only parent's tools)
- ✓ Retry cap refuses repeated work
- ✓ Transcripts written to disk
- ✓ Child transcript never enters parent context
- ✓ Gate rejects child that skips artifact
- ✓ Gate refuses model-supplied command checks (trusted-author rule)
- ✓ Credentials scrubbed from child environment
- ✓ Steering reaches live child mid-run
- ✓ Cancel stops only named child, siblings keep running

---

## Test coverage

- **Unit tests**: 900 passing (agent_loop, cancel, queue, subagent_spawn, subagent_live_agent, steering, control)
- **Integration tests**: demo script (30 end-to-end checks against live Bedrock haiku)
- **Security tests**: mutation-proved gates (revert fix → test fails)
- **Race tests**: barrier-based concurrent spawn/cancel/steer scenarios

---

## Commits

```
291430c test(bench): guard the promise a spec makes about its own tests
5c1c382 docs: reconcile specs and verify the gate runner obeys sandbox constraints
143ba0b refactor(core): one handle to a live child, and it can really stop it
e833771 test(cli): a runnable proof that steering a subagent works
0231e6a feat(subagents): steer and cancel one running child
c1d7817 feat(core): a bounded steering queue, delivered at the turn boundary
9872b1b feat(subagents): the acceptance gate now has a caller
404bedf feat(subagents): make a live child addressable, visible, and bounded
74f9c70 feat(subagents): make the limits real with a fan-out, and wire the retry cap
ea6b232 fix(subagents): five defects a real Bedrock run found, and a green suite hid
c7b3551 docs: spec a small JSONL frontend and a verified agent task
21b1203 docs: correct the pi subagent claim, because nobody had read the source
```

---

## Next steps

- **JSONL frontend**: specced but not implemented (rho-jsonl crate, small headless protocol). Lower priority than subagents steering.
- **Reasoning across providers**: separate branch (feat/reasoning-across-providers). Not in scope here.
- **TUI steer integration**: works conceptually; needs UI rendering for queued messages.
