"""Drive the real rho binary through the `/model` picker in a pty.

The probe covers the four surfaces the feature ships:

- `/model` alone opens the picker.
- `/model <id>` applies without opening a picker.
- `/effort <level>` sets the effort.
- `/effort` alone reports the current level as a notice.

It reads the screen with pyte and asserts on rendered cells, per the skill notes.
"""

import fcntl
import os
import pty
import select
import shutil
import struct
import sys
import tempfile
import termios
import time

import pyte

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ptyharness import teardown

ROWS, COLS = 40, 120
RHO = os.environ.get(
    "RHO_BINARY", os.path.expanduser("~/Work/Vibe/rho/target/release/rho")
)


def rows(screen):
    return [line.rstrip() for line in screen.display if line.strip()]


def pump(fd, stream, seconds):
    end = time.time() + seconds
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.1)
        if r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                return
            if not chunk:
                return
            stream.feed(chunk)


def spawn(home):
    os.environ["TERM"] = "xterm-256color"
    os.environ["HOME"] = home
    argv = [
        RHO,
        "--no-skills",
        "--no-agents",
        "--provider",
        "openrouter",
        "--model",
        "seed-model",
    ]
    pid, fd = pty.fork()
    if pid == 0:
        try:
            os.execv(argv[0], argv)
        finally:
            os._exit(127)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    return pid, fd


def main():
    home = tempfile.mkdtemp(prefix="rho-mp-")
    os.makedirs(os.path.join(home, ".rho"), exist_ok=True)
    # Pre-seed a starred file so the picker has more than one row.
    with open(os.path.join(home, ".rho/starred-models.toml"), "w") as f:
        f.write('starred = ["starred-a", "starred-b"]\n')

    pid, fd = spawn(home)
    screen = pyte.Screen(COLS, ROWS)
    stream = pyte.ByteStream(screen)
    findings = []
    try:
        pump(fd, stream, 2.5)
        assert any("seed-model" in r for r in rows(screen)), "banner names the seed model"

        # 1. `/model` opens the picker with three rows.
        os.write(fd, b"/model\r")
        pump(fd, stream, 1.2)
        r1 = rows(screen)
        assert any("seed-model" in row and "current" in row for row in r1), (
            f"picker header shows the current row: {r1}"
        )
        assert any("starred-a" in row for row in r1), f"starred-a row shows: {r1}"
        assert any("starred-b" in row for row in r1), f"starred-b row shows: {r1}"
        # Footer hint names the picker keys.
        assert any("choose" in row and "star" in row for row in r1), (
            f"picker footer hint drew: {r1}"
        )
        findings.append("picker opened with current + 2 starred rows")
        # esc closes.
        os.write(fd, b"\x1b")
        pump(fd, stream, 0.6)

        # 2. Move selection and Enter applies. Down to starred-a and Enter.
        os.write(fd, b"/model\r")
        pump(fd, stream, 1.0)
        os.write(fd, b"\x1b[B")  # Down
        pump(fd, stream, 0.4)
        os.write(fd, b"\r")     # Enter
        pump(fd, stream, 1.2)
        r2 = rows(screen)
        # The banner reports the new model.
        assert any("starred-a" in row for row in r2), (
            f"banner switched to starred-a: {r2}"
        )
        findings.append("Enter applied starred-a; banner updated")

        # 3. `/model direct-id` applies immediately.
        os.write(fd, b"/model direct-id\r")
        pump(fd, stream, 1.2)
        r3 = rows(screen)
        assert any("direct-id" in row for row in r3), (
            f"banner switched to direct-id: {r3}"
        )
        assert any("model set to direct-id" in row for row in r3), (
            f"notice named the new model: {r3}"
        )
        findings.append("/model direct-id applied without a picker")

        # 4. `/effort high` reports it.
        os.write(fd, b"/effort high\r")
        pump(fd, stream, 1.2)
        r4 = rows(screen)
        assert any("effort: high" in row for row in r4), (
            f"notice named effort high: {r4}"
        )
        findings.append("/effort high produced a notice")

        # 5. `/effort` alone shows current.
        os.write(fd, b"/effort\r")
        pump(fd, stream, 1.2)
        r5 = rows(screen)
        assert any("effort: high" in row for row in r5), (
            f"the second /effort call still reads `high`: {r5}"
        )
        findings.append("/effort alone reported the current level")

        # 6. `/effort loud` reports an error naming the valid levels.
        os.write(fd, b"/effort loud\r")
        pump(fd, stream, 1.2)
        r6 = rows(screen)
        assert any("xhigh" in row for row in r6), (
            f"error names the valid levels: {r6}"
        )
        findings.append("/effort loud pushed an error naming the valid levels")

        # 7. Star toggle. Open the picker, press `*` on the second row, esc, reopen,
        #    the row is no longer starred.
        os.write(fd, b"/model\r")
        pump(fd, stream, 1.0)
        # Move to the first starred row (`direct-id` is now current; `starred-a` and
        # `starred-b` follow).
        os.write(fd, b"\x1b[B")
        pump(fd, stream, 0.3)
        os.write(fd, b"*")   # unstar starred-a
        pump(fd, stream, 0.6)
        os.write(fd, b"\x1b")
        pump(fd, stream, 0.4)
        with open(os.path.join(home, ".rho/starred-models.toml")) as f:
            body = f.read()
        assert "starred-a" not in body, (
            f"file no longer lists starred-a: {body!r}"
        )
        assert "starred-b" in body, (
            f"file still lists starred-b: {body!r}"
        )
        findings.append("* removed starred-a from ~/.rho/starred-models.toml")

        # Quit with ctrl-c twice.
        os.write(fd, b"\x03")
        pump(fd, stream, 0.4)
        os.write(fd, b"\x03")
        pump(fd, stream, 1.0)
    finally:
        if not teardown(pid, fd, timeout=5.0):
            print(f"warning: rho child {pid} outlived its teardown", file=sys.stderr)
        shutil.rmtree(home, ignore_errors=True)

    print("--- FINDINGS")
    for finding in findings:
        print("   " + finding)
    print(f"--- OK · {len(findings)} checks passed")


if __name__ == "__main__":
    main()
