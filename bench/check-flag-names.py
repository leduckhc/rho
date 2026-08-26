#!/usr/bin/env python3
"""check-flag-names.py — no message may tell a user to pass a flag that does not exist.

`rho-core` refused a resume with "pass --allow-widen to allow it", and no such flag was
ever defined. This project already paid for that shape once: every subagent limit refusal
named a flag to raise, and the flags did not exist, which a live sweep found.

The guard reads every `pass --<flag>` phrase in `crates/*/src` and checks the name against
the flags clap really defines in `crates/rho-cli/src/cli.rs`.

It anchors on the phrase, never on a bare `--token`. A cargo flag such as `--features`, and
a bwrap argument such as `--ro-bind`, both appear in messages and neither is a rho flag, so
a guard that scanned every token would cry wolf and be deleted.

Run it from the repository root:

    python3 bench/check-flag-names.py
"""

from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
CLI = ROOT / "crates/rho-cli/src/cli.rs"
CRATES = ROOT / "crates"

# `pass --flag`, the phrase a message uses to tell a user what to do.
TOLD_TO_PASS = re.compile(r"pass\s+`?--([a-z][a-z0-9-]*)")
# A clap long flag, from its attribute or its field name.
FIELD = re.compile(r"^\s*(?:pub )?([a-z_][a-z0-9_]*)\s*:", re.M)


def defined_flags() -> set[str]:
    """Every long flag the command line accepts."""
    text = CLI.read_text()
    flags = {name.replace("_", "-") for name in FIELD.findall(text)}
    flags.update(re.findall(r'long\s*=\s*"([a-z][a-z0-9-]*)"', text))
    return flags


def main() -> int:
    flags = defined_flags()
    problems: list[str] = []
    for path in CRATES.rglob("src/**/*.rs"):
        if "target" in path.parts:
            continue
        for number, line in enumerate(path.read_text().splitlines(), 1):
            for named in TOLD_TO_PASS.findall(line):
                if named not in flags:
                    problems.append(
                        f"{path.relative_to(ROOT)}:{number}: a message says to pass "
                        f"--{named}, and the command line defines no such flag."
                    )
    for problem in problems:
        print(problem)
    print(f"VIOLATIONS {len(problems)} (checked against {len(flags)} defined flags)")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
