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

        # 2b. Fuzzy filter: `/model`, type `b`, Enter picks starred-b. See
        # `D-model-picker-allows-fuzzy-search-and-typed-fallback`.
        os.write(fd, b"/model\r")
        pump(fd, stream, 1.0)
        os.write(fd, b"b")   # type into query
        pump(fd, stream, 0.4)
        r2b = rows(screen)
        assert any("> b" in row for row in r2b), (
            f"the query prompt draws `> b`: {r2b}"
        )
        # Picker rows carry a `☆` or `★` glyph, so a filter check reads only picker
        # rows and not the banner (where the current id is drawn without a glyph).
        picker_rows = [row for row in r2b if "☆" in row or "★" in row]
        assert any("starred-b" in row for row in picker_rows), (
            f"starred-b survives the filter: {picker_rows}"
        )
        assert not any("starred-a" in row for row in picker_rows), (
            f"starred-a drops from the filter: {picker_rows}"
        )
        os.write(fd, b"\r")   # apply
        pump(fd, stream, 1.2)
        r2c = rows(screen)
        assert any("starred-b" in row and "·" in row for row in r2c), (
            f"banner switched to starred-b: {r2c}"
        )
        findings.append(
            "fuzzy typing `b` filtered to starred-b and Enter applied it"
        )

        # 2c. Typed fallback: a query that matches nothing still applies on Enter.
        os.write(fd, b"/model\r")
        pump(fd, stream, 1.0)
        for ch in b"vendor/unknown-model":
            os.write(fd, bytes([ch]))
        pump(fd, stream, 0.6)
        os.write(fd, b"\r")
        pump(fd, stream, 1.0)
        r2d = rows(screen)
        assert any("vendor/unknown-model" in row for row in r2d), (
            f"a no-match query applied verbatim: {r2d}"
        )
        findings.append(
            "a query with no match applied verbatim as the model id"
        )

        # Reset to a starred model so the star toggle probe uses one we can find.
        os.write(fd, b"/model starred-a\r")
        pump(fd, stream, 1.0)

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

        # 7. Star toggle. Open the picker, filter to `starred-b`, press Shift+Tab to
        #    unstar it, esc, reopen and confirm the file no longer names it.
        os.write(fd, b"/model\r")
        pump(fd, stream, 1.0)
        os.write(fd, b"b")   # filter to the one starred-b row
        pump(fd, stream, 0.4)
        os.write(fd, b"\x1b[Z")   # Shift+Tab (BackTab) toggles star
        pump(fd, stream, 0.8)
        os.write(fd, b"\x1b")
        pump(fd, stream, 0.4)
        with open(os.path.join(home, ".rho/starred-models.toml")) as f:
            body = f.read()
        assert "starred-b" not in body, (
            f"file no longer lists starred-b after Shift+Tab: {body!r}"
        )
        findings.append(
            "Shift+Tab on a filtered row removed starred-b from the file"
        )

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
