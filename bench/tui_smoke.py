"""Drive rho's TUI in a real pty and render it through a terminal emulator.

A flattened byte stream cannot show whether the screen is correct, because a TUI
overwrites cells. `pyte` models the screen, so what it reports is what a user sees.
"""
import os, pty, select, signal, time, fcntl, struct, termios
import pyte

ROWS, COLS = 30, 100
argv = [os.path.expanduser("~/Work/Vibe/rho/target/release/rho"),
        "--no-skills", "--provider", "openrouter",
        "--model", "anthropic/claude-haiku-4.5"]
os.environ["TERM"] = "xterm-256color"

screen = pyte.Screen(COLS, ROWS)
stream = pyte.ByteStream(screen)

pid, fd = pty.fork()
if pid == 0:
    os.execv(argv[0], argv)
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

pump(2.0)
snap("first frame, idle")
os.write(fd, b"say the word banana\r")
pump(22.0)
snap("after one real turn")
os.write(fd, b"\x03"); pump(1.0)
os.write(fd, b"\x03"); pump(1.5)
snap("after two Ctrl-C")

try:
    os.kill(pid, signal.SIGKILL)
except ProcessLookupError:
    pass
os.waitpid(pid, 0)
