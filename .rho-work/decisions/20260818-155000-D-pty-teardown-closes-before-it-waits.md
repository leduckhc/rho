# D-pty-teardown-closes-before-it-waits — A pty harness closes the master, then kills, then waits with a deadline

**Question (controller, session triage):** `bench/tui_first_frame.py` held an agent session
open for 30 minutes. What is the rule that stops this class of hang?

**Decision:** A benchmark harness ends a pty run in exactly this order. Close the master
file descriptor. Probe with `os.waitpid(pid, os.WNOHANG)` to prove the child is still ours.
Send `SIGKILL`. Then keep polling until a deadline of five seconds.
`bench/ptyharness.teardown` holds that order, and `bench/test_ptyharness.py` pins it. A
child that outlives the deadline makes the harness print a warning and continue. A
`timeout` of zero or less is refused, because no deadline that short can be honoured.

**Reason:** Two separate faults combined, and each one alone was survivable.

The wait had no bound. `os.waitpid(pid, 0)` blocks forever. A `sample` of the hung harness
showed one frame at the top of the stack, `__wait4`, for 885 of 885 samples.

The order was wrong, and that is what created the unkillable child. A run stops reading the
master as soon as it has its sample, so the pty buffer fills, and the child blocks inside a
write to its own terminal. `SIGKILL` cannot finish while that write sits in the kernel, so
the child sits in state `?Es`. Three runs for each order, against the release binary:

    close master, then SIGKILL             -> 13ms, 13ms, 13ms
    SIGKILL, master open, no drain         -> 11ms, timeout(>8s), timeout(>8s)
    SIGKILL, master open, keep draining    -> 0ms, 0ms, 0ms

The middle row is not deterministic. One run in three survived, which is why the harness
worked for two sprints before it hung.

Each number belongs to a named code shape, and a review caught an earlier draft that mixed
them. The shipped code had no deadline, so it hung on the first wedged run. Adding a
deadline but keeping the order gave 60.4 seconds over twelve runs, with twelve warnings.
Adding both gives 0.42 seconds and no warning. The old shape also never closed the master,
so twelve runs leaked twelve file descriptors, which is now measured.

A third lesson came from the test, not the code. The first test for the order asserted
elapsed time. It passed against the broken order, because a Python stand-in child reaches
only 605 milliseconds and never wedges. So the test now asserts the order itself: it
watches whether `os.fstat` on the master fails at the moment the wait begins.

A fourth lesson came from the test runner. A hang is the defect under test, and a plain
runner can only stall against it. So the runner arms `SIGALRM` around every test, and it
checks every forked pid afterwards. A hang now prints `FAIL`, and a leaked child does too.

**Rules out:** `os.waitpid(pid, 0)` in any harness in `bench/`, which a CI guard now greps
for. A teardown that kills before it closes the master. A signal a child can catch, such as
`SIGTERM`. A kill sent to a pid the harness no longer owns. A timing threshold as the guard
for this defect, because the wedge needs a real multi-threaded writer to appear. A test
runner that answers a hang with silence. A harness that can outlive the process it
measures.
