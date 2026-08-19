#!/usr/bin/env python3
"""Check that every test a spec names really exists.

A spec's `## Test cases` section is a promise. `bench/check-ids.py` cannot check it,
because a test name is not an id, so a spec could name a test that was never written
and every gate would still pass. That happened: `SPEC-agent-tasks` named fifteen
tests that did not exist, some because the implementation used a different name and
nobody reconciled the two. AGENTS.md step 13 requires the reconciliation, and this
guard is what makes it checkable.

The rule: inside a spec's test-case section, a backticked name that looks like a Rust
test name must exist as `fn <name>` somewhere under `crates/`.

**Only a delivered spec is enforced.** A draft spec names the tests its feature will
have, and that is the point of writing a spec first. So the guard reads the `Status:`
line: a spec that says `draft` is exempt, and a spec that says `delivered` must keep
every promise. Marking a single line `planned` exempts that line in any spec.

Run it from the repository root:

    python3 bench/check-spec-tests.py

It prints one line per violation and exits non-zero when it finds any.
"""

from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
SPECS = ROOT / "docs/specs"
CRATES = ROOT / "crates"

# A spec that is still a draft names tests it has not written yet, on purpose.
STATUS = re.compile(r"^Status:\s*(.+)$", re.IGNORECASE | re.MULTILINE)

# A heading that starts a section listing test names.
TEST_HEADING = re.compile(r"^#+\s.*test cases", re.IGNORECASE)
ANY_HEADING = re.compile(r"^##\s")
# A backticked lower_snake_case name with at least two underscores reads as a test.
CANDIDATE = re.compile(r"`([a-z][a-z0-9]*(?:_[a-z0-9]+){2,})`")


def test_names_in_source() -> set[str]:
    """Every `fn name` defined under `crates/`."""
    names: set[str] = set()
    for path in CRATES.rglob("*.rs"):
        if "target" in path.parts:
            continue
        for match in re.finditer(r"\bfn\s+([a-z_][a-z0-9_]*)", path.read_text()):
            names.add(match.group(1))
    return names


def named_in_test_sections(spec: pathlib.Path) -> list[tuple[int, str, str]]:
    """Return (line number, name, line) for each candidate in a test-case section."""
    out: list[tuple[int, str, str]] = []
    inside = False
    in_code = False
    for number, line in enumerate(spec.read_text().splitlines(), 1):
        if line.strip().startswith("```"):
            in_code = not in_code
            continue
        if in_code:
            continue
        if TEST_HEADING.match(line):
            inside = True
            continue
        # A new top-level section ends the test list.
        if inside and ANY_HEADING.match(line) and not TEST_HEADING.match(line):
            inside = False
        if not inside:
            continue
        for match in CANDIDATE.finditer(line):
            out.append((number, match.group(1), line))
    return out


def is_delivered(spec: pathlib.Path) -> bool:
    """True when a spec claims its feature shipped, so its promises are due."""
    match = STATUS.search(spec.read_text())
    if match is None:
        # No status line, so treat it as due. A spec should say where it stands.
        return True
    status = match.group(1).lower()
    return "draft" not in status and "planned" not in status


def main() -> int:
    defined = test_names_in_source()
    problems: list[str] = []
    checked = 0
    exempt = 0

    for spec in sorted(SPECS.glob("*.md")):
        if not is_delivered(spec):
            exempt += 1
            continue
        for number, name, line in named_in_test_sections(spec):
            checked += 1
            if name in defined:
                continue
            if "planned" in line.lower():
                continue
            problems.append(
                f"{spec.relative_to(ROOT)}:{number}: the spec names `{name}` "
                f"and no test defines it. Write the test, correct the name, or "
                f"mark the line planned."
            )

    for problem in problems:
        print(problem)
    print(
        f"VIOLATIONS {len(problems)} "
        f"(checked {checked} test names in delivered specs, {exempt} draft spec(s) exempt)"
    )
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
