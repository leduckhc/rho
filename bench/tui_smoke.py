"""Drive rho's TUI in a real pty and render it through a terminal emulator.

A flattened byte stream cannot show whether the screen is correct, because a TUI
overwrites cells. `pyte` models the screen, so what it reports is what a user sees.
"""
import os, pty, select, time, fcntl, struct, termios, sys
import pyte

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ptyharness import teardown

ROWS, COLS = 30, 100
argv = [os.path.expanduser("~/Work/Vibe/rho/target/release/rho"),
        "--no-skills", "--provider", "openrouter",
        "--model", "anthropic/claude-haiku-4.5"]
os.environ["TERM"] = "xterm-256color"

screen = pyte.Screen(COLS, ROWS)
stream = pyte.ByteStream(screen)

pid, fd = pty.fork()
if pid == 0:
    # A failed execv must end the child here. Without this guard the child unwinds into
    # the parent's module-level code, and two processes then drive the same script.
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

def snap(tag):
    lines = [l.rstrip() for l in screen.display]
    body = [l for l in lines if l]
    print(f"--- {tag} ({len(body)} non-empty rows)")
    for l in body:
        print("   |" + l)

# Every step sits inside the try block, so teardown runs on every exit path. A
# KeyboardInterrupt during the 22-second pump used to skip teardown and orphan the child,
# with the pty master still open. That is the shape of the 30-minute hang.
try:
    pump(2.0)
    snap("first frame, idle")
    os.write(fd, b"say the word banana\r")
    pump(22.0)
    snap("after one real turn")
    os.write(fd, b"\x03"); pump(1.0)
    os.write(fd, b"\x03"); pump(1.5)
    snap("after two Ctrl-C")
finally:
    # Teardown closes the master first, and has a deadline. The other order lets a child
    # blocked in a terminal write survive SIGKILL. See the note in bench/ptyharness.py.
    if not teardown(pid, fd, timeout=5.0):
        print(f"warning: child {pid} outlived its teardown deadline", file=sys.stderr)
