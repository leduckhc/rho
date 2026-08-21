#!/usr/bin/env python3
"""Check the agentic workflow template, and every filled track.

`agentic-workflow.yaml` is the blueprint the controller fills per unit of work. Nothing
parsed the three sprint files it replaced, so a dangling reference, a missing role, and a
deleted invariant were all undetectable. Each check below comes from a defect that reached
`main`, or from a defect this guard found in the template on its first run.

The rules this guard enforces:

1.  Every gate in a `gate_sets` entry, and in a stage's `gates`, names a real gate.
2.  `gate_sets.full` matches the `## Gate` block in `AGENTS.md`, command for command.
3.  Every stage kind belongs to exactly one phase, and every phase names a real kind.
4.  Every role in a stage kind is a real role. Two stages once dispatched a role that
    their own file did not hold.
5.  Every `inputs` and `inputs_when` reference resolves to a real slot that a real stage
    produces, in the lane that runs it. A `<stage>.review.<slot>` reference resolves
    against `spawn.review_handover.produces`, and a `fixed_dod.<lane>` reference resolves
    against `fixed_dod`.
6.  A lane never skips a stage that a later stage in that lane needs an input from.
7.  Every lane in `lanes` skips only real stage kinds.
8.  Every slot has a type. No slot carries the word `required`, because
    `slot_rules` states that every slot is required.
9.  No stage kind carries a `lanes` list. `lanes.<lane>.skips` is the only skip mechanism.
10. Every `dod_authored_by`, and every per-lane override of it, names a real role or the
    template, and never the role whose work the DoD scores.
11. A filled track under `.rho-work/tracks/` holds no stage with more than four items,
    uses a slug for every stage id, and never adds a stage kind of its own.

Run it from the repository root:

    python3 bench/check-agentic-workflow.py

It prints one line per violation and exits non-zero when it finds any.
"""

from __future__ import annotations

import pathlib
import re
import sys

try:
    import yaml
except ModuleNotFoundError:  # pragma: no cover - the runner must hold PyYAML
    print("VIOLATION: PyYAML is not installed. Run `python3 -m pip install pyyaml`.")
    sys.exit(2)

ROOT = pathlib.Path(__file__).resolve().parents[1]
TEMPLATE = ROOT / "agentic-workflow.yaml"
AGENTS = ROOT / "AGENTS.md"
TRACKS = ROOT / ".rho-work/tracks"

SLUG = re.compile(r"^[a-z0-9][a-z0-9-]*$")
MAX_ITEMS = 4
COUNTED_ITEM_KEYS = ("dod", "artifacts", "slices", "gate_extra")


def load(path: pathlib.Path) -> dict:
    with path.open(encoding="utf-8") as handle:
        return yaml.safe_load(handle)


def gate_block(path: pathlib.Path) -> list[str]:
    """Return the commands in the `## Gate` fenced block of AGENTS.md."""
    text = path.read_text(encoding="utf-8")
    match = re.search(r"^## Gate\b.*?```sh\n(.*?)```", text, re.S | re.M)
    if not match:
        return []
    return [line.strip() for line in match.group(1).splitlines() if line.strip()]


def check_gates(doc: dict, problems: list[str]) -> None:
    gates = doc.get("gates") or {}
    for name, members in (doc.get("gate_sets") or {}).items():
        for gate in members:
            if gate not in gates:
                problems.append(f"gate_sets.{name} names an unknown gate `{gate}`")

    declared = [gates[g] for g in (doc.get("gate_sets") or {}).get("full", []) if g in gates]
    listed = gate_block(AGENTS)
    if not listed:
        problems.append("AGENTS.md has no `## Gate` fenced block, so the gate cannot be compared")
    elif sorted(declared) != sorted(listed):
        for missing in sorted(set(listed) - set(declared)):
            problems.append(f"gate_sets.full is missing the AGENTS.md gate command `{missing}`")
        for extra in sorted(set(declared) - set(listed)):
            problems.append(f"gate_sets.full holds `{extra}`, which AGENTS.md `## Gate` does not")


def check_shape(doc: dict, kinds: dict, problems: list[str]) -> dict[str, set[str]]:
    """Check the phases, the roles, and the slots. Return the slots each kind produces."""
    roles = set(doc.get("roles") or {})
    placed: dict[str, str] = {}
    for phase in doc.get("phases") or []:
        listed = phase.get("stages")
        names = listed if isinstance(listed, list) else []
        names = list(names) + list(phase.get("allowed_kinds") or [])
        for name in names:
            if name not in kinds:
                problems.append(f"phase {phase['id']} names an unknown stage kind `{name}`")
            elif name in placed:
                problems.append(f"stage kind `{name}` is in phase {placed[name]} and phase {phase['id']}")
            else:
                placed[name] = phase["id"]
    for name in kinds:
        if name not in placed:
            problems.append(f"stage kind `{name}` belongs to no phase")

    produces: dict[str, set[str]] = {}
    for name, kind in kinds.items():
        if "lanes" in kind:
            problems.append(f"stage kind `{name}` carries a `lanes` list. Only lanes.<lane>.skips may skip")
        for role in as_list(kind.get("role")):
            if role not in roles:
                problems.append(f"stage kind `{name}` names an unknown role `{role}`")
        for role in as_list(kind.get("reviewed_by")):
            if role not in roles:
                problems.append(f"stage kind `{name}` names an unknown reviewer `{role}`")
        for gate in as_list(kind.get("gates")):
            if gate not in (doc.get("gate_sets") or {}):
                problems.append(f"stage kind `{name}` names an unknown gate set `{gate}`")
        author = kind.get("dod_authored_by")
        if author not in roles and author != "template":
            problems.append(f"stage kind `{name}` has no valid `dod_authored_by`, so its owner may write it")
        if author in as_list(kind.get("role")):
            problems.append(f"stage kind `{name}` lets its own role author the DoD it is scored against")
        for lane, per_lane in (kind.get("dod_authored_by_when") or {}).items():
            if per_lane not in roles and per_lane != "template":
                problems.append(f"{name}.dod_authored_by_when.{lane} names no role and is not `template`")
            if per_lane in as_list(kind.get("role")):
                problems.append(
                    f"{name}.dod_authored_by_when.{lane} lets its own role author its own DoD"
                )

        slots = kind.get("produces") or {}
        produces[name] = set(slots)
        for slot, spec in slots.items():
            if not isinstance(spec, dict) or "type" not in spec:
                problems.append(f"{name}.produces.{slot} has no `type`")
            elif "required" in spec:
                problems.append(f"{name}.produces.{slot} carries `required`. Every slot is required")
    return produces


def as_list(value) -> list:
    if value is None:
        return []
    return value if isinstance(value, list) else [value]


def resolve(doc: dict, produces: dict[str, set[str]], ref: str) -> str | None:
    """Return a problem string when a binding reference does not resolve."""
    parts = str(ref).split(".")
    head = parts[0]
    if head == "fixed_dod":
        if len(parts) != 2 or parts[1] not in (doc.get("fixed_dod") or {}):
            return f"`{ref}` names no entry in fixed_dod"
        return None
    if len(parts) >= 3 and parts[1] == "review":
        review = ((doc.get("spawn") or {}).get("review_handover") or {}).get("produces") or {}
        if head not in produces:
            return f"`{ref}` binds an unknown stage `{head}`"
        if parts[2] not in review:
            return f"`{ref}` names no slot in spawn.review_handover.produces"
        return None
    if head not in produces:
        return f"`{ref}` binds an unknown stage `{head}`"
    if len(parts) != 2 or parts[1] not in produces[head]:
        return f"`{ref}` names no slot that `{head}` produces"
    return None


def check_bindings(doc: dict, kinds: dict, produces: dict[str, set[str]], problems: list[str]) -> None:
    lanes = {k: v for k, v in (doc.get("lanes") or {}).items() if isinstance(v, dict)}
    for lane, spec in lanes.items():
        for skipped in as_list(spec.get("skips")):
            if skipped not in kinds:
                problems.append(f"lane {lane} skips an unknown stage kind `{skipped}`")

    for name, kind in kinds.items():
        by_lane = kind.get("inputs_when") or {}
        overrides = list(by_lane) + list(kind.get("dod_authored_by_when") or {})
        for lane in overrides:
            if lane not in lanes:
                problems.append(f"stage kind `{name}` overrides an unknown lane `{lane}`")
            elif name in set(as_list(lanes[lane].get("skips"))):
                problems.append(f"stage kind `{name}` overrides lane {lane}, which skips it")
        for lane, spec in lanes.items():
            skips = set(as_list(spec.get("skips")))
            if name in skips:
                continue
            bound = by_lane.get(lane, kind.get("inputs")) or {}
            for slot, ref in bound.items():
                problem = resolve(doc, produces, ref)
                if problem:
                    problems.append(f"{name}.inputs.{slot} in lane {lane}: {problem}")
                    continue
                head = str(ref).split(".")[0]
                if head in skips:
                    problems.append(
                        f"{name}.inputs.{slot} in lane {lane} binds `{ref}`, but lane {lane} skips `{head}`"
                    )


def check_tracks(kinds: dict, problems: list[str]) -> None:
    if not TRACKS.is_dir():
        return
    for path in sorted(TRACKS.glob("*/track.yaml")):
        doc = load(path) or {}
        for stage in doc.get("stages") or []:
            where = f"{path.relative_to(ROOT)}"
            ident = stage.get("id", "<no id>")
            if not SLUG.match(str(ident)):
                problems.append(f"{where}: stage id `{ident}` is not a slug. See D-slug-ids")
            kind = stage.get("kind")
            if kind not in kinds:
                problems.append(f"{where}: stage `{ident}` uses an unknown kind `{kind}`")
            for key in COUNTED_ITEM_KEYS:
                items = stage.get(key)
                if isinstance(items, list) and len(items) > MAX_ITEMS:
                    problems.append(
                        f"{where}: stage `{ident}` has {len(items)} {key} items. The cap is {MAX_ITEMS}"
                    )


def main() -> int:
    problems: list[str] = []
    if not TEMPLATE.is_file():
        print(f"VIOLATION: {TEMPLATE.name} is missing")
        return 1
    doc = load(TEMPLATE)
    kinds = {kind["id"]: kind for kind in doc.get("stage_kinds") or []}
    if len(kinds) != len(doc.get("stage_kinds") or []):
        problems.append("two stage kinds share an id")

    check_gates(doc, problems)
    produces = check_shape(doc, kinds, problems)
    check_bindings(doc, kinds, produces, problems)
    check_tracks(kinds, problems)

    for problem in problems:
        print(f"VIOLATION: {problem}")
    print(f"VIOLATIONS {len(problems)} (stage kinds: {len(kinds)}, lanes: {len(doc.get('lanes') or {}) - 1})")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
