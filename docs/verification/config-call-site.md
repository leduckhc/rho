# Verification — the config call site, 20260820

Branch `feat/reasoning-across-providers`. Binary built with
`cargo build --release -p rho-cli`. Provider: Bedrock, live, account `980637428984`.

Every command ran against a scratch home and a scratch project, so no real `~/.config/rho`
or `~/.rho` was read:

```sh
mkdir -p /tmp/rho-v/home/rho /tmp/rho-v/proj
export HOME=/tmp/rho-v/home XDG_CONFIG_HOME=/tmp/rho-v/home
```

The global file for the runs below:

```toml
model = "us.anthropic.claude-haiku-4-5-20251001-v1:0"
provider = "bedrock"
tui-reasoning = "full"
```

## What passed

| # | What it proves | Command | Result |
| --- | --- | --- | --- |
| 1 | a config file alone drives a real turn | `rho run "Reply with exactly: FILE-OK"` | `FILE-OK`, exit 0 |
| 2 | the same run twice behaves the same | the same command, twice | `TWICE-1`, `TWICE-2`, exit 0 both |
| 3 | a broken project file stops the run | `.rho/config.toml` = `this is not toml =` | exit 1, and the message names `/private/tmp/rho-v/proj/.rho/config.toml` |
| 4 | a bad mode is refused, naming the flag | `rho run "hi" --reasoning loud` | exit 1, `the --reasoning flag is wrong: unknown reasoning mode "loud"` |
| 5 | a bad mode in a file is refused, naming the key | `tui-reasoning = "loud"` | exit 1, and the message names `tui-reasoning` |
| 6 | an untrusted project command never runs | `[credentials] bedrock = "!touch /tmp/rho-trust-probe-live"` | the marker file was **absent** |
| 8 | `RHO_MODEL` still reaches the product | `RHO_MODEL=... RHO_PROVIDER=bedrock rho run ...` | `ENV-OK`, exit 0 |

Run 1 is the defect this branch opened with. Before this change a config file changed
nothing, because no binary called `Config::load`.

Run 8 is a regression guard. Dropping the clap `env` attribute from `--model` made
`RHO_MODEL` dead, and all 874 tests still passed. Layer 5 now carries it.

## What the run found

**Run 6 held for the wrong reason, and run 7 proves it.** With `--trust-project` the same
command still did not run:

```sh
rho run "hi" --trust-project     # marker file still absent
```

A trusted command credential must run, so the gate looked like a wall. The cause is not the
gate. **No provider resolves a credential through `Config::resolve_credential` yet.**
`provider.rs` still reads `std::env::var(...).unwrap_or_default()` in five places, which is
item I15 of the task notes and is not built.

So today:

- The `skill-paths` and `mcp-config` half of the gate **is** reachable, because the merge
  drops those keys before anything reads them.
- The command-credential half **is not** reachable, because a `credentials` block in a config
  file reaches no provider at all. The unit test proves the refusal, and nothing in production
  asks for it.

That also means the live probe's execution path is not reachable through a config file today.
It becomes reachable the moment I15 lands, so I15 must land with its gate already wired.

**A malformed message, found by reading run 5's output.** The refusal reads:

```
rho: cannot parse the config file the merged configuration: the tui-reasoning key value "loud" is not valid
```

"the config file the merged configuration" is not a sentence. `ConfigError::Parse` always
prints `cannot parse the config file {path}`, and `parse_reasoning` passes the string
`the merged configuration` as a fake path, because after the merge no real path is known.

It is left unfixed on purpose. A clean fix adds an error variant for a bad value that has no
file, and the error taxonomy is part of a contract that is already reviewed. See
`SPEC-config-call-site` section 2. The message names the key and the value, so it is wrong
prose rather than a wrong answer.

## The merge completeness guard, added later

`SPEC-config-call-site` promised `every_scalar_key_merges_and_reaches_the_config`, and no
test carried the name. `bench/check-spec-tests.py` found the gap.

`ConfigLayer::merge` assigns one field per line, by hand. Fifteen lines, and no test read
more than one key. So a forgotten line dropped that value in silence.

The new test writes one config file that sets every key. It merges the file over an empty
layer. Then it sweeps the merged layer for a single unset field, and it reads the sweep from
the Debug text. A hand-kept list of fields is the defect itself, so the assertion holds no
list. It also loads the file through `Config::load`, to prove each value reaches the
resolved config and not only the layer.

Three deliberate breaks, each restored from a copy in `/tmp` and never with `git checkout`:

| Break | Result |
| --- | --- |
| Delete `self.mcp_config = over.mcp_config.or(self.mcp_config);` | FAILED. `ConfigLayer::merge dropped a key it must carry: ... mcp_config: None ...` |
| Merge the wrong field: `self.model = over.provider.or(self.model)` | FAILED. `left: Some("openrouter")`, `right: Some("a-model")` |
| Delete `tui-mouse = true` from the fixture, which is a new field in miniature | FAILED. `EVERY_KEY must set every key of ConfigLayer, and it leaves one unset: ... tui_mouse: None ...` |

The command:

```sh
cargo test -p rho-config --test merge_order
```

It reports `6 passed; 0 failed` with the file restored.

The second break matters most. A sweep for `None` alone would pass it, because a wrongly
assigned field is still set. The load half is what catches it.

## Not covered here

- Azure and OpenRouter. Only Bedrock was exercised, and one provider is not every provider.
- The TUI. Only `rho run` was driven, so `tui-mouse` and `live` reasoning were proved by test
  and not by eye.
- Behaviour rules 4, 7, and 8 of the spec, the stderr announcements. They stay unbuilt while
  question U4 is open, because `Config` has no field to carry a notice.
