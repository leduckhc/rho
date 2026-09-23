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
import re
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


def picker_rows(lines):
    """Parse visible picker model rows as (id, vendor, raw_line)."""
    out = []
    for line in lines:
        match = re.search(r"[☆★]\s+([^\s]+)(?:\s+([^\s]+))?$", line)
        if not match:
            continue
        model_id = match.group(1)
        vendor = match.group(2)
        out.append((model_id, vendor, line))
    return out


def find_cell(screen, needle, inner=None):
    """Find the first (y, x) where `needle` (or `inner` inside it) begins."""
    target = inner or needle
    for y, line in enumerate(screen.display):
        outer = line.find(needle)
        if outer == -1:
            continue
        x = line.find(target, outer)
        if x != -1:
            return y, x
    return None, None


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

        # 1. `/model` opens the picker with the current model and model rows.
        os.write(fd, b"/model\r")
        pump(fd, stream, 1.2)
        r1 = rows(screen)
        assert any("seed-model" in row and "current" in row for row in r1), (
            f"picker header shows the current row: {r1}"
        )
        parsed = picker_rows(r1)
        assert any(model_id != "seed-model" for model_id, _, _ in parsed), (
            f"the picker has at least one selectable non-current row: {r1}"
        )
        assert not any("catalog load ended unexpectedly" in row for row in r1), (
            f"the picker should not draw a false catalog-load error: {r1}"
        )
        # Footer hint names the picker keys and has a space after `ready`.
        footer = next((row for row in r1 if "ready" in row), "")
        assert "ready " in footer, (
            f"footer separates `ready` from the hint with a space: {footer!r}"
        )
        assert "enter pick" in footer or "tab effort" in footer, (
            f"picker footer hint drew: {footer!r}"
        )
        # Pick one concrete catalog id for filter/star assertions later.
        target_id = next(
            model_id for model_id, _, _ in parsed if model_id != "seed-model"
        )
        target_vendor = next(
            (vendor for model_id, vendor, _ in parsed if model_id == target_id),
            None,
        )
        findings.append(
            "picker opened with current + selectable model rows and no false load error"
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

        # 2b. Filter: `/model`, type one concrete id, Enter picks it.
        # See `D-model-picker-allows-fuzzy-search-and-typed-fallback`.
        os.write(fd, b"/model\r")
        pump(fd, stream, 1.0)
        os.write(fd, target_id.encode())
        pump(fd, stream, 0.4)
        r2b = rows(screen)
        assert any(f"> {target_id}" in row for row in r2b), (
            f"the query prompt draws the typed id: {r2b}"
        )
        # Picker rows carry a `☆` or `★` glyph, so a filter check reads only picker rows.
        filtered_rows = [row for row in r2b if "☆" in row or "★" in row]
        assert any(target_id in row for row in filtered_rows), (
            f"the typed id survives the filter: {filtered_rows}"
        )
        os.write(fd, b"\r")   # apply
        pump(fd, stream, 1.2)
        r2c = rows(screen)
        assert any(target_id in row and "·" in row for row in r2c), (
            f"banner switched to the filtered id: {r2c}"
        )
        findings.append(
            "typed-id filtering narrowed the picker and Enter applied that id"
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

        # 7. Star toggle. Open the picker, filter to one known row, press
        #    Shift+Tab to star it, esc, reopen and confirm the file now names it.
        os.write(fd, b"/model\r")
        pump(fd, stream, 1.0)
        os.write(fd, target_id.encode())
        pump(fd, stream, 0.4)
        os.write(fd, b"\x1b[Z")   # Shift+Tab (BackTab) toggles star
        pump(fd, stream, 0.8)
        os.write(fd, b"\x1b")
        pump(fd, stream, 0.4)
        with open(os.path.join(home, ".rho/starred-models.toml")) as f:
            body = f.read()
        assert target_id in body, (
            f"file now lists the starred id after Shift+Tab: {body!r}"
        )
        findings.append(
            "Shift+Tab on a picker row persisted that star in the file"
        )

        # 8. Reopen the picker. A starred row now exists, so a `starred` header draws. A
        #    suggestion row draws its vendor dim. See
        #    `SPEC-the-model-picker-groups-and-labels-rows` sections 3 and 4.
        os.write(fd, b"/model\r")
        pump(fd, stream, 1.0)
        r8 = rows(screen)
        assert any(row.strip() == "starred" for row in r8), (
            f"a starred header draws after a star: {r8}"
        )
        # If this row has a vendor label, it should draw dim, not default colour.
        if target_vendor is not None:
            y, x = find_cell(screen, target_id, inner=target_vendor)
            assert y is not None, f"a vendor label draws for the starred row: {r8}"
            vendor_fg = screen.buffer[y][x].fg
            assert vendor_fg != "default", (
                f"the vendor cell draws dim, not default colour: fg={vendor_fg!r}, row={r8}"
            )
            findings.append(
                f"reopened picker drew a `starred` header and a dim vendor (fg={vendor_fg})"
            )
        else:
            findings.append("reopened picker drew a `starred` header")
        os.write(fd, b"\x1b")
        pump(fd, stream, 0.4)

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
