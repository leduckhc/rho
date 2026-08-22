# Verification — tool result handle

Feature: F-tool-result-handle. Spec: `SPEC-tool-result-handle`.
Date: 22 August 2026. Host: macOS on Apple Silicon.
Provider: `openrouter`, model `anthropic/claude-haiku-4.5` (the provider default).

Every command below was run. Every output below is real, trimmed only of the two startup
notices about skills and the default model.

## The run that nearly became a false proof

This is the most useful record on this page, so it comes first.

A 2.6 MB log was written with one marker line among 80,000. The prompt asked the model to
`cat` the file and report the odd status code.

```sh
./target/release/rho run "Run 'cat big.log' and then tell me the exact status code on the \
one line that is not status=ok." --root "$HOME/rho-verify-trh" --provider openrouter
```

```text
I'll run the cat command to check the contents of big.log and find the non-ok status code.
The output is large. Let me search for the non-ok status line:
Let me use grep to find the line that doesn't have status=ok:
The exact status code on the one line that is not status=ok is: **FATAL-MARKER-9f3a**
```

The answer is correct, and it proves nothing about this feature. The model ran `grep` on the
file. The arithmetic says why:

```sh
python3 -c "
data = open('big.log','rb').read()
at = data.find(b'FATAL-MARKER-9f3a')
print('marker at byte', at)
print('inside bash 100000 cap?', at < 100000)"
```

```text
marker at byte 1308924
inside bash 100000 cap? False
```

`bash` caps its own output at 100,000 bytes before it returns, so the store never held the
marker. Recording this run as a pass would have been a false claim. See
D-bash-cap-limits-the-store, which this run produced.

## The run that does prove it

The scenario has to force the path. The output must be under `bash`'s own cap, the answer must
sit past the 4,096-byte preview, and there must be no file for `grep` to shortcut through.

A command that prints 5,000 records, one of them odd:

```text
marker at byte 70890 of 73901
inside the 4096-byte preview? False
inside bash 100000 cap?      True
```

```sh
./target/release/rho run "Run exactly this command: seq 1 5000 | awk '{ if (\$1==4800) \
print \"record \" \$1 \" TOKEN-Q7x2\"; else print \"record \" \$1 \" ok\" }'
Then find the token on the one record that is not 'ok'.
In your final answer report three things: the token, the exact name of every tool you called, \
and the exact handle string you passed to any tool that took one." \
  --root "$HOME/rho-verify-trh" --provider openrouter
```

```text
I'll run that command and find the token.
Now I'll search the result for the record that contains "TOKEN-" (not "ok"):
Perfect! I found the token.

**Final Answer:**

- **Token:** TOKEN-Q7x2
- **Tools called:** bash, read_tool_result
- **Handle passed:** tr-9530ad22d0596c49-000001
```

This is the proof, and the handle is why. `tr-9530ad22d0596c49-000001` carries a nonce this
process generated at random. The model cannot invent it, so it can only have read it from a
preview block rho put in the context. The token sits at byte 70,890, far past the 4,096-byte
preview, so it can only have come from `read_tool_result`.

## The same thing twice

```text
**Token:** `TOKEN-Q7x2`
**Handle:** `tr-fa51c2f6260ec67a-000001`
```

Correct again, and the nonce is different. That is the resume isolation of spec section 4,
observed rather than argued: a second session cannot name the first session's handles.

## The failure paths

One prompt, two bad handles, no other tool.

```sh
./target/release/rho run "Call the read_tool_result tool twice, and report the exact error \
message each time. First call it with handle set to '../../etc/passwd'. Then call it with \
handle set to 'tr-0123456789abcdef-000042'. Do not run any other tool." \
  --root "$HOME/rho-verify-trh" --provider openrouter
```

```text
1. First call (`../../etc/passwd`):
   > invalid arguments: "../../etc/passwd" is not a result handle. A handle looks like
   > tr-0123456789abcdef-000001 and must be copied exactly from a tool_result_preview block.

2. Second call (`tr-0123456789abcdef-000042`):
   > invalid arguments: No stored result has handle tr-0123456789abcdef-000042. A handle
   > belongs to the session that made it, and it must be copied exactly from a
   > tool_result_preview block in this conversation.
```

The path traversal read no file and raised no panic. The well-formed handle from no session
reported `NotFound`, which says nothing about what exists on disk. The session continued after
both.

## Re-verified after the review fixes

A security review found that a subagent inherited no result policy, and that `put` left a
temporary file behind on a rename failure. Both were fixed, and the whole path was re-run
against the new binary.

```text
- **Token**: `TOKEN-Q7x2`
- **Record**: record 4800
- **Handle**: `tr-79f1b19a79421ecd-000001`

The token was found at byte offset 70890 in the stored result.
```

A third nonce, and the byte offset it names is exactly where the marker sits. A passing suite
was not evidence that the fixes preserved the product, so this run is.

## What is not verified here

- **A whole `bash` output over 100,000 bytes is not reachable.** `bash` cuts first. See
  D-bash-cap-limits-the-store and `F-bash-streams-to-the-store`.
- **A store failure was tested only in `rho-core`**, by `a_store_failure_keeps_the_cap`. No
  live run filled a disk.
- **The ten-megabyte cap was tested only in `rho-core`**, by
  `the_cap_applies_to_a_tool_that_does_not_bound_itself`. Reaching it live needs an MCP or
  plugin peer that returns ten megabytes, and rho ships none.
- **Only `openrouter` was driven.** The cap runs before any provider sees the message, so the
  feature is provider-independent by construction. That is an argument, not a measurement.
