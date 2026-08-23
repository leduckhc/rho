# rho benchmarks

This file holds measured numbers. Each number states the exact command that
produced it. A number that is not measured says so and says why. Do not estimate
a number. Do not copy a number from another project.

Platform for the numbers below: macOS on Apple Silicon (aarch64). Release build
profile: `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = "symbols"`.
See the workspace `Cargo.toml`.

Note on ownership: stage S8 filled the numbers it could measure without a network
and without a real terminal. Stage S9 measures resident memory per live session
against a real provider. Stage S11 automates every number in `bench/footprint.sh`
and runs it in CI.

## Reproduce these numbers

Run `bench/footprint.sh` from the repository root. It measures every number on
this page. It builds the release binary and the examples. Then it reports the
size, the memory, and the first-frame time. It takes the median of three runs for
each memory and timing number.

```sh
bash bench/footprint.sh          # readable table
bash bench/footprint.sh --json   # one JSON object for CI to diff
```

The script works on macOS and on Linux. The two systems report peak resident set
size in different units. macOS `/usr/bin/time -l` uses bytes. GNU `/usr/bin/time
-v` uses kilobytes. The script detects which reporter is present and converts
both to bytes.

The numbers on this page are macOS on Apple Silicon. CI runs the script on Linux
and publishes the JSON as a build artifact.

## Release binary size

The size of the `rho` binary, stripped, for two feature sets.

| Feature set | Command | Size (bytes) | Size |
| --- | --- | --- | --- |
| default | `cargo build --release -p rho-cli` | 9,691,632 | 9.2 MiB |
| minimal | `cargo build --release -p rho-cli --no-default-features --features minimal` | 6,274,400 | 6.0 MiB |

The default set links the TUI, all three providers, and the plugin host. The
minimal set is headless with one provider (OpenRouter) and no plugins.

Command to read the size after each build:

```sh
ls -l target/release/rho | awk '{print $5}'
```

## Time to first frame

The app draws the first frame before the first token arrives. The number below
measures the cost from the start of the process to the completed first render,
into a `ratatui` `TestBackend`.

Method: the example `crates/rho-tui/examples/first_frame.rs` builds the default
`TuiState` and renders one frame. It uses a headless `TestBackend`, so the number
excludes real terminal output. Two views:

- In-process render cost, printed by the example: about 0.3 ms to 0.5 ms.
- Whole-process wall time, including the loader and process start, over ten runs:
  minimum 1.91 ms, median 2.15 ms, maximum 3.54 ms.

Commands:

```sh
cargo build --release -p rho-tui --example first_frame
target/release/examples/first_frame            # prints the in-process render cost
python3 - <<'PY'                               # whole-process wall time, ten runs
import subprocess, time
ts=[]
for _ in range(10):
    t=time.perf_counter()
    subprocess.run(['target/release/examples/first_frame'])
    ts.append((time.perf_counter()-t)*1000)
ts.sort()
print('min=%.2f median=%.2f max=%.2f ms' % (ts[0], ts[len(ts)//2], ts[-1]))
PY
```

Caveat: this uses a test backend, not a real terminal. It skips process start, dynamic
linking, raw-mode setup, and the alternate-screen switch. The next section measures all four
on a real pseudo-terminal.

### Time to first frame on a real terminal

| Measure | Value |
| --- | --- |
| Minimum | 5.4 ms |
| **Median** | **6.5 ms** |
| Maximum | 8.0 ms |

Twelve runs. Each forks a real pty, sets a 30 by 100 window, launches the binary, and stops
the clock when the status line reaches the screen.

```sh
cargo build --release -p rho-cli
python3 bench/tui_first_frame.py
```

The figure includes building the provider client, because a user cannot start a session
without one.

#### How the harness ends a run

The harness closes the pty master first, and it kills the child second. Then it waits with
a deadline of five seconds. `bench/ptyharness.py` holds that order, and
`bench/test_ptyharness.py` pins it.

The order is not a style choice. A run stops reading the master as soon as it has its
sample, so the pty buffer fills. The child then blocks inside a write to its own terminal.
`SIGKILL` cannot finish while that write sits in the kernel, and the child stays in state
`?Es`. A close of the master makes the write fail with `EIO`, which frees the child at
once. Measured with the release binary on 2026-08-18, three runs for each order:

| Teardown order | Time to reap the child |
| --- | --- |
| Close the master, then `SIGKILL` | 13 ms, 13 ms, 13 ms |
| `SIGKILL`, master open, no drain | 11 ms, more than 8 s, more than 8 s |
| `SIGKILL`, master open, keep draining | 0 ms, 0 ms, 0 ms |

The shipped code had no deadline, so it hung on the first wedged run. A deadline alone only
converts that hang into a slow benchmark: with the wrong order kept, twelve runs took 60.4
seconds and every run reported a child over its deadline. With the order corrected, the
same twelve runs take 0.42 seconds. The old shape also never closed the master, so twelve
runs leaked twelve file descriptors. See `docs/verification/pty-harness-teardown.md`.

**For comparison, using each project's own published figure.** jcode reports 14.0 ms to
first frame and 48.7 ms to first input. pi reports 590.7 ms to first frame. Those come from
a different harness on different hardware, so read the comparison as an order of magnitude
and not a ranking.

## Idle resident memory

Two measurements, both peak resident set size from `/usr/bin/time -l`.

| What | Command | Peak RSS (bytes) | Peak RSS |
| --- | --- | --- | --- |
| First-frame render only (no runtime, no provider) | `/usr/bin/time -l target/release/examples/first_frame` | 2,605,056 | 2.5 MiB |
| `rho --help` (full binary, clap, no session) | `/usr/bin/time -l target/release/rho --help` | 7,028,736 | 6.7 MiB |

### Resident memory per session

Measured. The earlier note said this needed a live provider. It does not. An idle
session holds everything that costs memory, which is the provider client with its
TLS stack, the tool registry, the hook chain, and the context. A network call adds
a transient buffer, not the steady state.

Two numbers matter, and they answer different questions.

**One process that holds one session.**

| What | Peak RSS (bytes) | Peak RSS |
| --- | --- | --- |
| One idle session, default features | 8,727,000 | 8.3 MiB |

```sh
cargo build --release -p rho-cli --example idle_session
/usr/bin/time -l ./target/release/examples/idle_session   # read "maximum resident set size"
```

**One more session inside the same process.** This is the number that decides
whether one machine can hold 50 sessions. A total hides the answer, because the
binary, the allocator, and the TLS stack are paid once. The slope matters, not the
intercept.

| Sessions in one process | Peak RSS | Cost of one more session |
| --- | --- | --- |
| 1 | 8.32 MiB | — |
| 51 | 9.73 MiB | 29,600 B (0.028 MiB) |
| 101 | 10.75 MiB | 21,299 B (0.020 MiB) |

Across the whole range, from 1 to 101, the cost is **25,450 bytes, or 0.024 MiB,
per additional session**. The growth is linear. **101 idle sessions fit in
10.75 MiB.**

```sh
cargo build --release -p rho-cli --example many_sessions
RHO_SESSIONS=1   /usr/bin/time -l ./target/release/examples/many_sessions
RHO_SESSIONS=51  /usr/bin/time -l ./target/release/examples/many_sessions
RHO_SESSIONS=101 /usr/bin/time -l ./target/release/examples/many_sessions
```

Each figure is the median of three runs on macOS with Apple Silicon. Every session
gets its own provider client, tool registry, hook chain, and context, because a
real host does not share those between users.

## How these numbers compare, and how they do not

**Read this before you quote a comparison.** The published figures for other
harnesses are **not the same measurement**, and we will not pretend they are.

`jcode.sh` reports "extra proportional memory (PSS) that each additional client
adds once one is already running", sampled August 2026. Those harnesses run **one
operating-system process per session**, so their incremental cost includes a whole
process: a language runtime, a binary, and a TLS stack.

rho is a library first. A host embeds it and holds many sessions in one process.
So rho's incremental cost excludes the things a process-per-session design pays
again every time.

So the honest comparison is the **conservative** one. Set rho's *total* for one
whole process against the other harnesses' *incremental* cost per session:

| Harness | Number | What it measures |
| --- | --- | --- |
| rho | 8.3 MiB | **total** peak RSS of a whole process holding one session |
| jcode | ~10.4 MiB | extra PSS per additional session process |
| Codex CLI | ~21.6 MiB | extra PSS per additional session process |
| pi | ~76.5 MiB | extra PSS per additional session process |
| Claude Code | ~213 MiB | extra PSS per additional session process |
| OpenCode | ~318 MiB | extra PSS per additional session process |

Other harnesses: `https://jcode.sh` published benchmarks, sampled August 2026.

Even on that conservative footing, rho's whole process costs less than the
*marginal* session of every harness in the list. On its own ground, which is many
sessions inside one host, the marginal session costs 0.024 MiB.

## Live concurrent sessions

**The number this project rests on, measured at last.** Every earlier figure on this page
is an idle session. An idle session holds a provider client, a tool registry, and a
context, and it sends nothing. This section measures sessions that are all working at
once, each sending a real prompt to a real provider and draining a real streamed answer.

Provider OpenRouter, model `anthropic/claude-haiku-4.5`, macOS on Apple Silicon. Each
figure is the mean of two runs. Every session in every run succeeded.

| Live sessions | Peak RSS | Cost of one more | Slowest first token |
| --- | --- | --- | --- |
| 1 | 13.17 MiB | — | 1292 ms |
| 50 | 24.98 MiB | 247 KB | 1558 ms |
| 100 | 34.23 MiB | 189 KB | 3564 ms |

```sh
export OPENROUTER_API_KEY=...
cargo build --release -p rho-cli --example live_sessions
RHO_SESSIONS=50 /usr/bin/time -l ./target/release/examples/live_sessions
```

**A live session costs about ten times an idle one**, at roughly 247 KB against 25 KB.
That is expected and worth stating: a live session holds a response stream, a decode
buffer, and a queue of parsed events, and it holds them while the model generates.

**Concurrency barely hurts latency up to 50.** The slowest first token moved from 1292 ms
to 1558 ms, about 20 percent, for fifty times the work. At 100 it more than doubles, so
the provider's own rate limiting is the likely bound rather than rho.

### Why no competitor publishes this number

jcode and pi both run one operating-system process per session. So their marginal cost
carries a whole process: a runtime, a binary, and a TLS stack. rho is a library, and a
host holds many sessions in one address space.

The conservative comparison, using each project's own published figure:

| Harness | 50 sessions | Basis |
| --- | --- | --- |
| rho | **26 MB**, measured live | Total for one process holding 50 live sessions |
| pi | about 3.7 GB | 76.5 MB incremental per session, times 50 |

That is about 146 times less. The comparison still favours the other side, because pi's
figure is incremental while rho's is a total.

**What this does not show.** The sessions ran one short turn each. A long session
accumulates context, and context is the dominant cost over time, so a long-running fleet
will not hold at 247 KB. Measuring that needs a soak test, and there is not one yet.

## Token accounting and cost

The same example reports the accounting, and two of the three providers used to report
nothing at all. See decision D-measured-cost-and-cache.

```
tokens          in 70200 out 306 cache_read 0
cache hit rate  0.0 %
cost            $0.000000 as charged by the provider
```

The input figure is exactly fifty times the single-session figure of 1404, which is the
check that the accounting is real rather than approximate.

**The cost is the amount the provider charged, never an estimate from a price table.** It
reads zero here because this key bills upstream directly, so OpenRouter reports no charge
of its own. A field that a provider does not report stays absent rather than reading as
free.

**The cache hit rate is zero, and that is an honest zero.** rho now asks for the
accounting and parses it, and Anthropic caching through this path did not engage. Placing
a cache breakpoint at the end of the stable prefix is the next step and it is not done.
See decision D-measured-cost-and-cache.

## A note on running the script

`bench/footprint.sh` measures the minimal build last, because cargo keys its output
path on the package and not on the feature set, so the two binaries cannot coexist at
`target/release/rho`.

The script then rebuilds the default binary, so it never leaves a crippled one behind.
That trap was real. The controller ran the script, then ran `rho --provider bedrock`,
and got `the provider bedrock is not in this build`. The message named the fix, but the
cause was invisible. The script now restores the default build and checks that it runs.

## Linux numbers, from CI

Every other figure on this page is macOS on Apple Silicon. CI now runs
`bench/footprint.sh` on `ubuntu-latest` and uploads the JSON as a build artifact,
so Linux is measured too. Run 32051431230, 2026-08-17.

| What | Linux x86_64 | macOS aarch64 |
| --- | --- | --- |
| Binary, default features | 12,747,104 B (12.2 MiB) | 9,708,160 B (9.3 MiB) |
| Binary, `minimal` features | 8,335,912 B (8.0 MiB) | 6,274,432 B (6.0 MiB) |
| Peak RSS, 1 session | 11,739,136 B (11.2 MiB) | 8,732,672 B (8.3 MiB) |
| Peak RSS, 101 sessions | 14,524,416 B (13.9 MiB) | 11,206,656 B (10.7 MiB) |
| Cost per extra session | 27,852 B (0.027 MiB) | 24,739 B (0.024 MiB) |
| Time to first frame, median | 1.48 ms | 3.26 ms |

Three honest notes.

The cost per extra session agrees closely across the two systems, at 27 KB and
25 KB. That agreement is the useful signal, because it is the slope.

The Linux run reported 51 sessions at 11,718,656 bytes, which is **lower** than its
own 1-session figure of 11,739,136 bytes. That is measurement noise on a shared CI
runner, not a real result. So read the 1-to-101 slope, and treat a single point on a
hosted runner with suspicion. A local run is quieter.

The Linux binary is larger. That is normal, and it follows from a different object
format and different system libraries.

**What is still not measured here.**

- Time to first frame on a real pseudo-tty, with raw mode and the alternate
  screen. `bench/footprint.sh` does not add a pty harness.
- Time to first token against each provider. See stage S9.
- Any figure on Windows. Neither the script nor CI covers it.

Resident memory during a live streamed turn is now measured. It is 14.8 MiB,
recorded in `docs/verification/sprint-1.md`. A live turn costs about 6.5 MiB more
than the 8.3 MiB idle session on this page.

## JSONL codec, for the session log and the providers

Measured on macOS on Apple Silicon (aarch64), rustc 1.95.0, in sprint 2. Each
number is the best of 30 rounds. Corpus A is a real 1848-record pi session file,
5.28 MiB. Corpus B is 50000 short records in the tool-event shape, 7.66 MiB.

```sh
cd bench/jsonl-codec
cargo run --release -- ~/.pi/agent/sessions/<project>/<stamp>_<uuid>.jsonl
```

| Path | Corpus | `serde_json` | `sonic-rs` | `simd-json` |
| --- | --- | --- | --- | --- |
| typed decode, line by line | A | 2.01 ms | 1.87 ms | 2.40 ms |
| typed decode, line by line | B | 8.45 ms | 7.40 ms | 15.80 ms |
| typed encode, record by record | A | 2.15 ms | 0.76 ms | 1.54 ms |
| typed encode, record by record | B | 5.05 ms | 4.99 ms | 4.64 ms |
| untyped value, line by line | A | 2.23 ms | 1.11 ms | not run |
| untyped value, line by line | B | 16.02 ms | 6.13 ms | not run |

The cost of the extra dependency, on a minimal binary with the rho release
profile:

| Build | Binary size | Dependency tree | Cold build |
| --- | --- | --- | --- |
| `serde_json` only | 352,128 B | 21 lines | 5.0 s |
| plus `sonic-rs` | 434,784 B | 102 lines | 8.5 s |

The decision is in `docs/adr/20260818-014343-ADR-jsonl-codec.md`. `serde_json` is the default.
`sonic-rs` sits behind the `fast-json` feature, which is off by default.

Read one caution with these numbers. A session append is one record of about 200
bytes, so the encode costs about 100 ns and the write costs microseconds. A faster
codec does not make `store` faster. The codec matters on resume, and it matters
more on the provider SSE path.

## Session log, append and resume

Measured in sprint 2 on macOS on Apple Silicon (aarch64), rustc 1.95.0, release build with
the workspace profile. The bench drives the real `SessionWriter` and the real
`SessionReader`, not a copy of their logic.

```sh
# The scratch bench lives outside the repository, because it links rho-core by path.
# Source: 20000 appends of a 150-byte assistant message, then one full read.
```

| Path | Result |
| --- | --- |
| Append, 20000 records | 35.2 ms total, 1762 ns per record |
| Lines written | 20001 for 20000 appends, plus the header |
| File size | 5977922 bytes |
| Resume, full read | 20000 entries in 12.6 ms, 452.5 MiB per second |

The append cost includes the encode and the id. The write path holds one open sink and
writes one record as one write.

Two numbers explain the design, and both are measured. A reopen of the file per record
costs 17567 ns, which is 17 times the held sink. An `fsync` per record costs 3076785 ns,
which is about 3000 times the write. So the writer holds its sink and does not `fsync` per
record. See decision D-writer-holds-one-sink and `SPEC-sessions` section 4a.

## Sprint 3: the cost of the TUI experience

Stage U5 of sprint 3. It measures what the new interface costs. The spec
`docs/specs/20260818-090413-SPEC-tui-experience.md` states a cost budget per
feature. This section tests those claims.

Machine: macOS on Apple Silicon (aarch64), rustc 1.95.0. Date: 2026-08-18. Each
timing and memory number is the median of three runs. No network runs here.

One honest scope note comes first. Stage U4 wires the new renderer in four
slices. When these numbers were taken, a sibling was on the paste slice, and the
renderer was still the minimal one from `SPEC-tui`. The motion, the durations, and
the theme exist as pure functions, but the renderer does not call them yet. So the
frame numbers below are the renderer as it stands. Where a budget row needs the
wired renderer, the row is marked not yet checkable.

### Reproduce these numbers

```sh
cargo build --release -p rho-cli
cargo build --release -p rho-tui --example frame_bench
cargo build --release -p rho-tui --example first_frame
python3 bench/tui_frame.py          # readable table
python3 bench/tui_frame.py --json   # one JSON object
```

`bench/tui_frame.py` takes the median of three runs. It reuses
`bench/tui_first_frame.py` for the pseudo-terminal measurement. It drives
`crates/rho-tui/examples/frame_bench.rs` for the frame time and the allocation
count.

### Time to first frame, on a real pseudo-terminal

Measured before and after the sprint, on the same pty harness. The before figure
is the sprint-1 number recorded above. Both are rho's own measurements.

| When | Runs | Median | Command |
| --- | --- | --- | --- |
| Sprint 1 | 12 | 6.5 ms | `python3 bench/tui_first_frame.py` |
| Sprint 3 | 12 | 6.5 ms | `python3 bench/tui_first_frame.py` |

The sprint-3 figure is a median of medians. Three separate runs of twelve reported
6.8, 6.5, and 6.3 milliseconds, so 6.5 is the honest centre.

**A correction, recorded rather than quietly fixed.** The first draft of this row
said 6.1 milliseconds. That was the **minimum** of one run, not its median. A
minimum is the friendliest number in a set, and quoting it as a median overstates
the result by about six percent. The rule stands: report the median that the
harness prints, and state the run count beside it.

One run in twelve reaches about 440 milliseconds. That outlier is a cold start,
which is why the harness reports a median rather than a mean. The median is stable
across runs and the mean is not.

The two sprints agree within noise. The interface added no first-frame cost,
because the renderer is unchanged so far. The harness forks a real pty, sets a 30 by 100
window, launches the release binary, and stops the clock at the status line.

### Frame time under a streaming turn

The example renders one frame per streamed delta, through a `ratatui`
`TestBackend`, at 100 by 30. It times each `terminal.draw`.

| Measure | Value | Command |
| --- | --- | --- |
| 50th percentile | 58 us | `target/release/examples/frame_bench` |
| 99th percentile | 70 us | `target/release/examples/frame_bench` |

The sample count is 5000 frames per run. A percentile from a few hundred samples
is noise, so the count is stated. At a 100 ms tick, a 70 us frame is under one
part in a thousand of the budget. The renderer is not the bottleneck.

### Allocation per frame, in the steady state

A counting global allocator wraps the system allocator, inside the example. The
example arms the counter, renders a fixed transcript 2000 times with one terminal
reused, then divides. A reused terminal is the honest steady state.

| Measure | Value | Command |
| --- | --- | --- |
| Allocations per frame | 300 | `target/release/examples/frame_bench` |
| Bytes per frame | 15,406 | `target/release/examples/frame_bench` |

This is the minimal renderer, not the wired one. It allocates a `Line` per row
and a `String` per cell. So the U4 goal of a zero-allocation steady frame is not
met yet. The number is a baseline the wired renderer must beat.

The motion is measured on its own, because the renderer does not call it yet. The
`sweep_frame` helper returns a `Vec`, so it allocates once per call.

| Measure | Value | Command |
| --- | --- | --- |
| `sweep_frame` allocations per call | 1 | `target/release/examples/frame_bench` |

That is a finding. The budget row for motion claims zero allocations per frame.
As written, `sweep_frame` allocates one `Vec` per call. To meet the budget, the
wired renderer must write styles into cells that exist, not call `sweep_frame` per
frame. The per-character allocation that the spec refuses is absent, so the claim
holds on that narrow point. The per-frame zero does not hold through this helper.

### Resident memory, for the frame render

Two peak resident set size figures, both rho's own. They cover the render path
alone, with no provider client and no session.

| What | Peak RSS (bytes) | Peak RSS | Command |
| --- | --- | --- | --- |
| Idle frame | 2,621,440 | 2.50 MiB | `python3 -c '...' target/release/examples/first_frame` |
| Streaming frames | 3,309,568 | 3.16 MiB | `python3 -c '...' target/release/examples/frame_bench` |

`bench/tui_frame.py` reports these two. It uses the same portable reporter as
`bench/footprint.sh`. macOS reports `ru_maxrss` in bytes. Linux reports it in
kilobytes. The reporter converts by platform, so both mean bytes.

The idle and streaming session figures are larger and separate. An idle session
holds 8.3 MiB, measured in the idle-session section above. A live streamed turn
holds 14.8 MiB, recorded in `docs/verification/sprint-1.md`. Those hold a provider
client and a context. The two figures here hold neither.

### Binary size, sprint 3

Both feature sets grew since sprint 2, by about the same amount.

| Feature set | Sprint 2 | Sprint 3 | Command |
| --- | --- | --- | --- |
| default | 9,691,632 B | 10,221,504 B | `cargo build --release -p rho-cli` |
| minimal | 6,274,400 B | 6,804,304 B | `cargo build --release -p rho-cli --no-default-features --features minimal` |

The default set grew by 529,872 bytes. The minimal set grew by 529,904 bytes. The
two grew by almost the same amount. The minimal build has no TUI, so the growth is
not the TUI. It is the session log and the config work that landed earlier this
sprint. `bench/footprint.sh` measures the minimal size and restores the default
binary afterwards.

### Beside pi and jcode

State rho's own numbers beside the two prior-art figures. The prior-art figures
are each project's own published claim, not a rho measurement.

| Harness | First frame | Per session held | Ownership |
| --- | --- | --- | --- |
| rho | 6.1 ms | 8.3 MiB | rho measured it |
| jcode | 14 ms | 10.4 MiB | jcode's published claim |
| pi | 591 ms | 76.5 MiB | pi's published claim |

Read the comparison as an order of magnitude. The harnesses use different
machines and different methods. rho's 8.3 MiB is a whole idle session. The other
two figures are an incremental per-session cost. See the comparison section above
for why that footing favours the other side.

### The cost budget, tested

The spec claims mostly zero added rows, zero allocations per frame, and zero bytes
held. Motion claims eight bytes held, for the tick count.

| Budget row | Claim | Verdict |
| --- | --- | --- |
| Motion, bytes held | 8 bytes | Holds. The tick is one `u64` in the state. |
| Motion, allocations per frame | 0 | Not met by `sweep_frame`, which allocates one `Vec` per call. |
| The renderer, steady frame | 0 allocations | Not met yet. The minimal renderer allocates 300 per frame. |

The eight-byte claim for motion holds. A tick is one `u64`. The zero-allocation
claims do not hold through the code that exists. The renderer is not wired yet, so
these are baselines, not final verdicts. Stage U4 must land the wired renderer,
and then this section must run again.

### What is not measured, and why

- Frame time and allocation for the wired renderer. It is not landed, because
  stage U4 is in progress. These numbers cover the minimal renderer.
- Motion cost inside the render. The renderer does not call the motion yet.
- First frame on Windows. Neither the script nor CI covers it.
- Frame time on Linux. The example builds there, but this run was macOS only.

<<<<<<< ours
## Reasoning in `full` mode: the frame cost of a growing row

Date: 20260823, re-measured after the merge with `main`.

**The fix this section described is gone, and it is not needed.** A performance review found
that the previous renderer sanitised and wrapped the whole accumulated reasoning text on every
frame, and then cut it to a fixed band of fourteen rows. `tail_for_band` sliced the tail, and the
frame went from 2769 µs to 95 µs at 504 kB, which is 29 times cheaper.

That renderer no longer exists. `main` replaced it with a scrolling transcript in the alternate
screen, where the user can scroll back and rho can repaint any row. A tail would then **lose a
line the user can reach**, so the slicing was removed in the merge.

The new renderer does not have the defect. It builds a window of the visible lines instead of
wrapping every row, so the cost does not grow with the text:

```sh
cargo build --release -p rho-tui --example reason_bench
for d in 1000 2000 4000 8000; do RHO_DELTAS=$d ./target/release/examples/reason_bench; done
```

| deltas | row bytes | avg frame, merged renderer |
| --- | --- | --- |
| 1000 | 63 kB | 135 µs |
| 2000 | 126 kB | 110 µs |
| 4000 | 252 kB | 109 µs |
| 8000 | 504 kB | 109 µs |

Flat, with no slicing. Two branches met the same problem and the other one solved it better, so
the merge keeps its answer and drops mine. The numbers come from one machine, one run each, at a
fixed 100 by 30 terminal. They are a ratio, not a promise.

`bench/deleted-tests.txt` records the eight tests removed with `tail_for_band`, because they
pinned a function that no longer exists.

### What this measurement does not cover

A reviewer named it, and it is still true. The number is the cost of building the frame, through
`TestBackend`. Nothing is drawn to a real terminal, so no write, no flush, and no terminal-side
cost is in it.
