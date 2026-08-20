#!/usr/bin/env python3
"""Check that every artifact id resolves, and that no numeric id comes back.

rho names an artifact by a timestamp and a slug, not by a counter. A counter clashes
whenever two worktrees allocate the next number at the same time, and this project hit
that on every parallel stage. See `D-slug-ids`.

The rules this guard enforces:

1. A spec, an ADR, and a decision file is named `<yyyymmdd-hhmmss>-<KIND>-<slug>.md`.
2. No two files share a slug, and no two feature rows share a slug.
3. Every `SPEC-<slug>`, `ADR-<slug>`, `D-<slug>`, and `F-<slug>` reference resolves to a
   real file or a real feature row.
4. No numeric id, such as `SPEC-14` or `D-017`, appears anywhere except in the registry
   `docs/ids.md`, which exists to translate the commit history.

Run it from the repository root:

    python3 bench/check-ids.py

It prints one line per violation and exits non-zero when it finds any.
"""

from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "docs/ids.md"

# `.github/` belongs to another agent in this tree, so this guard never reads it.
# See `D-shared-working-tree`.
SKIP_DIRS = {".git", "target", "node_modules", ".github", "dist", ".astro"}
SKIP_FILES = {REGISTRY, ROOT / "tools/migrate-ids.py", ROOT / "bench/check-ids.py"}
TEXT_SUFFIXES = {".md", ".rs", ".toml", ".yaml", ".yml", ".css", ".astro", ".py", ".sh", ".html"}

FILE_NAME = re.compile(r"^(\d{8}-\d{6})-(SPEC|ADR|D)-([a-z0-9][a-z0-9-]*)\.md$")
REFERENCE = re.compile(r"(?<![0-9A-Za-z_-])(SPEC|ADR|D|F)-([a-z][a-z0-9-]*)(?![0-9A-Za-z])")
NUMERIC = re.compile(r"(?<![0-9A-Za-z_-])(SPEC|ADR|D|F)-(\d+)(?![0-9A-Za-z])")


def text_files() -> list[pathlib.Path]:
    out = []
    for path in ROOT.rglob("*"):
        if not path.is_file() or path.suffix not in TEXT_SUFFIXES:
            continue
        if any(part in SKIP_DIRS for part in path.parts):
            continue
        if path in SKIP_FILES:
            continue
        out.append(path)
    return sorted(out)


def collect_defined() -> tuple[dict[str, set[str]], list[str]]:
    """Return the defined slugs per kind, and any violation found while collecting."""
    problems: list[str] = []
    defined: dict[str, set[str]] = {"SPEC": set(), "ADR": set(), "D": set(), "F": set()}

    groups = {
        "SPEC": ROOT / "docs/specs",
        "ADR": ROOT / "docs/adr",
        "D": ROOT / ".rho-work/decisions",
    }
    for kind, directory in groups.items():
        if not directory.is_dir():
            problems.append(f"{directory}: the directory is missing")
            continue
        for path in sorted(directory.glob("*.md")):
            if path.name == "README.md":
                continue
            match = FILE_NAME.match(path.name)
            if not match:
                problems.append(
                    f"{path.relative_to(ROOT)}: the name must be <yyyymmdd-hhmmss>-<KIND>-<slug>.md"
                )
                continue
            found_kind, slug = match.group(2), match.group(3)
            if found_kind != kind:
                problems.append(f"{path.relative_to(ROOT)}: a {kind} file names itself {found_kind}")
            if slug in defined[kind]:
                problems.append(f"{path.relative_to(ROOT)}: the slug {kind}-{slug} is already taken")
            defined[kind].add(slug)

    features = ROOT / "docs/features.md"
    if features.is_file():
        for row in re.findall(r"^\| F-([a-z][a-z0-9-]*) \|", features.read_text(), re.M):
            if row in defined["F"]:
                problems.append(f"docs/features.md: the slug F-{row} appears in two rows")
            defined["F"].add(row)
    else:
        problems.append("docs/features.md: the file is missing")

    return defined, problems


def main() -> int:
    defined, problems = collect_defined()

    for path in text_files():
        try:
            text = path.read_text()
        except UnicodeDecodeError:
            continue
        relative = path.relative_to(ROOT)
        for kind, number in NUMERIC.findall(text):
            problems.append(
                f"{relative}: {kind}-{number} is a numeric id. Use the slug form, and see docs/ids.md"
            )
        for kind, slug in REFERENCE.findall(text):
            if slug not in defined[kind]:
                problems.append(f"{relative}: {kind}-{slug} resolves to no file and no row")

    counts = ", ".join(f"{kind}={len(slugs)}" for kind, slugs in sorted(defined.items()))
    if problems:
        for problem in sorted(set(problems)):
            print(problem)
        print(f"VIOLATIONS {len(set(problems))} (defined: {counts})")
        return 1
    print(f"VIOLATIONS 0 (defined: {counts})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
