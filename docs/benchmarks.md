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
nothing at all. See decision D-032.

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
See decision D-032.

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
