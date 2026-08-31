"""Drive the real `rho` interface on a pty, and watch a background task row appear.

This is the step 11 harness for `SPEC-the-task-event-bridge`. It proves the one thing no
unit test can: that a task row reaches a real user, in the real binary, from a real model
turn, with no harness supplying the missing half.

Before this lane the answer was no. The reducer folded a task row, the renderer drew it,
and nothing in any shipped binary subscribed to the registry. So this script is the test
that would have failed for every previous version of rho.

It reads the screen through `pyte`, because a TUI overwrites cells and a flattened byte
stream cannot say what a user sees.

Run it from the repository root, with credentials in the environment. Bedrock also needs
a region, and rho says so plainly when it is missing:

    cargo build --release -p rho-cli
    AWS_REGION=us-east-1 python3 bench/tui_task_bridge_drive.py

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

import pyte

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ptyharness import teardown

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# `RHO_DRIVE_BINARY` points the drive at another build. It is how this script was run
# against the branch before the bridge, to prove the drive fails without it.
BINARY = os.environ.get("RHO_DRIVE_BINARY", os.path.join(ROOT, "target/release/rho"))
ROWS, COLS = 30, 100

PROVIDER = os.environ.get("RHO_DRIVE_PROVIDER", "bedrock")
MODEL = os.environ.get(
    "RHO_DRIVE_MODEL", "us.anthropic.claude-haiku-4-5-20251001-v1:0"
)

# The prompt asks for a command that runs long enough to be seen mid-flight, and that
# reports a percentage so the progress cell has something to draw.
PROMPT = (
    "Run this in the background with the bash tool, and then tell me the task id. "
    "Command: for i in 1 2 3 4 5 6 7 8; do echo \"progress: $((i*12))%\"; sleep 1; done"
)

# The same thing twice, and a failure. "Twice" has caught two defects in this project, and
# a task that fails must not draw as one that finished well.
#
# The command text carries no word that looks like a task state. A first draft used
# `exit 3`, and the check then matched the command instead of the state word: the row still
# read `running` and the check passed anyway. A live drive caught it, which is the whole
# reason this step exists.
SECOND_PROMPT = (
    "Run this one in the background too, with the bash tool, and tell me its id. "
    "Command: sleep 2; /usr/bin/false"
)

# `state_label` writes `failed (<code>)` for a non-zero exit. The state word follows the
# command, so the check reads the row tail and never the command.
# No trailing `\b`. A first draft had one, and it can never match after the closing
# bracket of `failed (1)`, so the check reported nothing while the row drew it
# correctly. The alternatives that end in a letter keep their own boundary.
FAILED_STATE = re.compile(r"\b(failed \(\d+\)|killed\b|timed out\b|canceled\b)")

failures = []


def check(name, ok, detail=""):
    print(f"{'PASS' if ok else 'FAIL'}  {name}{(' — ' + detail) if detail else ''}")
    if not ok:
        failures.append(name)


def main():
    if not os.path.exists(BINARY):
        print(f"FAIL  the binary is missing: {BINARY}")
        print("      run: cargo build --release -p rho-cli")
        return 1
    if PROVIDER == "bedrock" and not os.environ.get("AWS_REGION"):
        # rho reports this itself and exits, which closes the pty and makes the first
        # write fail with a confusing errno. Say the real reason here instead.
        print("FAIL  bedrock needs AWS_REGION, for example us-east-1")
        return 1

    screen = pyte.Screen(COLS, ROWS)
    stream = pyte.ByteStream(screen)
    os.environ["TERM"] = "xterm-256color"

    pid, fd = pty.fork()
    if pid == 0:
        # A failed execv must end the child here. Without this guard the child unwinds
        # into the parent's module code, and two processes then drive one script.
        try:
            os.execv(
                BINARY,
                [
                    BINARY,
                    "--no-skills",
                    "--provider",
                    PROVIDER,
                    "--model",
                    MODEL,
                    "--no-mouse",
                ],
            )
        finally:
            os._exit(127)

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))

    def pump(seconds):
        end = time.time() + seconds
        while time.time() < end:
            ready, _, _ = select.select([fd], [], [], 0.2)
            if not ready:
                continue
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                return
            if not chunk:
                return
            stream.feed(chunk)

    def rows():
        return [line.rstrip() for line in screen.display]

    def task_rows():
        # A task row starts with the label `task `, which is the row grammar in
        # `SPEC-the-task-row-draws-its-progress`. A first draft matched any line holding
        # the word "task", and the model's own prose ("Your task id is task-1") then
        # counted as a row. Three checks passed on a build with no bridge at all.
        return [line for line in rows() if line.strip().startswith("task ")]

    try:
        pump(3.0)
        os.write(fd, PROMPT.encode() + b"\r")

        # Watch for a task row while the turn runs. The row must appear on its own, from
        # the registry, with no harness bridging anything.
        seen_running = None
        seen_progress = None
        deadline = time.time() + 75
        while time.time() < deadline:
            pump(1.0)
            for line in task_rows():
                if seen_running is None and "running" in line:
                    seen_running = line
                if seen_progress is None and re.search(r"\d+%", line):
                    seen_progress = line
            if seen_running and seen_progress:
                break

        check(
            "a background task draws a row in the real interface",
            seen_running is not None,
            repr(seen_running),
        )
        check(
            "the row draws the task's progress",
            seen_progress is not None,
            repr(seen_progress),
        )

        # The row must finish. This is the half a run-scoped stream would lose: the turn
        # has ended and rho is idle at the composer while the command still runs.
        finished = None
        deadline = time.time() + 60
        while time.time() < deadline:
            pump(1.0)
            for line in task_rows():
                if re.search(r"\b(done|exited|failed|ok)\b", line):
                    finished = line
                    break
            if finished:
                break

        check(
            "the row finishes while rho sits idle at the composer",
            finished is not None,
            repr(finished),
        )

        # ---- The same thing twice, and the failure path. ----------------------------
        os.write(fd, SECOND_PROMPT.encode() + b"\r")
        two_rows = None
        failed_row = None
        deadline = time.time() + 90
        while time.time() < deadline:
            pump(1.0)
            current = task_rows()
            if len(current) >= 2:
                two_rows = current
            for line in current:
                if FAILED_STATE.search(line):
                    failed_row = line
            if two_rows and failed_row:
                break

        check(
            "a second background task draws its own row",
            two_rows is not None,
            f"{len(two_rows) if two_rows else 0} rows",
        )
        check(
            "a task that fails draws a failed state, not a finished one",
            failed_row is not None and "done" not in failed_row,
            repr(failed_row),
        )
        if two_rows:
            check(
                "the first row is not replaced by the second",
                len({line for line in two_rows}) >= 2,
                repr(two_rows),
            )

        print("\n--- the screen at the end ---")
        for line in rows():
            if line:
                print(f"    {line}")

        os.write(fd, b"\x03")
        pump(0.5)
        os.write(fd, b"\x03")
        pump(0.5)
    finally:
        teardown(pid, fd)

    print(f"\nFAILED CHECKS {len(failures)}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
