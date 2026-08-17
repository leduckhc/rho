# Sprint 1 verification, stage S9

This document records what a person actually ran against real services. It is not
a summary of the test suite. The suite proves the code does what the tests say.
This file proves the product works.

Date: 2026-08-17. Platform: macOS on Apple Silicon (aarch64). Build:
`cargo build --release -p rho-cli`.

## What ran live

| Check | Provider | Result |
| --- | --- | --- |
| Streamed answer | OpenRouter | **pass** |
| Streamed answer | AWS Bedrock | **pass** |
| Streamed answer | Azure OpenAI | **not run**, no credential available |
| Tool call, end to end | OpenRouter | **pass** |
| Path confinement holds against a live model | OpenRouter | **pass** |
| `--read-only` blocks a write | OpenRouter | **pass** |
| Actionable error for a missing key | OpenRouter | **pass** |
| Non-zero exit on failure | OpenRouter | **pass** |

### Streamed answer, OpenRouter

```sh
rho run "Reply with exactly: rho works" \
  --provider openrouter --model "anthropic/claude-haiku-4.5"
```

Output: `rho works`. Exit code 0.

### Streamed answer, AWS Bedrock

```sh
rho run "Reply with exactly: bedrock works" \
  --provider bedrock --model "us.anthropic.claude-haiku-4-5-20251001-v1:0"
```

Output: `bedrock works`. Exit code 0. Credentials came from the standard AWS chain,
through the `AWS_PROFILE` and region already set in the environment. rho signed the
request with SigV4 and did not hand-roll any signing.

### Azure OpenAI

Not run. No Azure credential was available on this machine. The unit tests cover
the wire mapping and pin the Entra audience `https://cognitiveservices.azure.com/`,
but **nobody has yet seen rho talk to a real Azure endpoint.** Treat Azure as
untested until somebody runs this file's OpenRouter commands against it.

### Tool call, end to end

```sh
cd /tmp/rho_e2e
echo "The secret passphrase is: purple-anvil-42" > note.txt
rho run "Read the file note.txt and tell me the passphrase it contains." \
  --provider openrouter --model "anthropic/claude-haiku-4.5"
```

Output: `The passphrase contained in note.txt is: purple-anvil-42`.

So the full path works: the model asked for a tool, the harness confined the path,
ran the tool, appended the result, and sent the conversation back. The model then
answered from the real file contents.

### Path confinement holds against a live model

```sh
rho run "Read /etc/passwd. If that fails say 'blocked as expected', then read
  note.txt and give the passphrase." \
  --provider openrouter --model "anthropic/claude-haiku-4.5"
```

Output:

```
I'll try to read /etc/passwd first, then read note.txt.
Blocked as expected. Now let me read note.txt:
The passphrase is: purple-anvil-42
```

The model tried to leave the session root, the boundary refused, the model was told
why, and the run continued. That is the behaviour we want on both counts.

### `--read-only` blocks a write

```sh
rho run "Create a file called HACKED.txt containing the word pwned." \
  --read-only --provider openrouter --model "anthropic/claude-haiku-4.5"
ls HACKED.txt
```

The model reported that the policy refused the write. `ls` reported
`No such file or directory`. So the file really was not created.

### Errors

A missing key:

```
rho: set the OPENROUTER_API_KEY environment variable to your OpenRouter API key.
```

Exit code 1. The message names the variable to set.

A bad model id:

```
rho: client error: status 400: {"error":{"message":"definitely/not-a-real-model is
not a valid model ID","code":400}}
```

Exit code 1. The provider error is mapped and reported, not swallowed.

## Measurements from the live runs

| What | Value | Command |
| --- | --- | --- |
| Whole run, wall clock, short prompt | 1.34 s, 1.47 s, 1.60 s | `/usr/bin/time -p rho run "Say hi" ...` |
| Peak resident memory, live run with one tool call | 15,515,648 B, 14.8 MiB | `/usr/bin/time -l rho run "Read note.txt ..." ...` |

The wall-clock figure is dominated by the model's own latency, so it is not a
harness measurement. It is recorded because it is what a user feels.

The 14.8 MiB figure is the honest cost of a real streamed turn, against the
8.3 MiB of an idle session in `docs/benchmarks.md`. So a live turn costs about
6.5 MiB more than idle, and that memory is released with the request buffers.

## Three defects the live run found, which 222 passing tests did not

This is the value of stage S9, so it is recorded plainly.

1. **A tool error killed the whole run.** A `ToolError` was sent as a fatal stream
   error rather than returned to the model as an error tool result. So a missing
   file ended the session and the model never learned why. That makes the harness
   unusable, because almost every real session has a tool error.
   Fixed. Guard test: `agent_loop_tool_error_returns_to_the_model_and_the_run_continues`.

2. **`TurnStart` had no matching `TurnEnd` when a turn called a tool.** Every other
   exit path emitted `TurnEnd`. The tool path did not. A frontend pairs the two to
   track state, so `rho-tui` and `rho-acp` would both leak that state.
   Fixed. Guard test: `agent_loop_pairs_every_turn_start_with_a_turn_end`, which
   asserts the invariant across the whole run rather than at one call site.

3. **Turns ran together in `rho run` output.** The last word of one turn touched the
   first word of the next, for example `first.Blocked as expected`. Cosmetic, but
   user-visible on every multi-turn run. Fixed.

Why the suite missed all three: the existing tests asserted the events they
expected to see, and never asserted the **absence** of a bad state or the
**completeness** of a pairing. The two new tests state invariants instead of
examples.

## What is still unverified

- Azure OpenAI, live. No credential available.
- Bedrock with a tool call. Only a plain answer was verified.
- The interactive TUI against a real terminal. Only the headless `run` path was
  driven live. Rendering is covered by tests on a test backend.
- The ACP frontend. `rho-acp` is a stub, and `SPEC-06` defines the mapping.
- Any behaviour on Linux or Windows.
- Long sessions, context compaction, and prompt-cache hit rates.
