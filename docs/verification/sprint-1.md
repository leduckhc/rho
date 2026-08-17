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
| Streamed answer | Azure OpenAI | **pass** |
| Tool call, end to end | Azure OpenAI | **pass**, after a fix. See below. |
| Two tool calls in one turn | Azure OpenAI | **pass**, after the same fix |
| `--read-only` blocks a write | Azure OpenAI | **pass** |
| Path confinement holds | Azure OpenAI | **pass** |
| Tool call, end to end | OpenRouter | **pass** |
| Two tool calls in one turn | OpenRouter | **pass** |
| Tool call, end to end | AWS Bedrock | **pass** |
| Two tool calls in one turn | AWS Bedrock | **pass**, after a fix. See below. |
| `--read-only` blocks a write | AWS Bedrock | **pass** |
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

## The Bedrock parallel tool-call defect

A second live pass closed the gap this document previously recorded as
"Bedrock with a tool call, only a plain answer was verified". Closing it found a
real bug.

**Symptom.** One tool call worked. Two tool calls in one turn returned HTTP 400,
`Bedrock rejected the request as invalid`.

**Cause.** Converse requires strictly alternating roles. `rho-core` records one
`Role::Tool` message per tool result, which is right for its own model. Bedrock has
no tool role, so every result mapped to `user`. Two results therefore produced two
consecutive user messages, and the service refused the request. The built list was
`[User, Assistant, User, User]`.

**Fix.** `build_messages` now merges a run of messages that share a role into one
message. That is also what Bedrock wants: every tool result for one turn belongs in
a single user message.

**Why the tests missed it.** Every Bedrock fixture describes a *response*. This
defect was in the *request*. So no recorded fixture could have caught it, and the
single-call live check passed because one result cannot produce a consecutive pair.

**Guards added**, all offline:

- `build_messages_never_emits_two_messages_with_the_same_role_in_a_row`
- `build_messages_merges_tool_results_into_one_user_message`, which also asserts both
  results survive the merge in order, because silently dropping one would be worse
  than the 400
- `build_messages_keeps_a_single_tool_result_working`

**Verified live after the fix**, reading two files in one turn:

```
The canary value is: teal-lantern-77
The list.txt file has 3 lines.
```

OpenRouter was checked for the same case and was already correct.

## The Azure tool-call defect

**Symptom.** A plain answer worked. Any tool call returned HTTP 400:

```
Invalid value: 'tool'. Supported values are: 'assistant', 'system', 'developer',
and 'user'.
```

**Cause, and it was two bugs.** The Responses API has no `tool` role. It mixes
messages and typed items in one `input` array. A tool result is a
`function_call_output` item, not a message. The provider sent a message with
`role: "tool"`.

The second bug hid behind the first. The provider dropped the assistant's tool calls
entirely, with a comment saying replay was out of scope. But Responses requires a
`function_call` item to appear before its own `function_call_output`, with matching
`call_id` values. So even a corrected output item would have referenced a call the
service had never seen.

**Fix.** One normalised message now maps to **one or more** input items, because an
assistant turn with text and two tool calls is three items. Two details are easy to
miss and each costs a 400: `arguments` is a JSON **string**, not an object, and the
call must precede its output.

**Why the tests missed it.** Every Azure fixture describes a *response*. Both defects
were in the *request*. This is the same lesson as Bedrock, met twice.

**Guards added**, all offline:

- `build_request_body_never_sends_the_tool_role`
- `build_request_body_sends_tool_results_as_function_call_output_items`
- `build_request_body_replays_the_assistant_tool_calls`, which also asserts the
  arguments are a JSON string and that a call precedes its output
- `build_request_body_keeps_plain_text_messages_as_messages`

**Verified live after the fix.** Single and parallel tool calls both work.
`--read-only` refused a write and the file was not created. A read of `/etc/passwd`
was refused and the model recovered.

## What is still unverified
- The interactive TUI against a real terminal. Only the headless `run` path was
  driven live. Rendering is covered by tests on a test backend.
- The ACP frontend. `rho-acp` is a stub, and `SPEC-06` defines the mapping.
- Any behaviour on Linux or Windows.
- Long sessions, context compaction, and prompt-cache hit rates.
