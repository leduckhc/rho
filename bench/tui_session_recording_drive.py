"""Drive the real rho TUI in a pty and prove it records its session to disk.

No in-crate test can reach the event loop's `select!` arms, because the loop owns the
terminal. So this drive is the named guard for the seam call sites and the catalog fix.
See `SPEC-the-interactive-session-records-itself` section 14, and
`docs/verification/interactive-session-recording.md`.

The drive makes five checks:

1. A session file is created, and its path rides the transcript as a notice.
2. The file holds a session header and a model_change with a provider and a model.
3. A clean exit closes the file with a `closed` record.
4. The `/model` picker shows model rows and no `catalog load ended unexpectedly` line.
   This guards the bug the controller found by driving the real binary.
5. The session lock is held for the whole run, so a second rho on the same file refuses.

It reads the screen with pyte and asserts on rendered cells, per the skill notes.
Run it with the venv that has pyte: `/tmp/rho-pty-venv/bin/python`.
"""

import fcntl
import glob
import json
import os
import pty
import re
import select
import shutil
import struct
import subprocess
import sys
import tempfile
import termios
import time

import pyte

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ptyharness import teardown

ROWS, COLS = 50, 220
RHO = os.environ.get(
    "RHO_BINARY", os.path.expanduser("~/Work/Vibe/rho/target/debug/rho")
)
# The fingerprint the openrouter provider uses for its catalog cache. See
# `crates/rho-provider-openrouter/src/lib.rs::catalog_fingerprint`.
CATALOG_FINGERPRINT = "openrouter:https://openrouter.ai"


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


def seed_catalog(home):
    """Seed a fresh catalog cache of 447 models, so `/model` needs no network.

    A fresh cache makes `CatalogCache::list_models` return at once, without the provider
    and without an API key. So the drive reaches the catalog `select!` arm offline, which
    is the arm the fix touched. See `crates/rho-cli/src/catalog_cache.rs`.
    """
    models = [
        {"id": f"vendor/model-{i:03d}", "display_name": None} for i in range(447)
    ]
    cache = {
        "entries": {
            CATALOG_FINGERPRINT: {"fetched_at": int(time.time()), "models": models}
        }
    }
    with open(os.path.join(home, ".rho/model-catalog-cache.json"), "w") as handle:
        json.dump(cache, handle)


def spawn(home, session_file=None):
    os.environ["TERM"] = "xterm-256color"
    os.environ["HOME"] = home
    if session_file is not None:
        os.environ["RHO_SESSION_FILE"] = session_file
    else:
        os.environ.pop("RHO_SESSION_FILE", None)
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


def check(findings, failures, name, ok):
    if ok:
        findings.append(name)
    else:
        failures.append(name)
    return ok


def main():
    findings = []
    failures = []

    # --- Checks 1 to 4: one default run, seeded catalog. ---
    home = tempfile.mkdtemp(prefix="rho-rec-")
    os.makedirs(os.path.join(home, ".rho"), exist_ok=True)
    seed_catalog(home)

    pid, fd = spawn(home)
    screen = pyte.Screen(COLS, ROWS)
    stream = pyte.ByteStream(screen)
    session_path = None
    try:
        pump(fd, stream, 3.0)
        settled = rows(screen)

        # 1. A session file is created, and its path rides the transcript.
        notice = next(
            (row for row in settled if "session " in row and " at " in row), ""
        )
        match = re.search(r"session (\S+) at (\S+\.jsonl)", notice)
        session_path = match.group(2) if match else None
        check(
            findings,
            failures,
            "1a. transcript shows a `session <id> at <path>` notice",
            match is not None,
        )
        check(
            findings,
            failures,
            "1b. the path from the notice exists on disk",
            session_path is not None and os.path.exists(session_path),
        )

        # 4. The `/model` picker shows model rows and no catalog error.
        os.write(fd, b"/model\r")
        pump(fd, stream, 2.0)
        picker = rows(screen)
        has_model_rows = any("model-0" in row for row in picker)
        no_catalog_error = not any(
            "catalog load ended unexpectedly" in row for row in picker
        )
        check(
            findings,
            failures,
            "4a. the picker shows model rows from the catalog",
            has_model_rows,
        )
        check(
            findings,
            failures,
            "4b. no `catalog load ended unexpectedly` line drew",
            no_catalog_error,
        )
        os.write(fd, b"\x1b")  # esc closes the picker
        pump(fd, stream, 0.5)

        # 3. A clean exit closes the file. Quit with ctrl-c twice.
        os.write(fd, b"\x03")
        pump(fd, stream, 0.4)
        os.write(fd, b"\x03")
        pump(fd, stream, 1.5)
    finally:
        if not teardown(pid, fd, timeout=5.0):
            print(f"warning: rho child {pid} outlived its teardown", file=sys.stderr)

    # 2 and 3: read the file back.
    body = ""
    if session_path and os.path.exists(session_path):
        with open(session_path) as handle:
            body = handle.read()
    records = [json.loads(line) for line in body.splitlines() if line.strip()]
    types = [record.get("type") for record in records]

    check(
        findings,
        failures,
        "2a. the file holds a session header record",
        "session" in types,
    )
    model_change = next(
        (record for record in records if record.get("type") == "model_change"), None
    )
    check(
        findings,
        failures,
        "2b. a model_change record carries a provider and a model",
        model_change is not None
        and bool(model_change.get("provider"))
        and bool(model_change.get("model")),
    )
    check(
        findings,
        failures,
        "3. the file ends with a `closed` record",
        types[-1:] == ["closed"],
    )

    shutil.rmtree(home, ignore_errors=True)

    # --- Check 5: the lock is held for the whole run. ---
    home2 = tempfile.mkdtemp(prefix="rho-lock-")
    os.makedirs(os.path.join(home2, ".rho"), exist_ok=True)
    seed_catalog(home2)
    named = os.path.join(home2, ".rho", "named-session.jsonl")

    pid2, fd2 = spawn(home2, session_file=named)
    screen2 = pyte.Screen(COLS, ROWS)
    stream2 = pyte.ByteStream(screen2)
    try:
        pump(fd2, stream2, 3.0)
        first_up = any(
            "named-session" in row or " at " in row for row in rows(screen2)
        )
        # A second rho on the same named file must refuse before it draws anything.
        env = dict(os.environ)
        env["HOME"] = home2
        env["RHO_SESSION_FILE"] = named
        env["TERM"] = "dumb"
        second = subprocess.run(
            [RHO, "--no-skills", "--no-agents", "--provider", "openrouter",
             "--model", "seed-model"],
            capture_output=True,
            text=True,
            timeout=30,
            env=env,
            stdin=subprocess.DEVNULL,
        )
        refused = second.returncode != 0 and (
            "another process" in second.stderr or "is open in another" in second.stderr
        )
        check(
            findings,
            failures,
            "5a. the first rho started and holds the named file",
            first_up,
        )
        check(
            findings,
            failures,
            "5b. a second rho on the same file refuses with the busy error",
            refused,
        )
        if not refused:
            failures.append(f"    second rho stderr: {second.stderr!r}")
    finally:
        if not teardown(pid2, fd2, timeout=5.0):
            print(f"warning: rho child {pid2} outlived its teardown", file=sys.stderr)
        shutil.rmtree(home2, ignore_errors=True)

    print("--- FINDINGS")
    for finding in findings:
        print("   PASS · " + finding)
    if failures:
        print("--- FAILURES")
        for failure in failures:
            print("   FAIL · " + failure)
        print(f"--- FAILED · {len(failures)} of {len(findings) + len(failures)} checks")
        sys.exit(1)
    print(f"--- OK · {len(findings)} checks passed")


if __name__ == "__main__":
    main()
