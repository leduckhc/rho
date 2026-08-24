"""Capture what rho's unbuilt slash commands really print.

Four guide pages quote a refusal string. The string came from a format literal in the
source, and nobody had watched rho print it. This drives the real TUI in a pty, types each
command, and reports the rows that changed.
"""

import fcntl
import os
import pty
import select
import struct
import sys
import termios
import time

import pyte

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ptyharness import teardown

ROWS, COLS = 40, 120
RHO = os.path.expanduser("~/Work/Vibe/rho/target/release/rho")
argv = [RHO, "--no-skills", "--provider", "openrouter", "--model", "anthropic/claude-haiku-4.5"]
os.environ["TERM"] = "xterm-256color"

screen = pyte.Screen(COLS, ROWS)
stream = pyte.ByteStream(screen)

pid, fd = pty.fork()
if pid == 0:
    try:
        os.execv(argv[0], argv)
    finally:
        os._exit(127)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))


def pump(seconds):
    end = time.time() + seconds
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.2)
        if r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                return
            if not chunk:
                return
            stream.feed(chunk)


def rows():
    return [l.rstrip() for l in screen.display if l.strip()]


def show(tag, keep=None):
    print(f"--- {tag}")
    for line in rows():
        if keep is None or keep.lower() in line.lower():
            print("   |" + line)


try:
    pump(2.5)
    show("idle frame")

    os.write(fd, b"/")
    pump(1.2)
    show("the command list")
    os.write(fd, b"\x1b")
    pump(0.8)

    for command in ("/model", "/sessions"):
        os.write(fd, command.encode() + b"\r")
        pump(1.5)
        print(f"--- after {command}")
        for line in rows():
            if "built" in line.lower() or command in line:
                print("   |" + line)

    # Walk the guide, page by page, the way a reader does.
    os.write(fd, b"/guide\r")
    pump(1.5)
    show("guide page 1")
    os.write(fd, b" ")
    pump(1.0)
    show("guide page 2 (space)")
    os.write(fd, b"\x1b[C")
    pump(1.0)
    show("guide page 3 (right)")
    os.write(fd, b"\x1b[C")
    pump(1.0)
    print("--- a next press on the last page: still page 3 below")
    show("guide last page holds")
    os.write(fd, b"\x1b[D")
    pump(1.0)
    show("guide back to page 2 (left)")
    os.write(fd, b"\x1b")
    pump(1.0)
    show("after esc")

    # Ctrl+O claims to expand a tool row. Nothing should change.
    before = rows()
    os.write(fd, b"\x0f")
    pump(1.0)
    print(f"--- Ctrl+O changed the screen: {rows() != before}")

    os.write(fd, b"\x03")
    pump(1.0)
    os.write(fd, b"\x03")
    pump(1.5)
finally:
    if not teardown(pid, fd, timeout=5.0):
        print(f"warning: child {pid} outlived its teardown deadline", file=sys.stderr)
