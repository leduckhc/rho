"""Bounded teardown for a pseudo-terminal child process.

A benchmark harness must never outlive the thing it measures. On 2026-08-18 a `rho`
child reached state `?Es`, which is the kernel ending a process and not finishing. The
harness then called `os.waitpid(pid, 0)`, which blocks with no bound, so the benchmark
hung for 30 minutes and held a whole agent session open. A `sample` of the harness showed
one frame at the top of the stack: `__wait4`.

So teardown here has a deadline. A child that will not die makes the harness report the
fact and continue. It never makes the harness wait forever.

The deadline alone is not the whole fix, because the order is what creates the stuck
child. A measurement stops reading the master fd as soon as it has its sample, so the pty
buffer fills, and the child then blocks inside a write to its own terminal. `SIGKILL`
cannot finish while that write sits in the kernel. Closing the master first makes the
write fail with `EIO`, which frees the child at once.

Three shapes, and each number names the code that produced it. Twelve runs of
`bench/tui_first_frame.py`:

    shipped: no deadline, kill then close   hung on the first wedged run, 30 minutes
    deadline added, still kill then close   60.4 s total, 12 deadline warnings
    deadline and close, then kill            0.42 s total, no warning

The middle line is not the shipped defect. It is the half fix, measured on the way, and it
shows that a deadline alone only converts a hang into a slow benchmark.

The old shape also leaked the master fd on every run, because it never closed it. Twelve
runs leaked 12 file descriptors. `teardown` closes it once per run, so the count stays
flat.

Use `teardown` and not `reap` alone, because `teardown` holds that order for you.
"""
import os
import signal
import time


def reap(pid, timeout=5.0, poll=0.02, kill=True):
    """End the child `pid`, and wait for it for at most `timeout` seconds.

    Probe first, and send `SIGKILL` only while the child is still ours. Then poll with
    `os.WNOHANG` until the deadline. Pass `kill=False` to hold the signal back, which the
    tests use to make a child outlive its deadline.

    Return `True` when the child is reaped, or when it was reaped already. Return `False`
    when the deadline passes and the child is still there. This function never blocks
    longer than `timeout`, and `timeout` must be greater than zero.
    """
    if timeout <= 0:
        # A child killed microseconds ago is not reaped yet, so a deadline of zero reports
        # a false failure for a healthy child. Refuse the value instead of lying.
        raise ValueError(f"timeout must be greater than zero, got {timeout!r}")
    deadline = time.perf_counter() + timeout
    while True:
        try:
            done, _status = os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            # Another party reaped the child, so there is nothing left to wait for. No
            # signal goes out on this path, because the operating system reuses a pid and
            # the number may now belong to an unrelated process.
            return True
        if done == pid:
            return True
        if kill:
            # The probe above proves the child is still ours, so this signal is safe.
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            kill = False  # One signal is enough. SIGKILL cannot be caught or ignored.
        if time.perf_counter() >= deadline:
            return False
        time.sleep(poll)


def close_pty(fd):
    """Close the pseudo-terminal master `fd`. Ignore a file descriptor already closed."""
    try:
        os.close(fd)
    except OSError:
        pass


def teardown(pid, fd, timeout=5.0):
    """End the child `pid` and release its pseudo-terminal `fd`, in the safe order.

    Close the master fd first, and kill second. Read the module docstring for the
    measurement that shows why the other order stalls.

    Return `True` when the child is reaped inside `timeout`.
    """
    close_pty(fd)
    return reap(pid, timeout=timeout)
