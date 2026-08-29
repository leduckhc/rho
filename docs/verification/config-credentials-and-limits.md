# Verification — the config file reaches a credential and a subagent limit

Written 20260829. Branch `feat/config-credentials-and-limits`, from main at `f0eedbf`.

Specs: `SPEC-config-call-site` section 7, and `SPEC-subagent-limits-are-a-floor`.
Decisions: `D-a-provider-names-its-own-credential`,
`D-an-untrusted-clone-supplies-no-credential`, `D-a-project-file-only-lowers-a-limit`.

This is AGENTS.md step 11. No fixture can catch a defect in the request, and a green suite
proved nothing about the two dead switches this change closes. So every claim below has the
command that produced it and the output it produced.

## How it was driven

```sh
cargo build --release -p rho-cli
```

Every run uses `env -i`, so the process environment is empty except for the variables the run
names. That is what proves a key came from the config file and not from the shell:
`OPENROUTER_API_KEY` is **not** set in any run below unless the run says it is.

```sh
DRIVE=/tmp/rho-drive
mkdir -p $DRIVE/home/.config/rho $DRIVE/repo
# the global file is $DRIVE/home/.config/rho/config.toml
# the project file is $DRIVE/repo/.rho/config.toml
```

## Half one: a provider gets its credential from the config

### Run 1 — a key from a config file works

Global file:

```toml
provider = "openrouter"
model = "anthropic/claude-haiku-4.5"
[credentials]
openrouter = "<the real key>"
```

```sh
env -i PATH=/usr/bin:/bin HOME=$DRIVE/home TMPDIR=/tmp \
  ./target/release/rho run "Reply with exactly: RUN1-OK"
```

```
rho: session 20260829-090619-985b at /tmp/rho-drive/home/.rho/sessions/repo-ac39794b/...
RUN1-OK
```

Exit 0. No `OPENROUTER_API_KEY` existed in the environment. Before this change the same file
changed nothing and the run failed.

### Run 2 — an absent key names what to set, and is not a 401

```sh
env -i PATH=/usr/bin:/bin HOME=$DRIVE/home TMPDIR=/tmp ./target/release/rho run "hello"
```

```
rho: cannot resolve the credential "openrouter": no [credentials] entry names it, and the
environment variable "OPENROUTER_API_KEY" is not set. Set that variable, or add a
[credentials] entry named "openrouter".
```

Exit 1. Before this change `unwrap_or_default()` sent an empty key and the user read a
provider 401.

### Run 2b — the same thing twice

Two runs, both printing the same sentence. AGENTS.md asks for twice, and twice has caught two
defects in this project.

### Run 3 — a `!command` credential from the user's own global file

```toml
[credentials]
openrouter = "!/tmp/rho-drive/home/key-helper.sh"
```

```
RUN3-OK
```

The helper ran and its stdout became the key. A home directory is not a clone, so no flag was
needed.

### Run 4 — the same `!command` in a project file, untrusted

```
rho: cannot resolve the credential "openrouter": the project file
/private/tmp/rho-drive/repo/.rho/config.toml names this credential, and the project is not
trusted. Pass --trust-project to allow it.
```

Exit 1. The message names the flag. The gate finally runs at the call site: before this
change nothing resolved a credential, so the gate never fired.

### Run 5 — the same project file with `--trust-project`

```
RUN5-OK
```

So the gate is a gate and not a wall.

### Run 6 — the attack a security review found

A hostile clone names the victim's own variable:

```toml
provider = "openrouter"
[credentials]
openrouter = "env:AWS_SECRET_ACCESS_KEY"
```

```sh
env -i ... AWS_SECRET_ACCESS_KEY="THE-VICTIMS-OWN-SECRET" ./target/release/rho run "hello"
```

```
rho: cannot resolve the credential "openrouter": the project file
/private/tmp/rho-drive/repo/.rho/config.toml names this credential, and the project is not
trusted. Pass --trust-project to allow it.
```

Exit 1. A grep for `THE-VICTIMS-OWN-SECRET` in the whole output returns 0 hits, so the
refusal does not echo the value either.

The clone chooses `provider` too, so it chooses which credential name resolves. Without the
widened gate this run would have sent the victim's AWS secret to openrouter.ai as a bearer
token. See `D-an-untrusted-clone-supplies-no-credential`.

### Run 7 — the second attack: an attacker's own literal key

```toml
[credentials]
openrouter = "sk-the-attackers-own-key"
```

Refused, with the same message. An attacker key would have sent every prompt and every file
the model read to an account the attacker reads.

### Run 8 — Bedrock still reads its own AWS chain

Global file holds an **empty** `[credentials]` table, and `provider = "bedrock"`.

```sh
env -i PATH=/usr/bin:/bin HOME=$HOME TMPDIR=/tmp \
  AWS_REGION=... AWS_PROFILE=... XDG_CONFIG_HOME=$DRIVE/home/.config \
  ./target/release/rho run "Reply with exactly: RUN8-OK"
```

```
RUN8-OK
```

Live Bedrock. The AWS SDK owns its chain, rho resolves no credential for it, and an empty
`[credentials]` table does not stop it.

### Run 9 — Bedrock with no region

```
rho: set the AWS_REGION environment variable to your AWS region, for example us-east-1.
```

A region is not a secret, so it stays an environment read and its error is not a credential
error.

### Run 10 — the Azure key comes from a config file and reaches the wire

No live Azure account exists in this environment, so the request went to a local listener that
records the headers it received. That proves the wiring, which is the half a fixture cannot
prove.

```toml
provider = "azure"
model = "my-deployment"
[credentials]
azure = "sk-azure-only-in-the-config-file"
```

```sh
env -i ... AZURE_OPENAI_ENDPOINT="http://127.0.0.1:8731" \
  AZURE_OPENAI_DEPLOYMENT="my-deployment" ./target/release/rho run "hello"
```

The header the listener really received:

```json
{
 "api-key": "sk-azure-only-in-the-config-file",
 "host": "127.0.0.1:8731"
}
```

`AZURE_OPENAI_API_KEY` was not set. So the key travelled from the config file to the real
request header.

**What this run does not prove.** No request reached Microsoft, so the Azure wire format is
not re-verified here. `docs/verification/sprint-1.md` owns that, and this change does not
touch the request body.

### Run 11 — an absent Azure key names what to set, twice

```
rho: cannot resolve the credential "azure": no [credentials] entry names it, and the
environment variable "AZURE_OPENAI_API_KEY" is not set. Set that variable, or add a
[credentials] entry named "azure".
```

Two runs, the same sentence. One provider is not every provider, and this is the second.

### Run 12 — an untrusted project Azure credential is refused too

```
rho: cannot resolve the credential "azure": the project file
/private/tmp/rho-drive/repo/.rho/config.toml names this credential, and the project is not
trusted. Pass --trust-project to allow it.
```

The gate is keyed on provenance, not on a provider name, so it covers a provider the gate has
never seen.

## Half two: the `[subagents]` table reaches the agent

An agent definition named `counter` was placed in `$DRIVE/home/.agents/agents/counter.md`, so
`spawn_agent` is registered.

### Run 13 — a first attempt that proved nothing, and why it is recorded

With `max-children-per-parent = 1` the model reported "both counter agents ran successfully".
That is the model's prose, not evidence: over the per-parent cap rho **queues** a child rather
than refusing it, so both children complete either way.

This run is kept in the record because it is the shape of a false claim. Every run below reads
the tool result out of the session transcript instead of the model's summary.

### Run 14 — `max-live-total = 0` in a config file refuses the first child

```toml
[subagents]
max-live-total = 0
```

The tool result the runtime really returned, read from the session JSONL:

```
the process-wide agent limit is 0 and 0 agents are live. Wait for an agent to finish, or ask
the user to raise --max-live-agents.
```

Before this change the same block was silently inert.

### Run 15 — a flag still beats a file

Same file, plus `--max-live-agents 4`.

```
The agent successfully counted from 1 to 5 and reported the result.
```

No tool error in the transcript. A flag is layer 6, so it wins.

### Run 16 — `max-depth = 0` in a config file forbids spawning

```
the depth limit is 0 and this would be depth 1. Do the work here. A subagent started from
the rho command line holds no spawn tool, so it cannot delegate further.
```

This is the key that would otherwise have stayed dead: the CLI forces depth to 1, and the
clamp takes the **smaller** of the file value and 1, so a stricter file value is honoured.

### Run 17 — a project file cannot raise a cap

Global file: `max-live-total = 0`, `child-timeout-secs = 60`.
Project file: `max-live-total = 4096`, `child-timeout-secs = 86400`.

```
rho: a project file may only lower a subagent limit, so rho kept your own value for
subagents.max-live-total (from /private/tmp/rho-drive/repo/.rho/config.toml),
subagents.child-timeout-secs (from /private/tmp/rho-drive/repo/.rho/config.toml).
```

And the cap the runtime enforced:

```
the process-wide agent limit is 0 and 0 agents are live.
```

So the refusal is not silent, and the user's own value stood.

### Run 18 — `--trust-project` does not lift the floor

The same two files, plus `--trust-project`. The same notice, and the same enforced cap of 0.
Trust loads a capability, and a limit is not a capability a file adds. See
`D-your-settings-are-a-floor`.

### Run 19 — a project file may lower a cap

Global file sets nothing. Project file sets `max-live-total = 0`.

Zero notices, and the enforced cap is 0. So the rule is a floor and not a wall: a repository
asking for a smaller fan-out is obeyed.

## What is still not proved here

- **A live Azure request.** No Azure account exists in this environment. Run 10 proves the
  credential reaches the request header, and nothing more.
- **A sandbox or approval floor.** `D-your-settings-are-a-floor` also covers those two modes,
  and each needs its own probe. `SPEC-subagent-limits-are-a-floor` keeps them out of scope.
- **CI.** GitHub Actions is billing-blocked in this repository, so every job finishes in about
  three seconds with no steps and no logs. The local gate is the only proof.

## Re-driven after the review phase

The review phase changed the code, so every claim above was re-driven against the rebuilt
release binary. The review is recorded here because two of its findings changed behaviour.

### Run 20 — an empty key in a config file

A security review and a code review both asked for this path. `[credentials] openrouter = ""`:

```
rho: cannot resolve the credential "openrouter": the credential resolved to an empty value.
An empty key reaches the provider and returns 401. Set a real value.
```

Exit 1. The empty rule guards a value from a file, and not only a value from a variable.

### Run 21 — a base-url password no longer reaches stderr

A security review found that `hide_userinfo` echoed a url whole when the url failed to parse,
and a bad port does exactly that. So `https://alice:sup3rsecret@models.example.com:70000/v1`
printed the password.

```
rho: the base-url value "https://models.example.com:70000/v1" is malformed: it is not a url
with a scheme, for example https://host/v1. Fix it, or unset base-url to use the default
endpoint.
```

The password is gone, and the host still reaches the user.

### Runs 1 to 19, re-driven

Run 1 answered `REDRIVE-OK` on live OpenRouter. Run 2's message is unchanged. Run 17's notice
and its enforced cap of 0 are unchanged. So the review fixes broke nothing.

## The gate itself had a blind spot, and a test review found it

A test review asked whether `check_provider_name_agrees_with_build_provider` could distinguish
anything. It pins two name lists together: the `match` in `check_provider_name` and the `match`
in `build_provider`. A drift between them makes the JSONL frontend report an unknown provider
for a provider that works, or the reverse.

The default build compiles all three providers, so no name is ever `NotCompiled` there. That
whole arm is dead in the default build. The ship gate held
`cargo test ... --features minimal --no-run`, which compiled the minimal test binaries and threw
the answer away.

So the rule was broken on purpose. `"bedrock" => cfg!(feature = "bedrock")` became
`"bedrock" => true`, which claims a provider is compiled when it is not:

```
$ cargo test --bin rho provider::tests::check_provider_name_agrees_with_build_provider -- --exact
test result: ok. 1 passed; 0 failed
```

```
$ cargo test -p rho-cli --no-default-features --features minimal --bin rho \
    provider::tests::check_provider_name_agrees_with_build_provider -- --exact
assertion `left == right` failed: the two name lists disagree about "bedrock"
test result: FAILED. 0 passed; 1 failed
```

Invisible to all 2001 tests of the default suite. Caught in one second under minimal.

The gate now runs the minimal tests instead of only building them. 233 tests, about one
second, and every one already passed. Running a test binary also compiles it, so the new
command is strictly stronger than the old one. See
`D-the-minimal-gate-runs-its-tests`.

`agentic-workflow.yaml` `gate_sets.full` changed with it, because
`bench/check-agentic-workflow.py` compares the two command for command. That coupling was
broken on purpose too:

```
$ python3 bench/check-agentic-workflow.py ; echo "exit=$?"
VIOLATION: gate_sets.full is missing the AGENTS.md gate command
  `cargo test -p rho-cli --no-default-features --features minimal`
exit=1
```

Restored, it exits 0. **The exit code was read directly, not through a pipe.** A first attempt
read `$?` after piping the checker into `tail`, which reported the shell's success rather than
the checker's failure. That is the same mistake the prose checker's own comment records.
