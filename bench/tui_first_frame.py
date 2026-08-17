"""Time to first frame on a real pseudo-terminal.

`docs/benchmarks.md` measured this against a ratatui test backend and said plainly that
a real-terminal number needs a pty harness. This is that harness. It includes process
start, dynamic linking, raw-mode setup, and the alternate-screen switch, which the test
backend all skip.
"""
import os, pty, select, signal, time, fcntl, struct, termios, statistics

argv = [os.path.expanduser("~/Work/Vibe/rho/target/release/rho"), "--no-skills",
        "--provider", "openrouter", "--model", "anthropic/claude-haiku-4.5"]
os.environ["TERM"] = "xterm-256color"

def one_run():
    start = time.perf_counter()
    pid, fd = pty.fork()
    if pid == 0:
        os.execv(argv[0], argv)
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
            # The first frame has arrived once the status line is on screen.
            if b"[idle]" in buf:
                seen = time.perf_counter() - start
                break
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
    return seen

samples = [s for s in (one_run() for _ in range(12)) if s is not None]
samples.sort()
ms = [s * 1000 for s in samples]
print(f"runs {len(ms)}")
print(f"min    {ms[0]:.1f} ms")
print(f"median {statistics.median(ms):.1f} ms")
print(f"max    {ms[-1]:.1f} ms")
