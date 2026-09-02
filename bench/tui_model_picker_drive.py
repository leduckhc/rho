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
    # Start with an empty starred file so the drive proves that the picker still shows
    # rows from the provider suggestion list. See
    # D-the-picker-seeds-from-a-per-provider-suggestion-list.
    with open(os.path.join(home, ".rho/starred-models.toml"), "w") as f:
        f.write("starred = []\n")

    pid, fd = spawn(home)
    screen = pyte.Screen(COLS, ROWS)
    stream = pyte.ByteStream(screen)
    findings = []
    try:
        pump(fd, stream, 2.5)
        assert any("seed-model" in r for r in rows(screen)), "banner names the seed model"

        # 1. `/model` opens the picker with the current model and openrouter suggestions.
        os.write(fd, b"/model\r")
        pump(fd, stream, 1.2)
        r1 = rows(screen)
        assert any("seed-model" in row and "current" in row for row in r1), (
            f"picker header shows the current row: {r1}"
        )
        # The openrouter suggestion list should include a couple of well-known ids.
        assert any("claude-sonnet-4.5" in row for row in r1), (
            f"a suggestion row (sonnet) shows: {r1}"
        )
        assert any("gpt-5" in row for row in r1), (
            f"a suggestion row (gpt-5) shows: {r1}"
        )
        # Footer hint names the picker keys and has a space after `ready`.
        footer = next((row for row in r1 if "ready" in row), "")
        assert "ready " in footer, (
            f"footer separates `ready` from the hint with a space: {footer!r}"
        )
        assert "enter pick" in footer or "tab effort" in footer, (
            f"picker footer hint drew: {footer!r}"
        )
        findings.append(
            "picker opened with current + openrouter suggestions on an empty starred file"
        )
        # esc closes.
        os.write(fd, b"\x1b")
        pump(fd, stream, 0.6)

        # 2. Move selection and Enter applies. Down to a suggestion row and Enter.
        os.write(fd, b"/model\r")
        pump(fd, stream, 1.0)
        os.write(fd, b"\x1b[B")  # Down
        pump(fd, stream, 0.4)
        os.write(fd, b"\r")     # Enter
        pump(fd, stream, 1.2)
        r2 = rows(screen)
        # The banner reports the new model (whichever suggestion was at row 1).
        new_model = None
        for line in r2:
            if " · openrouter" in line:
                # Banner format: "... · <model> · openrouter"
                parts = [p.strip() for p in line.split("·")]
                if len(parts) >= 3:
                    new_model = parts[-2]
                    break
        assert new_model is not None and new_model != "seed-model", (
            f"banner switched to a suggestion: {r2}"
        )
        findings.append(f"Enter applied suggestion {new_model}; banner updated")

        # 2b. Fuzzy filter: `/model`, type `sonnet`, Enter picks the sonnet suggestion.
        # See `D-model-picker-allows-fuzzy-search-and-typed-fallback`.
        os.write(fd, b"/model\r")
        pump(fd, stream, 1.0)
        os.write(fd, b"sonnet")   # type into query
        pump(fd, stream, 0.4)
        r2b = rows(screen)
        assert any("> sonnet" in row for row in r2b), (
            f"the query prompt draws `> sonnet`: {r2b}"
        )
        # Picker rows carry a `☆` or `★` glyph, so a filter check reads only picker rows.
        picker_rows = [row for row in r2b if "☆" in row or "★" in row]
        assert any("claude-sonnet-4.5" in row for row in picker_rows), (
            f"sonnet survives the filter: {picker_rows}"
        )
        assert not any("gpt-5" in row for row in picker_rows), (
            f"gpt-5 drops from the filter: {picker_rows}"
        )
        os.write(fd, b"\r")   # apply
        pump(fd, stream, 1.2)
        r2c = rows(screen)
        assert any("claude-sonnet-4.5" in row and "·" in row for row in r2c), (
            f"banner switched to claude-sonnet-4.5: {r2c}"
        )
        findings.append(
            "fuzzy typing `sonnet` filtered to claude-sonnet-4.5 and Enter applied it"
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

        # 7. Star toggle. Open the picker, filter to the sonnet suggestion, press
        #    Shift+Tab to star it, esc, reopen and confirm the file now names it.
        os.write(fd, b"/model\r")
        pump(fd, stream, 1.0)
        os.write(fd, b"sonnet")   # filter to claude-sonnet-4.5
        pump(fd, stream, 0.4)
        os.write(fd, b"\x1b[Z")   # Shift+Tab (BackTab) toggles star
        pump(fd, stream, 0.8)
        os.write(fd, b"\x1b")
        pump(fd, stream, 0.4)
        with open(os.path.join(home, ".rho/starred-models.toml")) as f:
            body = f.read()
        assert "claude-sonnet-4.5" in body, (
            f"file now lists claude-sonnet-4.5 after Shift+Tab: {body!r}"
        )
        findings.append(
            "Shift+Tab on a suggestion starred claude-sonnet-4.5 in the file"
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
