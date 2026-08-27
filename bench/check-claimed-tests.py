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

import pathlib
import re
import subprocess
import sys


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], capture_output=True, text=True, check=False
    ).stdout


class LedgerError(Exception):
    """A line in `deleted-tests.txt` does not parse. The guard must fail, not guess."""


# A test name in this project: a lowercase identifier. It must be the whole first token, so a
# stray reason word such as `false` or `project` cannot pass as a name.
TEST_NAME = re.compile(r"^[a-z][a-z0-9_]*$")
# A git short or full hash. A commit is the proof of a deliberate removal, so it must be real
# hex, never the word `HEAD`, which moves and now points at a commit that deleted nothing.
COMMIT_HASH = re.compile(r"^[0-9a-f]{7,40}$")


def default_range() -> str:
    if git("rev-parse", "--verify", "-q", "origin/main").strip():
        return "origin/main..HEAD"
    return "HEAD~20..HEAD"


def deleted_names() -> set[str]:
    """The names a commit removed on purpose, from `bench/deleted-tests.txt`.

    A commit that deletes a test names it, and the words alone do not say whether the name was
    added or removed. The guard found that on its first run over the whole branch, against a test
    the owner's ruling had reversed. So a removal is recorded rather than guessed at.

    Each entry is one physical line: `<test_name> <commit-hash> <reason>`. The parser is strict.
    An earlier version took `line.split()[0]` from every non-comment line, so a wrapped reason
    turned each of its words into a false exemption. A continuation line that began with a real
    test name would then exempt that test from the guard forever, which is the very loss this
    file exists to stop. So a line that does not parse fails loudly.
    """
    path = pathlib.Path(__file__).with_name("deleted-tests.txt")
    if not path.exists():
        return set()
    names = set()
    for number, raw in enumerate(path.read_text().splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split()
        # First token is a test name, second is a real hex commit, and a reason follows.
        if len(parts) < 3 or not TEST_NAME.match(parts[0]) or not COMMIT_HASH.match(parts[1]):
            raise LedgerError(
                f"bench/deleted-tests.txt:{number}: cannot parse this line. Each entry is one "
                f"physical line: <test_name> <commit-hash> <reason>. A wrapped reason or a "
                f"missing hash silently exempts a real test. Offending line: {raw!r}"
            )
        names.add(parts[0])
    return names


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

    try:
        allowed = deleted_names()
    except LedgerError as error:
        print(error)
        print("VIOLATIONS 1 (bench/deleted-tests.txt does not parse)")
        return 1
    missing = []
    for name in claimed_names(body):
        if name in allowed:
            continue
        # The name must end where the declaration does. A plain substring search passed
        # `fn <name>_renamed`, which is exactly the loss this guard exists to catch, so the
        # first version of it was satisfied by a rename.
        found = git(
            "grep", "-lE", rf"fn {re.escape(name)} *[(<]", "--", "crates"
        ).strip()
        if not found:
            missing.append(name)

    for name in missing:
        print(f"a commit message names `{name}`, and no test of that name exists")
    if missing:
        print(
            "add a test, or record a deliberate removal in bench/deleted-tests.txt "
            "with the commit and the reason"
        )
    print(f"VIOLATIONS {len(missing)} (range {span})")
    return 1 if missing else 0


if __name__ == "__main__":
    raise SystemExit(main())
