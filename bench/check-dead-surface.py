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
# Counting a use by regex alone cannot see the one shape that hurts most: a struct
# field-init shorthand. `Thing { foo }` and `Thing { a, foo }` both end a bare `foo`
# on `}` or `,`, which is exactly the shape a value passed to a call ends on. An earlier
# alternation counted that as a use, so a public `fn foo` that only ever named a struct
# field read as live. rho names many functions after the field they fill, so this was
# not a corner case. It is the same family as the defect this guard exists to catch: a
# check that passes on the shape it was written to report buys false confidence.
#
# The fix is to know the enclosing bracket. A bare value inside `(` or `[` is a use; a
# bare identifier inside a struct-literal `{` is a field, not a use. A regex cannot track
# nesting, so we tokenise once and classify each identifier by its neighbours and by the
# bracket that encloses it. A re-export never reaches here: `strip_use_statements` drops
# every `use`/`pub use`/`pub(crate) use` line before we count, so a name that is only
# re-exported correctly reads as unwired.

_TOKEN = re.compile(r"[A-Za-z_][A-Za-z0-9_]*|[0-9][A-Za-z0-9_.]*|.", re.S)
# A `{` that opens a struct literal is preceded by a type path (`Thing`, `mod::Variant`,
# `Self`). A `{` that opens a block sits after these instead, and a bare identifier under
# such a `{` is a statement or a block value, never a field shorthand.
_TYPE_INTRO = {"->", "impl", "for", "struct", "enum", "trait", "union", "where", "dyn", "as"}


def tokenize(text: str) -> list[str]:
    """Split Rust source into coarse tokens, skipping strings and char literals.

    Brackets inside a string or a char literal must not move the nesting stack, or the
    enclosing-bracket verdict drifts for the rest of the file. Comments are already gone
    by the time this runs. Lifetimes (`'a`) read as a `'` punctuation token followed by an
    identifier, which is harmless.
    """
    tokens: list[str] = []
    i = 0
    n = len(text)
    while i < n:
        c = text[i]
        if c.isspace():
            i += 1
            continue
        if c == '"' or ((c == "r" or c == "b") and i + 1 < n and text[i + 1] in '#"'):
            i = _skip_string(text, i)
            continue
        if c == "'":
            end = _skip_char(text, i)
            if end is not None:
                i = end
                continue
            tokens.append("'")
            i += 1
            continue
        match = _TOKEN.match(text, i)
        assert match is not None
        tokens.append(match.group(0))
        i = match.end()
    return tokens


def _skip_string(text: str, i: int) -> int:
    """Return the index just past a string literal that starts at `i`."""
    n = len(text)
    # A raw string `r".."`, `r#".."#`, or a byte variant carries no escapes.
    if text[i] in "rb":
        j = i + 1
        if j < n and text[j] in "rb":
            j += 1
        hashes = 0
        while j < n and text[j] == "#":
            hashes += 1
            j += 1
        if j < n and text[j] == '"':
            close = '"' + "#" * hashes
            end = text.find(close, j + 1)
            return end + len(close) if end != -1 else n
        # Not a raw string after all, e.g. a bare identifier `b`.
        return i + 1
    j = i + 1
    while j < n:
        if text[j] == "\\":
            j += 2
            continue
        if text[j] == '"':
            return j + 1
        j += 1
    return n


def _skip_char(text: str, i: int) -> int | None:
    """Return the index past a char literal at `i`, or None for a lifetime."""
    n = len(text)
    if i + 1 < n and text[i + 1] == "\\":
        j = i + 2
        if j < n:
            j += 1
        while j < n and text[j] != "'":
            j += 1
        return j + 1 if j < n else n
    if i + 2 < n and text[i + 2] == "'":
        return i + 3
    return None


def _opens_struct_literal(tokens: list[str], brace_index: int) -> bool:
    """Does the `{` at `brace_index` open a struct literal rather than a block?

    A struct literal is a type path immediately before `{`. A block follows a keyword,
    a `)`, `=>`, `->`, or another delimiter. Getting this right is what separates a field
    shorthand from a block value, and rho's UpperCamel types make the path recognisable.
    """
    k = brace_index - 1
    if k < 0 or not _is_ident(tokens[k]):
        return False
    last = tokens[k]
    if last != "Self" and not last[0].isupper():
        return False
    # Walk back over the path (`a::b::C`, with generics) to what introduces it.
    while k - 1 >= 0 and tokens[k - 1] == ":" and k - 2 >= 0 and tokens[k - 2] == ":":
        k -= 2
        if k - 1 >= 0 and _is_ident(tokens[k - 1]):
            k -= 1
    intro = tokens[k - 1] if k - 1 >= 0 else ""
    return intro not in _TYPE_INTRO and intro != ":"


def _is_ident(token: str) -> bool:
    return bool(token) and (token[0].isalpha() or token[0] == "_")


def count_uses(tokens: list[str]) -> dict[str, int]:
    """Count, per identifier, how many times it is used as a function.

    A use is a call `foo(`, a method call `.foo(`, a path `::foo` or `foo::<T>()`, a cast
    `foo as`, a value passed inside `(` or `[` (`Some(foo)`, `map(x, foo)`, `[foo]`), an
    assignment or default value `= foo`, a struct-field or dispatch-table value `field: foo`,
    or a match-arm value `=> foo`. A field-init shorthand under a struct-literal `{` is not
    a use, and a definition `fn foo` is not a use of itself.
    """
    counts: dict[str, int] = {}
    stack: list[str] = []  # enclosing brackets: '(', '[', or '{' / '{s' for a struct literal.
    for index, token in enumerate(tokens):
        if token in "([":
            stack.append(token)
            continue
        if token == "{":
            stack.append("{s" if _opens_struct_literal(tokens, index) else "{")
            continue
        if token in ")]}":
            if stack:
                stack.pop()
            continue
        if not _is_ident(token):
            continue
        prev = tokens[index - 1] if index > 0 else ""
        prev2 = tokens[index - 2] if index > 1 else ""
        nxt = tokens[index + 1] if index + 1 < len(tokens) else ""
        nxt2 = tokens[index + 2] if index + 2 < len(tokens) else ""
        enclosing = stack[-1] if stack else ""
        if prev == "fn":
            continue  # a definition, not a use
        if nxt == "!":
            continue  # a macro name, not this function
        used = (
            nxt == "("                                   # call or method call
            or (nxt == ":" and nxt2 == ":")              # `foo::bar`, `foo::<T>()`
            or (prev == ":" and prev2 == ":")            # `Type::foo`, `Self::foo`
            or nxt == "as"                               # `foo as fn()`
            or prev in "(["                              # first value in a call or list
            or (prev == "," and enclosing in ("(", "["))  # later value in a call or list
            or prev == "="                               # `let f = foo;`, `default = foo`
            or (prev == ":" and prev2 != ":")            # `field: foo`, a dispatch-table value
            or (prev == ">" and prev2 == "=")            # `=> foo`, a match-arm value
        )
        if used:
            counts[token] = counts.get(token, 0) + 1
    return counts

# A name too generic to attribute, or one the language calls for us.
IGNORED = {
    "new", "default", "from", "into", "fmt", "clone", "drop", "next", "poll", "run",
    "call", "main", "len", "is_empty", "name", "description", "kind", "schema", "execute",
}


def source_files() -> list[pathlib.Path]:
    """Every crate source file, excluding a test tree and an examples tree.

    An `examples/` file builds and runs, but it is not production. Counting it as a caller
    kept `sweep_frame` and `MotionInputs::animating` alive, because the only use of each was
    in `crates/rho-tui/examples/frame_bench.rs`. A user never runs an example.
    """
    files = []
    for path in CRATES.rglob("*.rs"):
        parts = set(path.parts)
        if parts & {"target", "tests", "benches", "examples"}:
            continue
        files.append(path)
    return files


def strip_comments(text: str) -> str:
    """Drop every comment, so a doc comment never counts as a caller.

    A doc comment like ``/// See `Foo::bar` `` names a function, it does not call it. With
    the comments left in, three functions read as live on a doc mention alone, and
    `crates/rho-tui/src/paste.rs::units` looked alive on two. So both block comments and
    line comments go before anything is counted. A `//` inside a URL scheme such as
    `https://` is kept, because it is not a comment.
    """
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    out: list[str] = []
    for line in text.splitlines():
        cut = None
        position = 0
        while position < len(line) - 1:
            if line[position] == "/" and line[position + 1] == "/":
                if position == 0 or line[position - 1] != ":":
                    cut = position
                    break
            position += 1
        out.append(line if cut is None else line[:cut])
    return "\n".join(out)


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


def strip_use_statements(text: str) -> str:
    """Drop every `use`, `pub use`, and `pub(crate) use` statement, wrapped or not.

    A re-export moves a name; it never calls it. An earlier version filtered one line at a
    time, so a wrapped `pub use crate::{a,\n    b}` kept its later lines and every name on
    them read as called. A review found the same construct giving opposite verdicts. A
    `pub(crate) use` re-export must go too, or a crate-local re-export of an otherwise dead
    name would read as a caller.
    """
    starts_use = re.compile(r"^(?:pub(?:\([^)]*\))?\s+)?use\s")
    out: list[str] = []
    in_use = False
    for line in text.splitlines():
        stripped = line.strip()
        if not in_use and starts_use.match(stripped):
            in_use = not stripped.endswith(";")
            continue
        if in_use:
            in_use = not stripped.endswith(";")
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
    bodies = {
        path: strip_comments(without_test_modules(path.read_text())) for path in files
    }
    allowed, problems = read_allowlist()
    used_entries: set[str] = set()

    # Count every use once. A `pub use` line moves a name, it does not call it, so it is
    # stripped before counting; a definition `fn foo` is not counted as a use of itself,
    # so no per-file definition-line surgery is needed.
    callers_by_name: dict[str, int] = {}
    for text in bodies.values():
        for name, count in count_uses(tokenize(strip_use_statements(text))).items():
            callers_by_name[name] = callers_by_name.get(name, 0) + count

    for path, text in bodies.items():
        relative = path.relative_to(ROOT).as_posix()
        for match in DEFINITION.finditer(text):
            name = match.group(1)
            if name in IGNORED or len(name) < 4:
                continue
            callers = callers_by_name.get(name, 0)
            key = f"{relative}::{name}"
            if callers > 0:
                if key in allowed:
                    problems.append(
                        f"{key} has a caller now, so its allowlist entry is unnecessary. Remove it."
                    )
                    used_entries.add(key)
                continue
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
