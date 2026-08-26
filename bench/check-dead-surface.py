#!/usr/bin/env python3
"""check-dead-surface.py — a public function no code calls is a wiring defect.

Writing rho's user documentation found five features that exist in the code and reach no
user, because one call was missing. A test suite cannot see any of them: the code under
test works, and nothing uses it. Only a person driving the product finds them.

    McpSchemaCache::save        no caller, so no MCP tool ever reached the model
    motion_enabled              never consulted, so the sweep never drew
    Config::resolve_credential  no caller, so a [credentials] block did nothing

This guard reports that shape. It is not a general dead-code check: the compiler already
finds a private unused item, and it cannot see a public one.

**A test is not a caller.** Every one of the five defects had passing tests over working
code. So `tests/` and a `#[cfg(test)]` module are excluded when counting callers.

**An exemption is a ledger entry, never a bare path.** `bench/allowed-uncalled.txt` holds
one line per exemption:

    crates/rho-core/src/session/key.rs::mint  SPEC-session-store-wiring consumes it.

The reason is mandatory. An entry naming a `SPEC-` slug that does not resolve fails, so a
lane cannot be invented to silence the guard. An entry whose function has gained a caller
fails as unnecessary, so the file cannot fill with lines nobody questions. See
`D-dead-surface-is-a-defect-class` and `D-the-dead-surface-allowlist-names-its-lane`.

**What it cannot see.** A field the renderer never reads, and a live call bound to the
wrong boolean, are both dead surface and neither is an uncalled function. Two of the five
defects had those shapes, and a different guard would be needed for them.

**It is not in the ship gate yet, and that is deliberate.** Its first run reported 35
functions after the entries I could justify precisely. Writing a reason for each needs the
owning spec, and inventing thirty-five reasons in one pass would build the dustbin that
`D-the-dead-surface-allowlist-names-its-lane` forbids. So the triage is a named next job,
and the guard joins the gate when the ledger is honest. A guard nobody runs protects
nothing, and a guard everyone silences protects less.

Run it from the repository root:

    python3 bench/check-dead-surface.py

It prints one line per violation and exits non-zero when it finds any.
"""

from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
CRATES = ROOT / "crates"
SPECS = ROOT / "docs/specs"
ALLOWLIST = ROOT / "bench/allowed-uncalled.txt"

# A `pub fn` or `pub async fn`, capturing the name.
DEFINITION = re.compile(r"^\s*pub (?:async )?fn ([a-z_][a-z0-9_]*)", re.M)
# A name used as a call, a method, or a value passed to something else.
def use_pattern(name: str) -> re.Pattern[str]:
    escaped = re.escape(name)
    # `name(`, `.name(`, `::name`, and `name` as a bare value in a call or a list.
    return re.compile(rf"(?<![a-zA-Z0-9_]){escaped}\s*(?:\(|,|\)|;|\]|\bas\b)|::{escaped}\b")

# A name too generic to attribute, or one the language calls for us.
IGNORED = {
    "new", "default", "from", "into", "fmt", "clone", "drop", "next", "poll", "run",
    "call", "main", "len", "is_empty", "name", "description", "kind", "schema", "execute",
}


def source_files() -> list[pathlib.Path]:
    """Every crate source file, excluding a test tree."""
    files = []
    for path in CRATES.rglob("*.rs"):
        parts = set(path.parts)
        if "target" in parts or "tests" in parts or "benches" in parts:
            continue
        files.append(path)
    return files


def without_test_modules(text: str) -> str:
    """Drop every `#[cfg(test)]` module, so a test never counts as a caller."""
    out = []
    depth = 0
    skipping = False
    lines = text.splitlines()
    for index, line in enumerate(lines):
        if not skipping and "#[cfg(test)]" in line:
            skipping = True
            depth = 0
            continue
        if skipping:
            depth += line.count("{") - line.count("}")
            if depth <= 0 and "}" in line:
                skipping = False
            continue
        out.append(line)
    return "\n".join(out)


def read_allowlist() -> tuple[dict[str, str], list[str]]:
    """The exemptions, by `path::function`, and any problem with the file itself."""
    entries: dict[str, str] = {}
    problems: list[str] = []
    if not ALLOWLIST.exists():
        return entries, problems
    for number, raw in enumerate(ALLOWLIST.read_text().splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split(None, 1)
        key = parts[0]
        reason = parts[1].strip() if len(parts) > 1 else ""
        if not reason:
            problems.append(
                f"bench/allowed-uncalled.txt:{number}: {key} has no reason. "
                f"An exemption states why, or it is a place to hide a defect."
            )
            continue
        for slug in re.findall(r"SPEC-[a-z0-9-]+", reason):
            if not any(spec.name.endswith(f"{slug}.md") for spec in SPECS.glob("*.md")):
                problems.append(
                    f"bench/allowed-uncalled.txt:{number}: the reason names {slug}, "
                    f"which resolves to no spec."
                )
        entries[key] = reason
    return entries, problems


def main() -> int:
    files = source_files()
    bodies = {path: without_test_modules(path.read_text()) for path in files}
    allowed, problems = read_allowlist()
    used_entries: set[str] = set()

    for path, text in bodies.items():
        relative = path.relative_to(ROOT).as_posix()
        for match in DEFINITION.finditer(text):
            name = match.group(1)
            if name in IGNORED or len(name) < 4:
                continue
            pattern = use_pattern(name)
            callers = 0
            for other, other_text in bodies.items():
                hits = len(pattern.findall(other_text))
                if other == path:
                    # Its own definition line is not a call.
                    hits -= len(re.findall(rf"pub (?:async )?fn {re.escape(name)}\b", other_text))
                callers += max(hits, 0)
            if callers > 0:
                key = f"{relative}::{name}"
                if key in allowed:
                    problems.append(
                        f"{key} has a caller now, so its allowlist entry is unnecessary. Remove it."
                    )
                    used_entries.add(key)
                continue
            key = f"{relative}::{name}"
            if key in allowed:
                used_entries.add(key)
                continue
            problems.append(
                f"{key} is public and no code calls it. Wire it, delete it, or add a line "
                f"to bench/allowed-uncalled.txt with the reason."
            )

    for problem in problems:
        print(problem)
    print(f"VIOLATIONS {len(problems)} (checked {len(files)} source files)")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
