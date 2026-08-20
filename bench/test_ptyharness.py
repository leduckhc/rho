"""Tests for the bounded teardown in `bench/ptyharness.py`.

Run it directly: `python3 bench/test_ptyharness.py`.

The defect these tests pin: teardown that waits with no bound, in the wrong order. A child
blocked writing into a full pty buffer cannot finish dying, not even under `SIGKILL`, while
the master stays open and nobody drains it. An unbounded `os.waitpid` then hangs the
harness, and the hang travels up to whatever runs it.

Two rules hold in this file.

A hang is a failure. The runner arms `SIGALRM` around every test, so a test that blocks
reports `FAIL` instead of stalling. The defect under test is a hang, so a runner that can
only report an exception is not enough.

Every fork is registered in `FORKED`. The runner checks each pid after each test, and
reports a child left behind. A leaked child is a defect in the test, not a detail.
"""
import os
import pty
import signal
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ptyharness

FORKED = []


def fork_child(body):
    """Fork a pty child that runs `body`, and register the pid for the leak check."""
    pid, fd = pty.fork()
    if pid == 0:
        try:
            body()
        finally:
            os._exit(0)
    FORKED.append(pid)
    return pid, fd


def test_reap_collects_a_live_child():
    """`reap` kills a running child, and reports success."""
    pid, fd = fork_child(lambda: time.sleep(600))
    start = time.perf_counter()
    ok = ptyharness.reap(pid, timeout=5.0)
    took = time.perf_counter() - start
    ptyharness.close_pty(fd)
    assert ok is True, "reap must report success for a child it can kill"
    assert took < 5.0, f"reap took {took:.2f}s for a killable child"


def test_reap_uses_a_signal_the_child_cannot_catch():
    """`reap` must send `SIGKILL`, and not a signal a child can ignore.

    The whole design rests on an uncatchable signal. A child that ignores `SIGTERM` proves
    it. Without this test, changing `SIGKILL` to `SIGTERM` keeps the suite green, and a
    `rho` with a `SIGTERM` handler then brings the hang back.
    """

    def ignores_sigterm():
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        signal.signal(signal.SIGHUP, signal.SIG_IGN)
        time.sleep(600)

    pid, fd = fork_child(ignores_sigterm)
    time.sleep(0.2)  # Let the child install its handlers before the signal arrives.
    ok = ptyharness.reap(pid, timeout=3.0)
    ptyharness.close_pty(fd)
    assert ok is True, (
        "reap failed against a child that ignores SIGTERM. The signal must be SIGKILL, "
        "which no process can catch, block, or ignore."
    )


def test_reap_returns_false_and_does_not_hang_on_a_wedged_child():
    """`reap` gives up at its deadline, rather than waiting with no bound.

    The kill signal is held back by passing `kill=False`, so the child stays alive for
    the whole call. That is the same shape as a child wedged in the exiting state, and it
    needs no unkillable process.
    """
    pid, fd = fork_child(lambda: time.sleep(600))
    start = time.perf_counter()
    ok = ptyharness.reap(pid, timeout=0.3, kill=False)
    took = time.perf_counter() - start
    assert ptyharness.teardown(pid, fd, timeout=5.0) is True, "cleanup must not leak a child"
    assert ok is False, "reap must report failure when the child outlives the deadline"
    assert took < 2.0, f"reap ignored its 0.3s deadline and took {took:.2f}s"
    assert took >= 0.3, f"reap returned after {took:.2f}s, before its own deadline"


def test_reap_accepts_a_child_that_is_already_gone():
    """`reap` reports success when another party reaped the child first.

    It must also send no signal in that case. A pid is reused, so a kill after somebody
    else reaped the child can reach an unrelated process.
    """
    pid, fd = pty.fork()
    if pid == 0:
        os._exit(0)
    os.waitpid(pid, 0)  # Another party reaps it first. The child wrote nothing.

    signalled = []
    real_kill = os.kill

    def spy_kill(target, sig):
        signalled.append((target, sig))
        return real_kill(target, sig)

    os.kill = spy_kill
    try:
        assert ptyharness.reap(pid, timeout=0.2) is True
    finally:
        os.kill = real_kill
    ptyharness.close_pty(fd)
    assert signalled == [], (
        f"reap signalled {signalled} for a pid it no longer owns. The operating system "
        "reuses a pid, so that signal can reach an unrelated process."
    )


def test_reap_rejects_a_deadline_of_zero():
    """`reap` refuses a deadline that cannot hold its own contract.

    A child killed microseconds ago is not reaped yet, so `timeout=0` reports `False` for
    a perfectly killable child. That is a trap, so the value is refused at the door.
    """
    for bad in (0, 0.0, -1):
        try:
            ptyharness.reap(os.getpid(), timeout=bad, kill=False)
        except ValueError:
            continue
        raise AssertionError(f"reap accepted timeout={bad!r}, which cannot be honoured")


def test_close_pty_is_safe_twice():
    """A double close must not raise, because teardown runs on every path."""
    pid, fd = fork_child(lambda: None)
    ptyharness.reap(pid, timeout=1.0)
    ptyharness.close_pty(fd)
    ptyharness.close_pty(fd)


def test_teardown_closes_the_master_before_it_waits():
    """`teardown` must close the pty master before it waits for the child.

    This pins the invariant behind the 30-minute hang. Measured with
    `target/release/rho` on 2026-08-18, three runs of each order:

        close master, then SIGKILL       -> 13 ms, 13 ms, 13 ms
        SIGKILL, master open, no drain   -> 11 ms, timeout(>8s), timeout(>8s)
        SIGKILL, master open, keep drain -> 0 ms, 0 ms, 0 ms

    The assertion checks the order and not the clock, because the wedge needs a real
    multi-threaded writer to show up. A Python stand-in child reaches only 605 ms, so a
    timing threshold here would pass against the very defect it is written for.
    """

    def writes_until_it_cannot():
        blob = b"x" * 4096
        try:
            while True:
                os.write(1, blob)
        except OSError:
            pass

    pid, fd = fork_child(writes_until_it_cannot)
    time.sleep(0.2)  # Let the buffer fill. The parent never reads it.

    seen = {}
    real_reap = ptyharness.reap

    def spy(spy_pid, timeout=5.0, poll=0.02, kill=True):
        try:
            os.fstat(fd)
            seen["master_closed"] = False
        except OSError:
            seen["master_closed"] = True
        return real_reap(spy_pid, timeout=timeout, poll=poll, kill=kill)

    ptyharness.reap = spy
    try:
        ok = ptyharness.teardown(pid, fd, timeout=3.0)
    finally:
        ptyharness.reap = real_reap

    assert seen.get("master_closed") is True, (
        "teardown waited for the child while the pty master was still open. That is the "
        "order that hung bench/tui_first_frame.py for 30 minutes."
    )
    assert ok is True, "teardown must reap the child inside its deadline"


def test_teardown_reports_a_child_it_could_not_reap():
    """`teardown` returns what `reap` returns, and never a fixed `True`.

    Both harnesses print their warning behind `if not teardown(...)`. So a `teardown` that
    always returns `True` silences the warning, and a real leak then passes in silence.
    """
    real_reap = ptyharness.reap
    ptyharness.reap = lambda *a, **k: False
    try:
        pid, fd = fork_child(lambda: None)
        FORKED.remove(pid)  # The stub never reaps, so this test cleans up by hand.
        assert ptyharness.teardown(pid, fd, timeout=1.0) is False, (
            "teardown hid a failed reap behind a True. The warning in both harnesses "
            "depends on that value."
        )
    finally:
        ptyharness.reap = real_reap
    assert ptyharness.reap(pid, timeout=5.0) is True


def _leaked_children():
    """Return the registered pids that are still alive, or still unreaped."""
    leaked = []
    for pid in FORKED:
        try:
            done, _status = os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            continue  # Reaped, which is the wanted state.
        if done == 0:
            leaked.append(pid)
    return leaked


def _run_one(name, fn, limit=30):
    """Run one test under a watchdog, so a hang reports FAIL instead of stalling."""

    def bark(_signum, _frame):
        raise TimeoutError(
            f"{name} did not finish inside {limit}s. A hang is the defect under test, "
            "so the watchdog reports it as a failure."
        )

    previous = signal.signal(signal.SIGALRM, bark)
    signal.setitimer(signal.ITIMER_REAL, limit)
    try:
        fn()
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)


if __name__ == "__main__":
    failures = 0
    for name, fn in sorted(globals().items()):
        if not name.startswith("test_"):
            continue
        del FORKED[:]
        try:
            _run_one(name, fn)
            print(f"ok   {name}")
        except BaseException as exc:  # A KeyboardInterrupt must still report and clean up.
            failures += 1
            print(f"FAIL {name}: {type(exc).__name__}: {exc}")
        leaked = _leaked_children()
        if leaked:
            failures += 1
            print(f"FAIL {name}: left {len(leaked)} child process(es) behind: {leaked}")
            for pid in leaked:
                ptyharness.reap(pid, timeout=5.0)
    print(f"{failures} failure(s)")
    raise SystemExit(1 if failures else 0)
