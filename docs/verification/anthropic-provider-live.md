# Verification: rho-provider-anthropic driven for real

Date 20260901. Commit built from `80cc10e`, plus the empty-frame fix that came out of the
first live drive.

## The drive that answered

```
cargo run --release -p rho-provider-anthropic --example live_drive -- \
    "Reply with exactly one word: pomegranate"
```

Configuration:

- `base_url = http://127.0.0.1:58788/dev1/anthropic` (xdent tunnel, `azure-claude` route)
- `model    = claude-sonnet-4-6`
- credential is a dummy string, because the proxy holds the real credential

Output:

```
EVENT MessageStart { role: Assistant }
EVENT TextStart { index: 0 }
EVENT TextDelta { index: 0, delta: "pom" }
EVENT TextDelta { index: 0, delta: "egranate" }
EVENT TextEnd { index: 0 }
EVENT Done { stop_reason: EndTurn }
elapsed = 7.267457416s
answer  = "pomegranate"
```

Six normalised events, one-word answer, no error.

## The defect the drive found

The first drive returned the six events above and then errored:

```
stream error: stream decode error: anthropic message event did not parse:
    expected value at line 1 column 2
```

The event name was `message`, and the data was empty. Neither the wiremock test nor the
decoder tests could see it, because both drove hand-written SSE that ended cleanly. The SSE
spec sets the default event name to `message` when the wire has no `event:` line, and the
proxy sent an empty trailing frame under that name.

The fix is defensive at three points: skip an empty event name; skip an empty data body;
skip a `message`-named event, because rho does not use the default name.

An earlier decision on this project already forbids swallowing an unknown **content-bearing**
event. The rule this fix adds does not weaken that: the three skipped cases are framing, not
content, and the launch amendment on `SPEC-anthropic-messages-provider` named `ping` in the
same class. A future decision may name `message` explicitly in the spec.

## Tests, without the tunnel

Thirteen tests green in the crate, four of them wiremock integration tests that drive the
full stack against a local mock:

```
tests/identity.rs         1 pass
tests/request_body.rs     3 pass
tests/decoder.rs          5 pass
tests/wiremock_flow.rs    4 pass
```

The wiremock tests cover the four boundaries the tunnel exercised: the request headers, the
happy SSE flow, an empty credential never reaching the network, and the 401 and 503 status
mappings. The tunnel drive proved they were sufficient, once the empty-frame case was fixed.

## Tool calls, driven for real

Same tunnel and model. One prompt asks for a file read, the other asks for two.

```
$ echo "the anthropic pass phrase is turquoise" > fact.txt
$ rho run "Read fact.txt and reply with only the pass phrase" \
    --provider anthropic \
    --base-url http://127.0.0.1:58788/dev1/anthropic \
    --model claude-sonnet-4-6 --no-skills --no-agents
turquoise
    4.12 real
```

Two calls in one assistant message. This is the sprint-1 killer defect on Bedrock, and it
does not reproduce here.

```
$ echo "alpha line" > a.txt && echo "beta line" > b.txt
$ rho run "Read a.txt and b.txt, then reply with both first lines separated by a comma" \
    --provider anthropic \
    --base-url http://127.0.0.1:58788/dev1/anthropic \
    --model claude-sonnet-4-6 --no-skills --no-agents
The first lines are:
**alpha line**, **beta line**
    4.88 real
```

The session file confirms the shape:

```
assistant message: content types = ['text', 'tool_call', 'tool_call']
                   tool_call names = ['read', 'read']
```
