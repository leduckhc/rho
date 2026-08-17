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

Caveat: this uses a test backend, not a real terminal. It does not include the
cost of raw-mode setup and the alternate-screen switch, which happen once at
start-up and touch the real terminal. A real-terminal number needs a pseudo-tty
harness. Stage S11 adds that in `bench/footprint.sh`.

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

**What is still not measured.**

- Resident memory during an active streamed turn, with a real provider. Stage S9.
- Time to first frame on a real pseudo-tty, including raw mode and the alternate
  screen. Stage S11.
- Time to first token against each provider. Stage S9.
- Any number on Linux. Every figure here is macOS on Apple Silicon.
