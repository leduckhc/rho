#!/usr/bin/env bash
#
# footprint.sh — measure the size and memory footprint of rho.
#
# This script reproduces every number in docs/benchmarks.md. Run it from the
# repository root. It measures four things:
#
#   1. release binary size, for the default and the minimal feature sets;
#   2. peak resident set size for 1, 51, and 101 idle sessions in one process;
#   3. the derived cost of one more session;
#   4. the time to the first rendered frame.
#
# Each memory and timing number is the median of three runs. A single sample is
# noise.
#
# Usage:
#   bash bench/footprint.sh            # readable table
#   bash bench/footprint.sh --json     # one JSON object, for CI to diff
#   bash bench/footprint.sh --no-build # measure existing artifacts, do not build
#
# The script fails loudly if a prerequisite is missing. It never prints a zero
# or a blank where a measurement failed.

set -euo pipefail

# ---------------------------------------------------------------------------
# Options.
# ---------------------------------------------------------------------------
JSON=0
BUILD=1
for arg in "$@"; do
  case "$arg" in
    --json) JSON=1 ;;
    --no-build) BUILD=0 ;;
    *) echo "footprint.sh: unknown option: $arg" >&2; exit 2 ;;
  esac
done

RUNS=3

# ---------------------------------------------------------------------------
# Fail loudly on a missing tool.
# ---------------------------------------------------------------------------
die() { echo "footprint.sh: $*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "required tool not found: $1"; }

need cargo
need python3
need awk
need sort
[ -x /usr/bin/time ] || die "required tool not found: /usr/bin/time"

# ---------------------------------------------------------------------------
# Detect the peak-RSS reporter and its unit.
#
# macOS: /usr/bin/time -l prints "maximum resident set size" in BYTES.
# GNU:   /usr/bin/time -v prints "Maximum resident set size" in KILOBYTES.
#
# We convert both to bytes, so the printed number means the same on each system.
# A wrong unit reports a 1024x error, so we choose the branch explicitly.
# ---------------------------------------------------------------------------
RSS_MODE=""
RSS_SOURCE=""
if /usr/bin/time -l true 2>&1 | grep -qi "maximum resident set size"; then
  RSS_MODE="macos"
  RSS_SOURCE="macOS /usr/bin/time -l, value in bytes"
elif /usr/bin/time -v true 2>&1 | grep -qi "Maximum resident set size"; then
  RSS_MODE="gnu"
  RSS_SOURCE="GNU /usr/bin/time -v, value in kilobytes, converted to bytes"
else
  die "no usable /usr/bin/time reporter for peak resident set size"
fi

# ---------------------------------------------------------------------------
# measure_rss_bytes CMD [ARGS...]
# Runs the command under the reporter and prints its peak RSS in bytes.
# The command's own stdout is discarded. The reporter writes to stderr.
# ---------------------------------------------------------------------------
measure_rss_bytes() {
  local out
  if [ "$RSS_MODE" = "macos" ]; then
    out=$(/usr/bin/time -l "$@" 2>&1 >/dev/null) || die "command failed under time: $*"
    # A macOS line reads: "  1245184  maximum resident set size". Unit is bytes.
    printf '%s\n' "$out" | awk '/maximum resident set size/ {print $1; exit}'
  else
    out=$(/usr/bin/time -v "$@" 2>&1 >/dev/null) || die "command failed under time: $*"
    # A GNU line reads: "Maximum resident set size (kbytes): 1234". Unit is KiB.
    local kb
    kb=$(printf '%s\n' "$out" | awk -F': ' '/Maximum resident set size/ {print $2; exit}')
    [ -n "$kb" ] || die "could not read Maximum resident set size from time output"
    printf '%s\n' "$(( kb * 1024 ))"
  fi
}

# ---------------------------------------------------------------------------
# median NUM...
# Prints the median of its numeric arguments.
# ---------------------------------------------------------------------------
median() {
  local n=$#
  [ "$n" -gt 0 ] || die "median needs at least one value"
  local mid=$(( n / 2 ))
  printf '%s\n' "$@" | sort -n | sed -n "$(( mid + 1 ))p"
}

# ---------------------------------------------------------------------------
# median_rss CMD [ARGS...]
# Runs the command RUNS times and prints the median peak RSS in bytes.
# ---------------------------------------------------------------------------
median_rss() {
  local samples=()
  local i value
  for (( i = 0; i < RUNS; i++ )); do
    value=$(measure_rss_bytes "$@")
    [ -n "$value" ] && [ "$value" -gt 0 ] 2>/dev/null || die "empty or zero RSS sample for: $*"
    samples+=("$value")
  done
  median "${samples[@]}"
}

# ---------------------------------------------------------------------------
# mib BYTES
# Prints a byte count as MiB with two decimals.
# ---------------------------------------------------------------------------
mib() { awk -v b="$1" 'BEGIN { printf "%.2f", b / 1048576 }'; }

# ---------------------------------------------------------------------------
# Build the artifacts, unless --no-build was given.
# The examples come from the default build, because the minimal build links
# fewer crates. We measure the default size first, then build minimal last.
# ---------------------------------------------------------------------------
BIN="target/release/rho"
EX_IDLE="target/release/examples/idle_session"
EX_MANY="target/release/examples/many_sessions"
EX_FRAME="target/release/examples/first_frame"

if [ "$BUILD" -eq 1 ]; then
  echo "footprint.sh: building the default release binary and examples..." >&2
  cargo build --release -p rho-cli >&2
  cargo build --release -p rho-cli --example idle_session >&2
  cargo build --release -p rho-cli --example many_sessions >&2
  cargo build --release -p rho-tui --example first_frame >&2
fi

# Every artifact must exist before we measure it.
[ -f "$BIN" ]     || die "release binary not built: $BIN (run without --no-build)"
[ -x "$EX_IDLE" ] || die "example not built: $EX_IDLE"
[ -x "$EX_MANY" ] || die "example not built: $EX_MANY"
[ -x "$EX_FRAME" ] || die "example not built: $EX_FRAME"

# ---------------------------------------------------------------------------
# 1. Release binary size, default feature set.
# ---------------------------------------------------------------------------
SIZE_DEFAULT=$(wc -c < "$BIN" | tr -d ' ')
[ -n "$SIZE_DEFAULT" ] && [ "$SIZE_DEFAULT" -gt 0 ] || die "could not read default binary size"

# ---------------------------------------------------------------------------
# 2. Peak RSS for 1, 51, and 101 sessions, and the first-frame RSS.
# ---------------------------------------------------------------------------
RSS_1=$(RHO_SESSIONS=1 median_rss "$EX_MANY")
RSS_51=$(RHO_SESSIONS=51 median_rss "$EX_MANY")
RSS_101=$(RHO_SESSIONS=101 median_rss "$EX_MANY")
RSS_IDLE=$(median_rss "$EX_IDLE")

# Cost of one more session, across the whole range from 1 to 101.
COST_PER_SESSION=$(( (RSS_101 - RSS_1) / 100 ))
[ "$COST_PER_SESSION" -gt 0 ] || die "cost per session came out non-positive; check the samples"

# ---------------------------------------------------------------------------
# 3. Time to the first frame, whole-process wall time.
# The example renders one frame into a ratatui TestBackend. We time the whole
# process, so the number includes process start and dynamic linking.
# ---------------------------------------------------------------------------
FIRST_FRAME_MS=$(RUNS="$RUNS" python3 - "$EX_FRAME" <<'PY'
import os, subprocess, sys, time
exe = sys.argv[1]
runs = max(3, int(os.environ.get("RUNS", "3")))
ts = []
for _ in range(runs):
    start = time.perf_counter()
    subprocess.run([exe], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    ts.append((time.perf_counter() - start) * 1000.0)
ts.sort()
print("%.2f" % ts[len(ts) // 2])
PY
)
[ -n "$FIRST_FRAME_MS" ] || die "first-frame measurement produced no value"

# ---------------------------------------------------------------------------
# 4. Release binary size, minimal feature set. Built last, since it overwrites
# the default binary at target/release/rho.
# ---------------------------------------------------------------------------
if [ "$BUILD" -eq 1 ]; then
  echo "footprint.sh: building the minimal release binary..." >&2
  cargo build --release -p rho-cli --no-default-features --features minimal >&2
  SIZE_MINIMAL=$(wc -c < "$BIN" | tr -d ' ')
else
  # Without a build we cannot hold both binaries at once. Report the default
  # size for both, and warn, rather than print a wrong number.
  SIZE_MINIMAL=""
fi

# ---------------------------------------------------------------------------
# Report.
# ---------------------------------------------------------------------------
PLATFORM="$(uname -s)-$(uname -m)"

if [ "$JSON" -eq 1 ]; then
  # A single JSON object, so CI can store it and a future run can diff it.
  minimal_json="null"
  [ -n "$SIZE_MINIMAL" ] && minimal_json="$SIZE_MINIMAL"
  cat <<JSON
{
  "platform": "$PLATFORM",
  "rss_unit_source": "$RSS_SOURCE",
  "runs_per_number": $RUNS,
  "binary_size_default_bytes": $SIZE_DEFAULT,
  "binary_size_minimal_bytes": $minimal_json,
  "idle_session_peak_rss_bytes": $RSS_IDLE,
  "peak_rss_1_session_bytes": $RSS_1,
  "peak_rss_51_sessions_bytes": $RSS_51,
  "peak_rss_101_sessions_bytes": $RSS_101,
  "cost_per_extra_session_bytes": $COST_PER_SESSION,
  "first_frame_median_ms": $FIRST_FRAME_MS
}
JSON
else
  echo "rho footprint. Platform: $PLATFORM."
  echo "Peak RSS source: $RSS_SOURCE."
  echo "Each memory and timing number is the median of $RUNS runs."
  echo
  echo "Release binary size:"
  printf '  default  %10s bytes  %6s MiB\n' "$SIZE_DEFAULT" "$(mib "$SIZE_DEFAULT")"
  if [ -n "$SIZE_MINIMAL" ]; then
    printf '  minimal  %10s bytes  %6s MiB\n' "$SIZE_MINIMAL" "$(mib "$SIZE_MINIMAL")"
  else
    echo "  minimal  not measured, because --no-build cannot hold two binaries"
  fi
  echo
  echo "Peak resident set size:"
  printf '  1 idle session       %10s bytes  %6s MiB\n' "$RSS_IDLE" "$(mib "$RSS_IDLE")"
  printf '  1 session process    %10s bytes  %6s MiB\n' "$RSS_1" "$(mib "$RSS_1")"
  printf '  51 sessions process  %10s bytes  %6s MiB\n' "$RSS_51" "$(mib "$RSS_51")"
  printf '  101 sessions process %10s bytes  %6s MiB\n' "$RSS_101" "$(mib "$RSS_101")"
  printf '  cost per more session %9s bytes  %6s MiB\n' "$COST_PER_SESSION" "$(mib "$COST_PER_SESSION")"
  echo
  echo "Time to first frame:"
  printf '  whole process median %s ms\n' "$FIRST_FRAME_MS"
fi
