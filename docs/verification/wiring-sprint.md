# The wiring sprint, driven for real

Date: 2026-08-25. Worktree: `rho-wiring`, branch `feat/wire-the-dead-switches`.
Binary: `target/release/rho`, 0.1.0, default features.
Spec: `SPEC-wire-the-dead-switches`.

Four capabilities existed in the code and reached no user. This records what each does now,
from a real run, and the breaks that prove the guards.

## The trust bypass

Recorded on its own page, because it is a security defect rather than a wiring one. See
`docs/verification/profile-trust-bypass.md`. In short: the same attack that loaded an
attacker's skill through a project profile now answers `NO`, `--trust-project` still answers
`YES`, and the environment door is closed too.

## A local model host, through `base-url`

A strict OpenAI-compatible stub, refusing every path but `/v1/chat/completions`:

```sh
OPENROUTER_API_KEY=local-key-not-used rho run "say the word" \
  --base-url http://127.0.0.1:8099/v1 --model my-local-model --no-skills

rho: base-url is set, so OPENROUTER_API_KEY goes to 127.0.0.1. Unset base-url to use the default endpoint.
LOCAL-HOST-REACHED
```

What the host received:

```
PATH /v1/chat/completions model=my-local-model stream=True auth=yes
```

**A defect this probe found.** The first stub answered any path, and the run looked like a
pass while rho had actually sent `/v1/api/v1/chat/completions`. OpenRouter serves its chat
endpoint under `/api`, and no other OpenAI-compatible host does, so the feature would have
returned 404 against Ollama or vLLM. `with_openai_host` now sets the standard path and accepts
a base with or without the `/v1` suffix. `with_base_url` keeps OpenRouter's path, because a
test points it at a mock of OpenRouter.

The lesson is about the probe, not the code: **a stub that accepts anything proves nothing.**

The refusals, each a real run:

```
rho run "hi" --base-url http://models.example.com/v1
rho: the base-url value "http://models.example.com/v1" is not valid: plain http is allowed
only to localhost, 127.0.0.0/8, or [::1], because the credential would travel in clear text

rho run "hi" --base-url https://x.example/v1 --provider bedrock
rho: base-url is set to https://x.example/v1, and the bedrock provider names its endpoint its
own way. Remove base-url, or use --provider openrouter.
```

## Two switches, not one

`--no-skills` used to remove every subagent in silence.

```sh
rho run "List every tool you have, names only." --no-skills --trust-project
# spawn_agent appears: subagents survive a skills switch now

rho run "Reply OK" --no-agents --trust-project
rho: agent discovery is off, so rho offers no subagent. Remove --no-agents to use one.
```

## Motion

`state.animate` was read by the renderer and assigned nowhere, so the sweep never drew. The
app now carries the choice, and `--no-motion`, `tui-motion = false`, and `RHO_REDUCE_MOTION=1`
each stop it. The footer names the state in words either way, so nothing rests on movement.

## The MCP cache

The write now sits in the `Ok` arm of the pool's connect task, which is the only place that
knows the handshake succeeded and still holds the tool list. `extensions::load` returns long
before that, so a write there would have cached nothing.

It reads the file, updates one entry, and renames a temporary file over the old one, under a
lock file that spans all three steps. An earlier version of this paragraph claimed two sessions
could never lose each other's entry with the rename alone, and a review disproved it: eight
writers left one entry. The lock is why the claim holds now. The persisted key is a hash of the
config
fingerprint, because a fingerprint embeds server `env` values and would otherwise write a
token to disk in clear text. A test asserts the file holds no secret.

## The guards

```sh
python3 bench/check-flag-names.py
VIOLATIONS 0 (checked against 73 defined flags)
```

Broken on purpose, by restoring the old message:

```
crates/rho-core/src/session/mod.rs:153: a message says to pass --allow-widen, and the command
line defines no such flag.
VIOLATIONS 1
```

**The cache lock, and a test that does not prove it.** A concurrency review ran eight servers
on eight threads against one cache file and one entry of eight survived, on every run. The
read-modify-write held no lock, so each writer overwrote the others, and a multi-server user's
cache never converged. That is the same class this whole change exists to fix, so the earlier
note calling it "one extra handshake" was wrong.

`record_tools` now holds a lock file across the read, the write, and the rename. A stale lock
is broken after two seconds, because a cache must never wedge a session.

`eight_concurrent_writers_keep_every_entry` exercises it, and **it stays green with the lock
removed on this machine**: the whole critical section finishes inside one scheduling quantum, so
the writers serialise by luck even behind a barrier. The lock's necessity rests on the review's
demonstration and on the shape of a read-modify-write, not on that test, and the test says so.

`bench/check-dead-surface.py` reports its ledger. It is **not** in the ship gate yet: after
the five entries I could justify precisely, 35 remain, and each needs its owning spec to write
an honest reason. Marking thirty-five in one pass would build the dustbin the decision
forbids, so the triage is the next job.

## Breaks that trip

| Break | Result |
| --- | --- |
| Stop recursing into profiles | FAILED, both profile trust tests |
| Forget `base_url` in the powerful set | FAILED, the completeness guard and the environment test |
| Leave the environment ungated | FAILED, the environment test |
| Restore the `--allow-widen` message | FAILED, the flag-name guard |

## Not covered

- A real local model. Ollama was serving an embedding model only, so a strict stub stands in
  for a chat model. The wire path is proved; a real model's output is not.
- A failing MCP handshake. The write sits in the `Ok` arm and no test drives the `Err` arm.
- The separator style in the binding table, which is still mixed.
