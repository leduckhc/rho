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

`state.animate` was read by the renderer and assigned nowhere, so the sweep never drew.
The app now carries the choice.

Driven for real. Each form parses correctly and reaches the config layer:

```sh
# --no-motion bare: turns animation off
rho run "say OK" --no-skills --no-motion
# exits: rho: authentication failed: OpenRouter rejected the API key (status 401).
# no argument error, flag parsed as Some(true)

# --no-motion false: overrides a global tui-motion = false
rho run "say OK" --no-skills --no-motion false
# exits: same auth error, no argument error, flag parsed as Some(false)

# bad value: parser rejects it
rho run "say OK" --no-motion bad
error: invalid value 'bad' for '--no-motion [<NO_MOTION>]'
  [possible values: true, false]
```

The TUI runs only in an interactive session. In a headless run, the flag reaches the config
layer and stops there. The footer names the motion state in words either way, so nothing rests
on movement being visible.

## The MCP cache

The write now sits in the `Ok` arm of the pool's connect task, which is the only place that
knows the handshake succeeded and still holds the tool list. `extensions::load` returns long
before that, so a write there would have cached nothing.

It reads the file, updates one entry, and renames a temporary file over the old one, under a
lock file that spans all three steps. An earlier version of this paragraph claimed two sessions
could never lose each other's entry with the rename alone, and a review disproved it: eight
writers left one entry. The lock is why the claim holds now.

The persisted key still hashes the full `fingerprint()`, including server `env` values.
An intermediate fix dropped `env` values from the key, and it was reverted.
The pool keys on `fingerprint()`, so dropping values made two servers that differ only
by an `env` value share one cache entry and serve the wrong tool list at turn one.
Hashing alone does not protect a low-entropy `env` value from offline brute force anyway:
the hash is unsalted 64-bit FNV-1a over a format that is public in the source.
Confidentiality comes from the file mode: the cache is `0o600` in a `0o700` directory,
matching `rho-core/src/transcript.rs`, and a test pins it.

Driven for real. An isolated `HOME` under `mktemp -d` and a minimal stdio MCP server in Python.

```sh
# Cache absent before the run
ls $HOME/.rho/
ls: /tmp/isolated-home/.rho/: No such file or directory

# Run 1: cache does not exist yet
HOME=/tmp/isolated-home rho run "say OK" \
  --mcp-config /tmp/mcp-probe/mcp.json --no-skills --model openai/gpt-4o-mini
rho: 1 MCP server(s) are configured, and no tool schema is cached yet. rho is
      connecting now, and their tools are available in the next session.
rho: authentication failed: OpenRouter rejected the API key (status 401).

# Cache present after the run:
ls $HOME/.rho/
mcp-schema-cache.json

cat $HOME/.rho/mcp-schema-cache.json
{
  "version": 1,
  "entries": {
    "1c9d59af7d65f9cd": {
      "server": "probe",
      "tools": [{"name": "echo_upper", ...}],
      "last_used": "1787773614748"
    }
  }
}

# Run 2: notice is gone, tools are available
HOME=/tmp/isolated-home rho run "say OK" \
  --mcp-config /tmp/mcp-probe/mcp.json --no-skills --model openai/gpt-4o-mini
rho: authentication failed: OpenRouter rejected the API key (status 401).
# No MCP notice. The cache was read and echo_upper was advertised.
```

The critical defect was: `spawn_connect` wrote the cache on a detached `tokio::spawn` that
nobody joined. A fast `rho run` exited and killed the write, so the cache was never written
and the notice repeated forever. `drain_connects` lands after every turn and awaits those tasks
before the process exits. The second run above proves the fix.

## The guards

```sh
python3 bench/check-flag-names.py
VIOLATIONS 0 (checked against 27 defined flags)
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

`record_tools` now holds a lock file across the read, the write, and the rename. A stale
lock is broken after two seconds, because a cache must never wedge a session. Each
`LockGuard` writes an unguessable nonce when it acquires the lock. A guard steals only
when the same nonce persisted unchanged past the timeout. `Drop` removes the file only
when it still holds that guard's nonce, so a guard can no longer delete a successor's
lock — that was a real defect a critic found and proved with overlapping writers.

Two residuals, named so they are not lost. A writer stalled past the steal timeout can
still be raced by a second steal. There is no eviction bound: `last_used` is recorded,
but no pruning runs yet.

`eight_concurrent_writers_keep_every_entry` exercises it, and **it stays green with the lock
removed on this machine**: the whole critical section finishes inside one scheduling quantum, so
the writers serialise by luck even behind a barrier. The lock's necessity rests on the review's
demonstration and on the shape of a read-modify-write, not on that test, and the test says so.

`bench/check-dead-surface.py` reports its ledger. It is **not** in the ship gate yet.
The count moves as code changes, so the command output is the authoritative figure.
Each uncalled item needs its owning spec before an honest reason can be written.
`bench/allowed-uncalled.txt` holds the current allowlist. Marking every item in one
pass would build the dustbin the decision forbids, so the triage is the next job.

```sh
python3 bench/check-dead-surface.py 2>&1 | tail -1
VIOLATIONS 45 (checked 112 source files)
```

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
