# Verification: the review fixes of 20260831

A seven-lens review of pull requests #4 to #16 found ten defects. This page records the three
that are fixed, what proved each one, and the seven that are open. Every command here ran, and
every number here is measured.

The gate ran in an isolated worktree, because a second writer switched the branch of the main
checkout while the first gate was running. A gate that spans a branch change proves nothing.

```sh
git worktree add --detach /tmp/rho-verify 0f1ee80
cd /tmp/rho-verify && CARGO_TARGET_DIR=/tmp/rho-verify-target bash gate.sh
```

Result: 2084 tests pass, 0 fail. Every checker reports `VIOLATIONS 0`. The workspace held 2074
tests before these fixes, and the three fixes add ten.

## 1. An empty signature is no signature

`replay_block` read the `signature` key with `Value::as_str`. A key holding `""` answers
`Some("")`, so a reasoning block travelled to Bedrock with an empty signature, which rejects the
whole turn. The refusal was not reported either.

The test beside it claimed this case was covered. It pinned the absent key and never the empty
value, and its doc comment named behaviour the code did not have.

Proved by a probe against the public API, before and after:

```
before  [empty-string] reasoning_blocks_sent=1 signature_on_wire=Some("") dropped_replays=[]
after   [empty-string] reasoning_blocks_sent=0 signature_on_wire=None     dropped_replays=[NoSignature]
```

Mutations:

| break | test that failed |
| --- | --- |
| the whole filter removed | `a_state_with_an_empty_signature_is_dropped`, `a_state_with_a_blank_signature_is_dropped` |
| `trim` removed from the filter | `a_state_with_a_blank_signature_is_dropped` alone |

The second row is why the blank test exists. See `D-an-empty-signature-is-no-signature`.

**Not driven against live Bedrock.** No credentials were available in this session. The wire
shape is proved by the SDK types and the probe, and a live call remains the stronger test.

## 2. A config file is read under a cap

`Config::read_file` used `std::fs::read_to_string`, which has no bound. A project file arrives
with a clone, so its size is chosen by whoever wrote the repository. The read also happens before
the trust gate, because the gate reads the parsed file to know what to strip.

Measured with the release binary:

```sh
cd /tmp/bigcfg && /usr/bin/time -l rho run "hi" --provider bedrock --model x
```

| `.rho/config.toml` | peak RSS before | peak RSS after |
| --- | --- | --- |
| 400 MB | 428 MB | not re-run |
| 800 MB | 848 MB | 10 MB |

After the fix the run refuses and names both the file and the limit:

```
rho: the config file /private/tmp/bigcfg/.rho/config.toml is larger than 1048576 bytes
```

Driven twice, as the failure path requires. Peak memory was 10.0 MB and 10.0 MB.

Mutations:

| break | test that failed |
| --- | --- |
| the `take` bound deleted, the length check kept | `a_source_with_no_end_is_refused_and_the_read_ends` **alone** |
| `>` changed to `>=` | `a_file_at_the_cap_is_accepted` alone |

**The first row is the important one.** With the bound deleted,
`a_file_over_the_cap_is_refused_and_names_the_limit` stayed green. A test that asserts the
refusal cannot see the bound. That is the `bash` line cap defect exactly, and it is why the
third test reads a source with no end. See `D-a-config-file-is-read-under-a-cap`.

## 3. A failed run releases the steering-queue observer

`Driver::run` released the observer on its last line, and six early returns never reached it. Two
of those end a failed run, so the event channel stayed open for ever: one leaked task per failed
run, a stale observer, and no end to the event stream.

Proved by a probe against the public API:

```
before  stream_closed_cleanly=false saw_error_event=true
after   stream_closed_cleanly=true  saw_error_event=true
```

Four frontends already worked around it, each with its own comment. The workaround holds only
while every failed return is preceded by an error item, and nothing pinned that.

Driven for real, with a bad credential so the provider fails inside the loop:

```sh
AWS_REGION=us-east-1 AWS_ACCESS_KEY_ID=... rho run "say hi" --provider bedrock --model ...
run 1: exit=1 seconds=0
run 2: exit=1 seconds=1
```

The JSONL frontend, twice, emits one fault and one settled and then exits:

```json
{"type":"fault","kind":"provider","message":"server error: status 500"}
{"type":"settled","stop_reason":"faulted"}
```

Mutation: with the release deleted, all five tests in `crates/rho-core/tests/run_failure.rs`
fail. The release is load-bearing on every path, including the good one.

`D-an-error-on-the-event-stream-ends-the-run` recorded this debt and deferred it, because
another worktree owned the queue. That worktree merged. Two claims in that decision were also
wrong, and both are corrected in it: there are three error sites, not two, and the one-line fix
it prescribed was not enough. See `D-a-failed-run-releases-the-queue-observer`.

## Findings that are open

Each of these is reproduced and none is fixed. A reader meets them here.

| finding | where | why it is still open |
| --- | --- | --- |
| A pending-run turn whose reasoning is refused still sends its `tool_use` with no thinking block, so Anthropic rejects the turn. | `rho-provider-bedrock/src/lib.rs:810-840` | Two lenses found it, including an outside review. The fix is a choice: refuse to build the request, or strip the orphaned call. It needs a decision. |
| An empty `tool_use` stop spins turns that run no tool and tell nobody. Measured: 5 provider calls, 0 tools, 0 errors, and it exits as if it hit the turn cap. | `rho-core/src/agent.rs:709,772` | The behaviour it should have is a product choice: a distinct stop reason, or a decode fault. |
| No finished background task is ever reaped. Each keeps up to 100 KB of output for the whole session. | `rho-core/src/tasks.rs:420` | The bound is a new limit in a contract, so it needs a spec and a number. |
| `redact_block` ends with a wildcard arm, directly under a comment that forbids one. A text block reaches the session file and its sidecar unredacted. | `rho-core/src/session/mod.rs` | Naming every arm is mechanical. Whether a text block is redacted at all is a decision about resume fidelity. |
| An unknown field in a session record is dropped. `fork` re-encodes the record, so the field dies on disk. | `rho-core/src/session/mod.rs` | The persisted format binds the next version of rho, so the change needs a spec and a migration plan. |
| `AgentEvent::Stream(_)` is a wildcard over 13 `StreamEvent` variants, so a new one is dropped in silence. | `rho-jsonl/src/pump.rs:70` | Mechanical, and it belongs with the task-event gap in the same map. |
| `newest_open` has no production caller, so the closed-versus-open crash offer reaches no user. Five tests cover it. | `rho-core/src/session/mod.rs:1551` | Wire it or delete it. That is a product choice. |

Two process gaps, also open:

- `bench/check-dead-surface.py` reports 36 violations and runs in neither the gate nor
  `agentic-workflow.yaml`. A guard nobody runs is not a guard.
- `bench/check-claimed-tests.py` prints `VIOLATIONS 0 (no commits in origin/main..HEAD)` on a
  merged branch. It never audited pull requests #4 to #16.

## Findings the review killed

- Two reported blockers said the suite was not green. Both were an artefact: one lens mutated
  `rho-config` to test for vacuous tests while another ran the whole suite in the same worktree.
  A mutation lens needs its own worktree.
- The three project-file trust rules hold. A live drive with a hostile `.rho/config.toml` showed
  that a limit is never raised, a credential is refused by name, and `session-root`, `base-url`
  and `skill-paths` are stripped, at profile depth too.
- All five `tracing::subscriber::with_default` test sites are sound. Three were proved by
  deleting the production log line and watching the test fail.
