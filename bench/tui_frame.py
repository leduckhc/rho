#!/usr/bin/env python3
"""tui_frame.py — measure what the sprint-3 interface costs.

This is the frame and first-frame measurement for stage U5 of sprint 3. It
matches the habits of `bench/footprint.sh`: it takes the median of three runs for
each timing and memory number, it reports the machine and the date, and it prints
a readable table by default or one JSON object with `--json`.

It measures five things, and it names the command behind each.

  1. Time to first frame, on a real pseudo-terminal. It reuses
     `bench/tui_first_frame.py`, which forks a pty, launches the release binary,
     and stops the clock when the status line reaches the screen. No mock.
  2. Frame time under a streaming turn, at the 50th and the 99th percentile. The
     example `crates/rho-tui/examples/frame_bench.rs` drives the real renderer
     through a `ratatui` `TestBackend`, one frame per streamed delta, and reports
     the two percentiles with the sample count behind them.
  3. Allocation per frame in the steady state. The same example arms a counting
     global allocator and renders a fixed transcript many times, with one
     terminal reused, then divides.
  4. Resident memory, idle and streaming. It measures the peak resident set size
     of the first-frame example (one idle frame) and of the frame_bench example
     (a streaming render loop).
  5. Binary size, default feature set. The minimal set is owned by
     `bench/footprint.sh`, because measuring it needs a rebuild-and-restore dance
     that that script already does safely. This script names that command.

No network. This script runs no git command. It reads and builds only; it never
touches `crates/rho-tui/src`.

Usage:
  python3 bench/tui_frame.py            # readable table
  python3 bench/tui_frame.py --json     # one JSON object
  python3 bench/tui_frame.py --no-build # measure existing artifacts
"""

import json
import os
import platform
import re
import subprocess
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RUNS = 3

BIN = os.path.join(REPO, "target", "release", "rho")
EX_FRAME = os.path.join(REPO, "target", "release", "examples", "frame_bench")
EX_FIRST = os.path.join(REPO, "target", "release", "examples", "first_frame")


def die(msg):
    print(f"tui_frame.py: {msg}", file=sys.stderr)
    sys.exit(1)


def run(cmd, **kw):
    return subprocess.run(cmd, cwd=REPO, **kw)


def median(values):
    if not values:
        die("median needs at least one value")
    s = sorted(values)
    return s[len(s) // 2]


def build():
    print("tui_frame.py: building the release binary and the examples...", file=sys.stderr)
    for cmd in (
        ["cargo", "build", "--release", "-p", "rho-cli"],
        ["cargo", "build", "--release", "-p", "rho-tui", "--example", "frame_bench"],
        ["cargo", "build", "--release", "-p", "rho-tui", "--example", "first_frame"],
    ):
        if run(cmd, stdout=sys.stderr.fileno(), stderr=sys.stderr.fileno()).returncode != 0:
            die(f"build failed: {' '.join(cmd)}")


# A fresh Python subprocess measures one command's peak RSS, so one command's
# peak never bleeds into the next through the cumulative child counter. This is
# the portable reporter `bench/footprint.sh` documents: macOS and the BSDs report
# ru_maxrss in bytes, Linux reports kilobytes, so the helper converts by platform.
_RSS_HELPER = r"""
import resource, subprocess, sys, os
with open(os.devnull, "wb") as null:
    r = subprocess.run(sys.argv[1:], stdout=null, stderr=null)
if r.returncode != 0:
    sys.exit(r.returncode)
peak = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
if sys.platform.startswith("linux"):
    peak *= 1024
print(peak)
"""


def rss_bytes(argv, env=None):
    proc = subprocess.run(
        [sys.executable, "-c", _RSS_HELPER, *argv],
        cwd=REPO,
        env={**os.environ, **(env or {})},
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        die(f"command failed under the RSS reporter: {' '.join(argv)}\n{proc.stderr}")
    value = proc.stdout.strip()
    if not value or int(value) <= 0:
        die(f"empty or zero RSS sample for: {' '.join(argv)}")
    return int(value)


def median_rss(argv, env=None):
    return median([rss_bytes(argv, env) for _ in range(RUNS)])


def first_frame_pty():
    """Reuse bench/tui_first_frame.py. Parse its runs, min, median, max."""
    script = os.path.join(REPO, "bench", "tui_first_frame.py")
    proc = subprocess.run(
        [sys.executable, script], cwd=REPO, capture_output=True, text=True
    )
    if proc.returncode != 0:
        die(f"tui_first_frame.py failed:\n{proc.stderr}")
    out = proc.stdout
    runs = _grab(r"runs\s+(\d+)", out, cast=int)
    return {
        "runs": runs,
        "min_ms": _grab(r"min\s+([\d.]+)\s*ms", out),
        "median_ms": _grab(r"median\s+([\d.]+)\s*ms", out),
        "max_ms": _grab(r"max\s+([\d.]+)\s*ms", out),
    }


def _grab(pattern, text, cast=float):
    m = re.search(pattern, text)
    if not m:
        die(f"could not parse {pattern!r} from:\n{text}")
    return cast(m.group(1))


def frame_bench():
    """Run the example RUNS times. Take the median of each percentile."""
    runs = []
    for _ in range(RUNS):
        proc = subprocess.run([EX_FRAME], cwd=REPO, capture_output=True, text=True)
        if proc.returncode != 0:
            die(f"frame_bench failed:\n{proc.stderr}")
        runs.append(json.loads(proc.stdout))
    first = runs[0]
    return {
        "frame_size": first["frame_size"],
        "stream_samples": first["stream_samples"],
        "render_frame_us_p50": median([r["render_frame_us_p50"] for r in runs]),
        "render_frame_us_p99": median([r["render_frame_us_p99"] for r in runs]),
        "steady_frames": first["steady_frames"],
        "render_allocs_per_frame": median([r["render_allocs_per_frame"] for r in runs]),
        "render_bytes_per_frame": median([r["render_bytes_per_frame"] for r in runs]),
        "motion_allocs_per_call": median([r["motion_allocs_per_call"] for r in runs]),
        "motion_in_renderer": first["motion_in_renderer"],
    }


def main():
    json_out = "--json" in sys.argv[1:]
    do_build = "--no-build" not in sys.argv[1:]
    for arg in sys.argv[1:]:
        if arg not in ("--json", "--no-build"):
            die(f"unknown option: {arg}")

    if do_build:
        build()
    for path in (BIN, EX_FRAME, EX_FIRST):
        if not os.path.exists(path):
            die(f"artifact not built: {path} (run without --no-build)")

    machine = f"{platform.system()}-{platform.machine()}"
    date = time.strftime("%Y-%m-%d", time.gmtime())

    ff = first_frame_pty()
    fb = frame_bench()
    rss_idle = median_rss([EX_FIRST])
    rss_stream = median_rss([EX_FRAME])
    size_default = os.path.getsize(BIN)

    if json_out:
        print(json.dumps(
            {
                "machine": machine,
                "date": date,
                "runs_per_number": RUNS,
                "first_frame_pty": ff,
                "frame_bench": fb,
                "idle_frame_peak_rss_bytes": rss_idle,
                "streaming_frame_peak_rss_bytes": rss_stream,
                "binary_size_default_bytes": size_default,
            },
            indent=2,
        ))
        return

    mib = lambda b: b / 1048576.0
    print(f"rho TUI frame cost. Machine: {machine}. Date: {date}.")
    print(f"Each timing and memory number is the median of {RUNS} runs.")
    print()
    print("Time to first frame, on a real pseudo-terminal "
          "(bench/tui_first_frame.py):")
    print(f"  runs {ff['runs']}   min {ff['min_ms']} ms   "
          f"median {ff['median_ms']} ms   max {ff['max_ms']} ms")
    print()
    print(f"Frame time under a streaming turn ({fb['frame_size']}, "
          f"{fb['stream_samples']} samples):")
    print(f"  p50 {fb['render_frame_us_p50']:.2f} us   "
          f"p99 {fb['render_frame_us_p99']:.2f} us")
    print()
    print(f"Allocation per frame, steady state ({fb['steady_frames']} frames):")
    print(f"  {fb['render_allocs_per_frame']:.1f} allocations/frame   "
          f"{fb['render_bytes_per_frame']:.0f} bytes/frame")
    print(f"  motion sweep_frame, in isolation: "
          f"{fb['motion_allocs_per_call']:.1f} allocation/call "
          f"(not yet called by the renderer)")
    print()
    print("Peak resident set size:")
    print(f"  idle frame (first_frame)      {rss_idle:>10} bytes  {mib(rss_idle):6.2f} MiB")
    print(f"  streaming frames (frame_bench){rss_stream:>10} bytes  {mib(rss_stream):6.2f} MiB")
    print()
    print("Binary size, default features:")
    print(f"  {size_default} bytes  {mib(size_default):.2f} MiB")
    print("  minimal feature set: measured by bench/footprint.sh, which rebuilds "
          "and restores the default binary safely.")


if __name__ == "__main__":
    main()
