#!/usr/bin/env python3
"""check-flag-names.py — no message may tell a user to pass a flag that does not exist.

`rho-core` refused a resume with "pass --allow-widen to allow it", and no such flag was
ever defined. This project already paid for that shape once: every subagent limit refusal
named a flag to raise, and the flags did not exist, which a live sweep found.

The guard reads every `<verb> --<flag>` phrase in `crates/*/src` and checks the name against
the flags clap really defines in `crates/rho-cli/src/cli.rs`.

It anchors on the phrase, never on a bare `--token`. A cargo flag such as `--features`, and
a bwrap argument such as `--ro-bind`, both appear in messages and neither is a rho flag, so
a guard that scanned every token would cry wolf and be deleted.

The verb list is not decorative. A message tells a user to `pass`, `raise`, `use`, `set`, or
`remove` a flag, and all five verbs occur before a real flag in this tree. The match is case
insensitive, because six messages begin `Pass --...` with a capital, and a case-sensitive
anchor read none of them.

The defined-flag set is built from the `#[arg(...)]` attributes on the command structs alone.
An earlier version read every `field:` line, so it held 72 names including `std`, `anyhow`,
and `approval`, and a message naming a flag that does not exist still passed.

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

# `<verb> --flag`, the phrase a message uses to tell a user what to do. Case insensitive, so
# `Pass --trust-project` is read the same as `pass --trust-project`.
TOLD_TO_PASS = re.compile(
    r"\b(?:pass|raise|use|set|remove)\s+`?--([a-z][a-z0-9-]*)", re.IGNORECASE
)
# The start of a `#[arg(...)]` attribute, which marks a clap flag on a command struct.
ARG_ATTR = re.compile(r"^\s*#\[arg\(")
# `long = "name"`, when the attribute renames the flag away from its field name.
LONG_VALUE = re.compile(r'long\s*=\s*"([a-z][a-z0-9-]*)"')
# A bare `long`, which keeps the field name as the flag name.
HAS_LONG = re.compile(r"\blong\b")
# The field declaration that follows the attribute, giving the flag its default name.
FIELD_NAME = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?([a-z_][a-z0-9_]*)\s*:")

# Flags clap adds for every command. A message may name either, so they are always valid.
CLAP_BUILTINS = {"help", "version"}


def defined_flags() -> set[str]:
    """Every long flag the command line accepts, from the `#[arg(...)]` attributes alone.

    A flag is a field that carries `#[arg(long)]` or `#[arg(long = "name")]`. Reading the
    attributes, not every `field:` line, keeps struct fields such as `notices` and `state`
    and type-annotated `let` bindings out of the set. See the module docstring.
    """
    lines = CLI.read_text().splitlines()
    flags: set[str] = set(CLAP_BUILTINS)
    index = 0
    while index < len(lines):
        if not ARG_ATTR.match(lines[index]):
            index += 1
            continue
        # Collect the whole attribute, which may wrap over several physical lines.
        attribute = lines[index].strip()
        last = index
        while not attribute.rstrip().endswith(")]") and last + 1 < len(lines):
            last += 1
            attribute += " " + lines[last].strip()
        if HAS_LONG.search(attribute):
            renamed = LONG_VALUE.search(attribute)
            if renamed:
                flags.add(renamed.group(1))
            else:
                # A bare `long` keeps the field name. Find the field the attribute decorates,
                # skipping any further attributes and doc comments between them.
                cursor = last + 1
                while cursor < len(lines):
                    text = lines[cursor].strip()
                    if not text or text.startswith("#[") or text.startswith("//"):
                        cursor += 1
                        continue
                    field = FIELD_NAME.match(lines[cursor])
                    if field:
                        flags.add(field.group(1).replace("_", "-"))
                    break
        index = last + 1
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
