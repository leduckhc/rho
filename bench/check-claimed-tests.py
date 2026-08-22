#!/usr/bin/env python3
"""check-claimed-tests.py — every test a commit message names must exist in the tree.

A green suite cannot see a missing test. That is not a hypothesis: on this branch a review
agent whose worktree isolation failed restored an older copy of a file over three tests, the
suite stayed green, and a commit message then claimed one of them by name.

So this guard reads the commit messages of a range, collects every name that looks like a test
in this project's style, and asserts that `fn <name>` exists somewhere under `crates/`.

Usage:
    python3 bench/check-claimed-tests.py [<git range>]

The default range is `origin/main..HEAD` when that resolves, and `HEAD~20..HEAD` otherwise, so
the guard is useful both in CI and on a branch with no upstream.

It reports one line per missing name and exits non-zero. It prints VIOLATIONS 0 when clean.
"""

import re
import subprocess
import sys


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], capture_output=True, text=True, check=False
    ).stdout


def default_range() -> str:
    if git("rev-parse", "--verify", "-q", "origin/main").strip():
        return "origin/main..HEAD"
    return "HEAD~20..HEAD"


def claimed_names(text: str) -> list[str]:
    # A test name in this project reads like a sentence: lowercase words joined by underscores.
    # Three underscores keeps ordinary identifiers such as `max_tokens` out of the set.
    candidates = re.findall(r"\b([a-z][a-z0-9_]{14,})\b", text)
    return sorted({name for name in candidates if name.count("_") >= 3})


def main() -> int:
    span = sys.argv[1] if len(sys.argv) > 1 else default_range()
    body = git("log", "--format=%B", span)
    if not body.strip():
        print(f"VIOLATIONS 0 (no commits in {span})")
        return 0

    missing = []
    for name in claimed_names(body):
        found = git("grep", "-l", f"fn {name}", "--", "crates").strip()
        if not found:
            missing.append(name)

    for name in missing:
        print(f"a commit message names `{name}`, and no test of that name exists")
    print(f"VIOLATIONS {len(missing)} (range {span})")
    return 1 if missing else 0


if __name__ == "__main__":
    raise SystemExit(main())
