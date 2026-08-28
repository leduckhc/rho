# Verification: the JSONL frontend

Step 11 of AGENTS.md. Every command and every line of output below is real. Nothing is
paraphrased. One filter is applied, and only one: the `rho:` skill notices that rho writes
to stderr at startup are removed, because they repeat in every run and say nothing about the
protocol. Where a section shows fewer lines than the run produced, it says so. The client is
`bench/drive-jsonl.sh`, a plain shell script. It writes one JSON command per line to rho's
stdin. It reads one line at a time from rho's stdout. It uses two named pipes, and no bash
feature newer than version 3.2.

Build:

```sh
cargo build --release -p rho-cli
```

Run one case:

```sh
bash bench/drive-jsonl.sh <case> [provider] [model]
```

`-->` is a line the client wrote. `<--` is a line rho wrote.

## What the live runs found

Two defects. Neither one could have been caught by a fixture.

**A swallowed abort, and a run that then never settled.** The serve loop races the command
reader against the run's event stream in a `select!`. `LineReader::next_line` kept the
part-read line in a local variable. So when the event branch won, the dropped future took
those bytes with it. The abort line arrived cut in half, and the run streamed on for ever.
The part-read line now lives in the reader, so the future is cancel safe. The test
`a_dropped_next_line_loses_no_bytes` reproduces it. Before the fix it read five bytes of a
sixteen byte command. After the fix it reads the whole command.

**A rho-core hang on a provider failure.** `crates/rho-core/src/agent.rs` line 569 returns
without calling `queue.unobserve()`. The task that forwards queue announcements then keeps
a clone of the event sender, and the event stream never closes. A frontend that read to
the end of the stream hung. This crate now settles a run when an error arrives, because an
error ends a run. The rho-core defect is **not fixed**. Another worktree owns the steering
queue. See `D-an-error-on-the-event-stream-ends-the-run`.

## 1. A prompt, against Bedrock

```text
=== case: prompt   provider: bedrock   model: global.anthropic.claude-haiku-4-5-20251001-v1:0
=== approval: allow-all (default)
=== binary: ./target/release/rho
--> {"type":"prompt","req_id":"p1","message":"Say the single word: ready. Nothing else."}
<-- {"req_id":"p1","command":"prompt","success":true}
<-- {"type":"turn_start"}
<-- {"type":"text_delta","index":0,"delta":"ready"}
<-- {"type":"turn_end","stop_reason":"end_turn"}
<-- {"type":"settled","stop_reason":"end_turn"}
=== rho exit code: 0
=== stderr:
=== case prompt result: PASS
```

## 2. A steer that lands at a turn boundary

A steer lands only at a turn boundary, so the run needs more than one turn. A tool call
gives it one. The proof is the order. `message_delivered` sits after `tool_end` and before
the next `turn_start`. The model then really says the steered word.

A prompt of one turn does not prove this. It queues the message and settles with the
message still queued. That is correct rho-core behaviour, and it says nothing about
delivery.

```text
=== case: steer   provider: bedrock   model: global.anthropic.claude-haiku-4-5-20251001-v1:0
=== approval: allow-all (default)
=== binary: ./target/release/rho
--> {"type":"prompt","req_id":"p1","message":"Use the bash tool to run: echo first. Then tell me what it printed."}
<-- {"req_id":"p1","command":"prompt","success":true}
<-- {"type":"turn_start"}
<-- {"type":"turn_end","stop_reason":"tool_use"}
<-- {"type":"tool_start","id":"tooluse_939lNw4kaWM6s3OBz88ESa","name":"bash","kind":"execute"}
--> {"type":"steer","req_id":"s1","message":"When you answer, also say the word banana."}
<-- {"req_id":"s1","command":"steer","success":true,"data":{"position":1}}
<-- {"type":"message_queued","position":1}
<-- {"type":"tool_update","id":"tooluse_939lNw4kaWM6s3OBz88ESa","output":"first"}
<-- {"type":"tool_end","id":"tooluse_939lNw4kaWM6s3OBz88ESa","ok":true}
<-- {"type":"message_delivered","count":1}
<-- {"type":"turn_start"}
<-- {"type":"text_delta","index":0,"delta":"The"}
<-- {"type":"text_delta","index":0,"delta":" comman"}
<-- {"type":"text_delta","index":0,"delta":"d printe"}
<-- {"type":"text_delta","index":0,"delta":"d \"first\"."}
<-- {"type":"text_delta","index":0,"delta":"\n\nAlso"}
<-- {"type":"text_delta","index":0,"delta":", banana"}
<-- {"type":"text_delta","index":0,"delta":"."}
<-- {"type":"turn_end","stop_reason":"end_turn"}
<-- {"type":"settled","stop_reason":"end_turn"}
=== rho exit code: 0
=== stderr:
=== case steer result: PASS
```

## 3. An abort that settles as cancelled

```text
=== case: abort   provider: bedrock   model: global.anthropic.claude-haiku-4-5-20251001-v1:0
=== approval: allow-all (default)
=== binary: ./target/release/rho
--> {"type":"prompt","req_id":"p1","message":"Write a very long essay about the sea. At least 800 words."}
<-- {"req_id":"p1","command":"prompt","success":true}
<-- {"type":"turn_start"}
<-- {"type":"text_delta","index":0,"delta":"#"}
--> {"type":"abort","req_id":"a1"}
<-- {"req_id":"a1","command":"abort","success":true,"data":{"running":true}}
<-- {"type":"turn_end","stop_reason":"canceled"}
<-- {"type":"settled","stop_reason":"cancelled"}
=== rho exit code: 0
=== stderr:
=== case abort result: PASS
```

Note the two spellings. `turn_end` carries `canceled`, from `rho_core::StopReason`.
`settled` carries `cancelled`, from `rho_core::AgentStopReason`. That enum has a serde
rename, so its value matches ACP. See `D-acp-cancelled-spelling`. A test pins both
spellings, so neither side can drift.

## 4. The same prompt twice, down one session

Twice has caught two defects in this project. Here it proves two things. A prompt sent
after `settled` is accepted. The conversation then carries both turns.

```text
=== case: twice   provider: bedrock   model: global.anthropic.claude-haiku-4-5-20251001-v1:0
=== approval: allow-all (default)
=== binary: ./target/release/rho
--> {"type":"prompt","req_id":"p1","message":"Say only: one"}
<-- {"req_id":"p1","command":"prompt","success":true}
<-- {"type":"turn_start"}
<-- {"type":"text_delta","index":0,"delta":"one"}
<-- {"type":"turn_end","stop_reason":"end_turn"}
<-- {"type":"settled","stop_reason":"end_turn"}
--> {"type":"prompt","req_id":"p2","message":"Say only: two"}
<-- {"req_id":"p2","command":"prompt","success":true}
<-- {"type":"turn_start"}
<-- {"type":"text_delta","index":0,"delta":"two"}
<-- {"type":"turn_end","stop_reason":"end_turn"}
<-- {"type":"settled","stop_reason":"end_turn"}
--> {"type":"get_messages","req_id":"g1"}
<-- {"req_id":"g1","command":"get_messages","success":true,"data":{"messages":[{"content":[{"text":"Say only: one","type":"text"}],"role":"user"},{"content":[{"text":"one","type":"text"}],"role":"assistan...
=== rho exit code: 0
=== stderr:
=== case twice result: PASS
```

## 5. The failure paths, including the same failure twice

```text
=== case: errors   provider: bedrock   model: global.anthropic.claude-haiku-4-5-20251001-v1:0
=== approval: allow-all (default)
=== binary: ./target/release/rho
--> {"type":"compact","req_id":"e1"}
<-- {"command":"unknown","success":false,"error":"unknown_command","message":"unknown variant `compact`, expected one of `prompt`, `steer`, `abort`, `get_state`, `set_model`, `new_session`, `get_messages`...
--> {not json
<-- {"command":"unknown","success":false,"error":"parse_error","message":"key must be a string at line 1 column 2"}
--> {"type":"prompt","message":"hi","only_if_cheap":true}
<-- {"command":"unknown","success":false,"error":"parse_error","message":"unknown field `only_if_cheap`, expected `req_id` or `message`"}
--> {"type":"set_model","req_id":"e4","provider":"nosuchprovider","model_id":"x"}
<-- {"req_id":"e4","command":"set_model","success":false,"error":"unknown_provider","message":"no provider is called nosuchprovider"}
--> {"type":"set_model","req_id":"e5","provider":"nosuchprovider","model_id":"x"}
<-- {"req_id":"e5","command":"set_model","success":false,"error":"unknown_provider","message":"no provider is called nosuchprovider"}
--> {"type":"get_state","req_id":"e6"}
<-- {"req_id":"e6","command":"get_state","success":true,"data":{"model_id":"global.anthropic.claude-haiku-4-5-20251001-v1:0","provider":"bedrock","running":false}}
=== rho exit code: 0
=== stderr:
=== case errors result: PASS
```

## 6. A dialog that times out, and the timeout denies

`RHO_APPROVAL=ask` turns the approval gate on. Before this frontend, `ask` had no answer in
any headless build, and rho refused to start with it.

This client answers nothing at all. The agent side owns the timeout. After 30 seconds the
dialog resolves as `cancelled`, which denies the tool. The run then continues and settles.

The transcript is one whole run. Only the `rho:` skill notices on stderr are removed, and
nothing else is cut, so the prompt reply and the streamed answer are both here. This run
asked about 2 tool(s) and reported 2 tool end(s). How many tools a model tries after a
denial is the model's own choice, so the count is not a property this case asserts. The
repeated-failure evidence is elsewhere: section 5 sends the same bad provider twice, section
4 sends the same prompt twice, and
`a_client_that_answers_no_dialog_denies_the_tool_and_the_run_continues` pins the behaviour
with no model in the loop.

```text
=== case: dialog   provider: bedrock   model: global.anthropic.claude-haiku-4-5-20251001-v1:0
=== approval: ask
=== binary: ./target/release/rho
--> {"type":"prompt","req_id":"p1","message":"Create a file called note.txt containing the word hello. Use your tools."}
<-- {"req_id":"p1","command":"prompt","success":true}
<-- {"type":"turn_start"}
<-- {"type":"turn_end","stop_reason":"tool_use"}
<-- {"type":"dialog","method":"confirm","id":"d0","title":"Allow the tool write?","message":"The agent wants to run write, which is a file edit operation.","timeout_ms":30000}
    (answering nothing on purpose: the agent-side timeout must decide)
<-- {"type":"tool_end","id":"tooluse_BUL40vQniICFrp1IUgE23T","ok":false}
<-- {"type":"turn_start"}
<-- {"type":"text_delta","index":0,"delta":"I can't"}
<-- {"type":"text_delta","index":0,"delta":" write"}
<-- {"type":"text_delta","index":0,"delta":" files"}
<-- {"type":"text_delta","index":0,"delta":" directly"}
<-- {"type":"text_delta","index":0,"delta":" due"}
<-- {"type":"text_delta","index":0,"delta":" to the current"}
<-- {"type":"text_delta","index":0,"delta":" approval policy. However, I can show"}
<-- {"type":"text_delta","index":0,"delta":" you how to create the file using the"}
<-- {"type":"text_delta","index":0,"delta":" bash"}
<-- {"type":"text_delta","index":0,"delta":" tool:"}
<-- {"type":"turn_end","stop_reason":"tool_use"}
<-- {"type":"dialog","method":"confirm","id":"d1","title":"Allow the tool bash?","message":"The agent wants to run bash, which is a command operation.","timeout_ms":30000}
<-- {"type":"tool_end","id":"tooluse_GMCjACYKZD74FE4IUOMl5p","ok":false}
<-- {"type":"turn_start"}
<-- {"type":"text_delta","index":0,"delta":"The current"}
<-- {"type":"text_delta","index":0,"delta":" approval policy prevents me from writing"}
<-- {"type":"text_delta","index":0,"delta":" files or"}
<-- {"type":"text_delta","index":0,"delta":" running"}
<-- {"type":"text_delta","index":0,"delta":" bash"}
<-- {"type":"text_delta","index":0,"delta":" commands. To"}
<-- {"type":"text_delta","index":0,"delta":" create the file, you"}
<-- {"type":"text_delta","index":0,"delta":"'ll"}
<-- {"type":"text_delta","index":0,"delta":" need to either"}
<-- {"type":"text_delta","index":0,"delta":":\n\n1."}
<-- {"type":"text_delta","index":0,"delta":" Adjust"}
<-- {"type":"text_delta","index":0,"delta":" the approval policy to allow the"}
<-- {"type":"text_delta","index":0,"delta":" `"}
<-- {"type":"text_delta","index":0,"delta":"write` or"}
<-- {"type":"text_delta","index":0,"delta":" `bash"}
<-- {"type":"text_delta","index":0,"delta":"` tools"}
<-- {"type":"text_delta","index":0,"delta":"\n2. Create"}
<-- {"type":"text_delta","index":0,"delta":" the file manually with"}
<-- {"type":"text_delta","index":0,"delta":":"}
<-- {"type":"text_delta","index":0,"delta":" `echo hello > note.txt`"}
<-- {"type":"turn_end","stop_reason":"end_turn"}
<-- {"type":"settled","stop_reason":"end_turn"}
=== rho exit code: 0
=== stderr:
=== case dialog result: PASS
```

## 7. The same dialog, answered

The client reads the id rho minted and echoes it back. The tool then runs. This transcript
is whole on the same terms as section 6.

```text
=== case: dialog-yes   provider: bedrock   model: global.anthropic.claude-haiku-4-5-20251001-v1:0
=== approval: ask
=== binary: ./target/release/rho
--> {"type":"prompt","req_id":"p1","message":"Use the bash tool to run: echo approved"}
<-- {"req_id":"p1","command":"prompt","success":true}
<-- {"type":"turn_start"}
<-- {"type":"turn_end","stop_reason":"tool_use"}
<-- {"type":"dialog","method":"confirm","id":"d0","title":"Allow the tool bash?","message":"The agent wants to run bash, which is a command operation.","timeout_ms":30000}
    (dialog id: d0)
--> {"type":"dialog_response","id":"d0","answer":{"confirmed":true}}
<-- {"command":"dialog_response","success":true,"data":{"delivered":true}}
<-- {"type":"tool_start","id":"tooluse_cSSkLeOTXmoCnTIlnHgQzF","name":"bash","kind":"execute"}
<-- {"type":"tool_update","id":"tooluse_cSSkLeOTXmoCnTIlnHgQzF","output":"approved"}
<-- {"type":"tool_end","id":"tooluse_cSSkLeOTXmoCnTIlnHgQzF","ok":true}
<-- {"type":"turn_start"}
<-- {"type":"text_delta","index":0,"delta":"Done"}
<-- {"type":"text_delta","index":0,"delta":"."}
<-- {"type":"text_delta","index":0,"delta":" Output"}
<-- {"type":"text_delta","index":0,"delta":":"}
<-- {"type":"text_delta","index":0,"delta":" `"}
<-- {"type":"text_delta","index":0,"delta":"approved`"}
<-- {"type":"turn_end","stop_reason":"end_turn"}
<-- {"type":"settled","stop_reason":"end_turn"}
=== rho exit code: 0
=== stderr:
=== case dialog-yes result: PASS
```

## 8. A second provider

One provider is not every provider.

```text
=== case: prompt   provider: openrouter   model: anthropic/claude-haiku-4.5
=== approval: allow-all (default)
=== binary: ./target/release/rho
--> {"type":"prompt","req_id":"p1","message":"Say the single word: ready. Nothing else."}
<-- {"req_id":"p1","command":"prompt","success":true}
<-- {"type":"turn_start"}
<-- {"type":"text_delta","index":0,"delta":"ready"}
<-- {"type":"text_delta","index":0,"delta":""}
<-- {"type":"text_delta","index":0,"delta":""}
<-- {"type":"turn_end","stop_reason":"end_turn"}
<-- {"type":"settled","stop_reason":"end_turn"}
=== rho exit code: 0
=== stderr:
=== case prompt result: PASS
```

The steer case also passes on OpenRouter, with the same delivery order:

```text
--> {"type":"steer","req_id":"s1","message":"When you answer, also say the word banana."}
<-- {"type":"message_queued","position":1}
<-- {"type":"tool_end","id":"toolu_bdrk_01N7sTYH8RNwJv4rsUqvTLRt","ok":true}
<-- {"type":"message_delivered","count":1}
<-- {"type":"settled","stop_reason":"end_turn"}
=== case steer result: PASS
```

## 9. An abort while a dialog is open

Added after review. A dialog blocks the approval gate, and `rho_core` awaits that gate with
no cancel arm of its own. So an abort had to end the dialog wait inside this crate, or the
run would sit until the dialog timed out.

The dialog timeout here is 30 seconds, and the client's own read bound is 15. So this case
cannot pass by waiting: if the abort does not end the wait, the read times out and the case
fails. The whole run took **3.3 seconds** of wall clock.

```text
=== case: dialog-abort   provider: bedrock   model: global.anthropic.claude-haiku-4-5-20251001-v1:0
=== approval: ask
=== binary: ./target/release/rho
--> {"type":"prompt","req_id":"p1","message":"Create a file called note.txt with the word hello. Use your tools."}
<-- {"req_id":"p1","command":"prompt","success":true}
<-- {"type":"turn_start"}
<-- {"type":"text_delta","index":0,"delta":"I'll create a file called note."}
<-- {"type":"text_delta","index":0,"delta":"txt with the word \"hello\" in"}
<-- {"type":"text_delta","index":0,"delta":" it."}
<-- {"type":"turn_end","stop_reason":"tool_use"}
<-- {"type":"dialog","method":"confirm","id":"d0","title":"Allow the tool write?","message":"The agent wants to run write, which is a file edit operation.","timeout_ms":30000}
    (aborting instead of answering: the run must settle without waiting 30s)
--> {"type":"abort","req_id":"a1"}
<-- {"req_id":"a1","command":"abort","success":true,"data":{"running":true}}
<-- {"type":"tool_end","id":"tooluse_pw7V5QsDxZvgXfx2TMGK4K","ok":false}
<-- {"type":"turn_start"}
<-- {"type":"turn_end","stop_reason":"canceled"}
<-- {"type":"settled","stop_reason":"cancelled"}
=== rho exit code: 0
=== stderr:
=== case dialog-abort result: PASS
```

## 10. The line cap, twice

Added after review. Two lines past the 1 MiB cap must draw two replies, because the
protocol promises one reply per command line, and a good command after them must still work.
The message names the cap and the bytes read.

```text
=== case: cap   provider: bedrock   model: global.anthropic.claude-haiku-4-5-20251001-v1:0
=== approval: allow-all (default)
=== binary: ./target/release/rho
--> {"type":"prompt","message":"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx...
<-- {"command":"unknown","success":false,"error":"line_too_long","message":"a command line passed the 1048576 byte cap a...
--> {"type":"prompt","message":"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx...
<-- {"command":"unknown","success":false,"error":"line_too_long","message":"a command line passed the 1048576 byte cap a...
--> {"type":"get_state","req_id":"c1"}
<-- {"req_id":"c1","command":"get_state","success":true,"data":{"model_id":"global.anthropic.claude-haiku-4-5-20251001-v...
=== rho exit code: 0
=== stderr:
=== case cap result: PASS
```

## What is not verified here

- Azure. This lane did not drive it. The frontend links no provider, so it holds no
  provider-specific code. That is an argument, and not a measurement.
- A client in another language. The shell client shows the wire is plain enough. Nobody
  wrote a Python or a Node client.
- Session recording. Another lane owns the session store wiring, so a JSONL run writes no
  session file yet.
