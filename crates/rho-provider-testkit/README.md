# rho-provider-testkit

Prove that your `rho_core::Provider` conforms, using the same assertions rho uses
for its own three providers.

You do not need to fork rho, and you do not need to copy a test file.

## Why this crate exists

rho's claim is that a third party writes a provider without forking. A third party
cannot run a test file that is private to `rho-core`. So the assertions live here, in
a normal crate you can depend on. See decision D-010.

## Use it

Add it as a dev dependency, next to `rho-core`.

```sh
cargo add rho-core
cargo add --dev rho-provider-testkit tokio --features tokio/full
```

Write one bridge from rho's abstract `Script` to your provider's own wire format.
That bridge is the only glue the testkit asks for.

```rust,ignore
use std::any::Any;
use async_trait::async_trait;
use rho_core::{CancelToken, CompletionRequest, Provider};
use rho_provider_testkit::{
    HarnessRun, ProviderHarness, SCRIPT_TEXT, SCRIPT_TOOL_NAME, Script, run_all,
    script_tool_arguments,
};

struct MyHarness;

#[async_trait]
impl ProviderHarness for MyHarness {
    async fn run(&self, script: Script) -> HarnessRun {
        // Serve `script` from a mock server or a recorded fixture. Never a network
        // call. Then start your provider against it.
        let stream = my_provider().stream(request(), CancelToken::new()).await.unwrap();
        HarnessRun { stream, guard: Box::new(()) as Box<dyn Any + Send> }
    }
}

#[tokio::test]
async fn my_provider_conforms() {
    run_all(&MyHarness).await;
}
```

## What the scripts mean

| `Script` | What your harness must produce |
| --- | --- |
| `Text` | The text `SCRIPT_TEXT`, split into two or more deltas. |
| `ToolCall` | One call to `SCRIPT_TOOL_NAME`, split across three or more chunks, with the JSON arguments split mid-token. The parsed result must equal `script_tool_arguments()`. |
| `Usage` | A text answer that also reports token usage. |
| `Gated` | A text answer whose transport sends the first chunk, then holds the rest back until the test observes an event. A provider that buffers the whole body deadlocks here and fails. |

`StagedHttpServer` implements the `Gated` behaviour for you, if your provider speaks
HTTP.

## What it checks

`run_all` runs every check. Each one is also public, so you can run one at a time
while you work.

- `provider_contract_emits_message_start_first`
- `provider_contract_emits_done_last`
- `provider_contract_text_deltas_in_order`
- `provider_contract_tool_call_end_has_parsed_arguments`
- `provider_contract_yields_first_event_before_stream_end`

A failure names the exact violation. For example, a provider that omits its first
event reports:

```
the first event must be MessageStart, but it was TextStart { index: 0 }
```

## Licence

MIT.
