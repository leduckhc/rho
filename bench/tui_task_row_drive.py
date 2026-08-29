#!/usr/bin/env python3
"""Drive the task row on a real pseudo-terminal, and check the bytes it writes.

This is the step 11 harness for `SPEC-the-task-row-draws-its-progress`. A `ratatui`
test backend cannot prove what matters here. It skips the real terminal, and it also
**hides one class of defect**: `ratatui` skips a zero-width grapheme, so an escape byte
never reaches a cell even when rho forgets to filter it, while the visible payload of
the sequence, `[2J`, does reach the screen. So the checks below read the raw byte
stream from the master side of a pty.

It runs `target/release/examples/task_row_drive`, which starts a real command through
the real `bash` tool, folds the real registry events with the real reducer, and draws
with the real renderer. Every scenario runs its task twice.

Run it from the repository root:

    cargo build --release --example task_row_drive -p rho-tui
    python3 bench/tui_task_row_drive.py

It prints one line per check and exits non-zero when any check fails.
"""

import fcntl
import os
import pty
import re
import select
import struct
import sys
import termios
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ptyharness import teardown

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BINARY = os.path.join(ROOT, "target/release/examples/task_row_drive")

# The scenarios, with the terminal size each one runs at.
RUNS = [
    ("progress", 100, 24),
    ("quiet", 100, 24),
    ("hostile", 100, 24),
    ("raw", 100, 24),
    ("missing", 100, 24),
    ("denied", 100, 24),
    # The narrow rule, live. Below 36 columns the progress leaves the row.
    ("progress", 32, 24),
    ("raw", 32, 24),
]

# The error role, 256-colour foreground 203. A failed task row must carry it.
ERROR_SGR = b"38;5;203"


def run(scenario, columns, rows, timeout=20.0):
    """Run one scenario on a pty and return every byte it wrote."""
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        try:
            os.execv(BINARY, [BINARY, scenario])
        finally:
            os._exit(127)
    out = bytearray()
    try:
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
        deadline = time.perf_counter() + timeout
        while time.perf_counter() < deadline:
            ready, _, _ = select.select([fd], [], [], 0.1)
            if not ready:
                continue
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            out.extend(chunk)
            if b"DONE|" in out:
                break
    finally:
        if not teardown(pid, fd, timeout=5.0):
            print(f"warning: child {pid} outlived its teardown deadline", file=sys.stderr)
    return bytes(out)


def row_lines(raw):
    """The `ROW|` snapshot lines the example printed."""
    text = raw.decode("utf-8", "replace")
    return [line[4:] for line in re.split(r"[\r\n]+", text) if line.startswith("ROW|")]


def bare_clear_screen(raw):
    """Every `[2J` that is a leaked payload, and not a real sequence or printed text.

    Two forms are legitimate. `ESC [2J` is `ratatui` clearing the screen, which is rho's
    own output. The literal text `\\u001b[2J` is the hostile command itself drawn on the
    row: the child's command line holds the JSON escape, so those nine characters are text
    a reader should see. A leak is neither: `[2J` with a real escape stripped off the front.
    """
    found = 0
    for match in re.finditer(rb"\[2J", raw):
        at = match.start()
        if at > 0 and raw[at - 1] == 0x1B:
            continue
        if raw[max(0, at - 6):at] == rb"\u001b":
            continue
        found += 1
    return found


def check(name, ok, detail=""):
    print(f"{'PASS' if ok else 'FAIL'}  {name}{(' — ' + detail) if detail else ''}")
    return 0 if ok else 1


def main():
    if not os.path.exists(BINARY):
        raise SystemExit(
            f"{BINARY} is missing. Run: cargo build --release --example task_row_drive -p rho-tui"
        )
    bad = 0
    for scenario, columns, rows in RUNS:
        raw = run(scenario, columns, rows)
        label = f"{scenario} at {columns} columns"
        lines = row_lines(raw)
        print(f"\n--- {label}: {len(raw)} bytes, {len(lines)} task rows")
        for line in lines:
            print(f"    {line.rstrip()!r}")

        bad += check(f"{label}: the run drew a task row", len(lines) >= 2, f"{len(lines)} rows")
        # Every row stays inside the terminal width.
        widest = max((len(line.rstrip()) for line in lines), default=0)
        bad += check(f"{label}: every row is inside the width", widest <= columns, f"widest {widest}")
        # No hostile payload reaches the terminal, in any scenario.
        bad += check(f"{label}: no OSC payload on the wire", b"pwned" not in raw)
        bad += check(f"{label}: no bell on the wire", b"\x07" not in raw)
        bad += check(f"{label}: no bare clear-screen payload", bare_clear_screen(raw) == 0)

        if scenario == "progress" and columns >= 36:
            bad += check(
                f"{label}: the percent draws",
                any("%" in line for line in lines),
            )
            bad += check(
                f"{label}: the separator draws",
                any("·" in line for line in lines),
            )
        if scenario == "progress" and columns < 36:
            bad += check(
                f"{label}: the progress leaves a narrow row",
                all("·" not in line for line in lines),
            )
            bad += check(
                f"{label}: the state word survives",
                all(("running" in line or "done" in line) for line in lines),
            )
        if scenario == "quiet":
            bad += check(
                f"{label}: no separator with no progress",
                all("·" not in line for line in lines),
            )
        if scenario in {"missing", "denied"}:
            bad += check(
                f"{label}: the row says it failed",
                any("failed" in line for line in lines),
            )
            bad += check(f"{label}: the failed row draws in the error role", ERROR_SGR in raw)
    print()
    if bad:
        print(f"FAILED CHECKS {bad}")
        return 1
    print("FAILED CHECKS 0")
    return 0


if __name__ == "__main__":
    sys.exit(main())
