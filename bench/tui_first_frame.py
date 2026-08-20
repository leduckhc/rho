"""Time to first frame on a real pseudo-terminal.

`docs/benchmarks.md` measured this against a ratatui test backend and said plainly that
a real-terminal number needs a pty harness. This is that harness. It includes process
start, dynamic linking, raw-mode setup, and the alternate-screen switch, which the test
backend all skip.
"""
import os, pty, select, time, fcntl, struct, termios, statistics, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ptyharness import teardown

argv = [os.path.expanduser("~/Work/Vibe/rho/target/release/rho"), "--no-skills",
        "--provider", "openrouter", "--model", "anthropic/claude-haiku-4.5"]
os.environ["TERM"] = "xterm-256color"

# The brand in the header, in UTF-8. It is on screen in every frame the interface draws.
MARKER = "ρ".encode()

def one_run():
    start = time.perf_counter()
    pid, fd = pty.fork()
    if pid == 0:
        # A failed execv must end the child here. Without this guard the child unwinds
        # into the measurement loop, and two processes then drive the same benchmark.
        try:
            os.execv(argv[0], argv)
        finally:
            os._exit(127)
    try:
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
        seen = None
        deadline = time.perf_counter() + 5
        buf = bytearray()
        while time.perf_counter() < deadline:
            r, _, _ = select.select([fd], [], [], 0.05)
            if r:
                try:
                    chunk = os.read(fd, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                buf.extend(chunk)
                # The first frame has arrived once the header brand is on screen.
                #
                # The marker was `[idle]` until sprint 3 replaced the status vocabulary. A
                # stale marker does not fail: it collects nothing, and the harness then
                # divides by zero samples. So the marker is the brand, which the header
                # always draws, and a zero-sample run is a hard error below.
                if MARKER in buf:
                    seen = time.perf_counter() - start
                    break
    finally:
        # Teardown runs on every path, closes the master first, and has a deadline.
        # See the measurement in bench/ptyharness.py for why the order matters.
        if not teardown(pid, fd, timeout=5.0):
            print(f"warning: child {pid} outlived its teardown deadline", file=sys.stderr)
    return seen

samples = [s for s in (one_run() for _ in range(12)) if s is not None]
samples.sort()
ms = [s * 1000 for s in samples]
if not ms:
    raise SystemExit(
        "tui_first_frame.py measured nothing. No run reached the marker "
        f"{MARKER!r} inside the deadline. Either the interface failed to start, or the "
        "marker is stale. A benchmark that measures nothing must fail loudly."
    )
print(f"runs {len(ms)}")
print(f"min    {ms[0]:.1f} ms")
print(f"median {statistics.median(ms):.1f} ms")
print(f"max    {ms[-1]:.1f} ms")
