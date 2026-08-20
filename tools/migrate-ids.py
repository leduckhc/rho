#!/usr/bin/env python3

"""One-shot migration: numeric artifact ids become timestamped files and slug references.

Run from the repository root. It prints what it will do, and it changes nothing
unless `--apply` is passed.

The scheme, after this runs:

    docs/specs/<yyyymmdd-hhmmss>-SPEC-<slug>.md      referenced as SPEC-<slug>
    docs/adr/<yyyymmdd-hhmmss>-ADR-<slug>.md         referenced as ADR-<slug>
    .rho-work/decisions/<ts>-D-<slug>.md             referenced as D-<slug>
    docs/features.md rows keyed F-<slug>             referenced as F-<slug>

No counter exists anywhere, so no worktree can allocate the same id as another.
"""

from __future__ import annotations

import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]

SPEC_SLUGS = {
    "SPEC-core-runtime": "core-runtime",
    "SPEC-provider-interface": "provider-interface",
    "SPEC-tool-interface": "tool-interface",
    "SPEC-hooks-and-plugins": "hooks-and-plugins",
    "SPEC-tui": "tui",
    "SPEC-acp": "acp",
    "SPEC-background-tasks": "background-tasks",
    "SPEC-skills": "skills",
    "SPEC-mcp": "mcp",
    "SPEC-bash-sandbox": "bash-sandbox",
    "SPEC-subagents": "subagents",
    "SPEC-budget-governor": "budget-governor",
    "SPEC-config": "config",
    "SPEC-sessions": "sessions",
    "SPEC-steering": "steering",
    "SPEC-approval": "approval",
}

ADR_SLUGS = {
    "ADR-plugin-mechanism": "plugin-mechanism",
    "ADR-footprint": "footprint",
    "ADR-event-model": "event-model",
    "ADR-session-format": "session-format",
    "ADR-jsonl-codec": "jsonl-codec",
}

DECISION_SLUGS = {
    "D-own-session-format": "own-session-format",
    "D-acp-is-real-acp": "acp-is-real-acp",
    "D-benchmarks-owner": "benchmarks-owner",
    "D-website-direction": "website-direction",
    "D-sprint-one-hook-interfaces": "sprint-one-hook-interfaces",
    "D-core-event-model-widens": "core-event-model-widens",
    "D-acp-cancelled-spelling": "acp-cancelled-spelling",
    "D-session-context-accessor": "session-context-accessor",
    "D-cancel-wake-race": "cancel-wake-race",
    "D-provider-contract-crate": "provider-contract-crate",
    "D-session-config": "session-config",
    "D-todo-in-a-green-stage": "todo-in-a-green-stage",
    "D-no-four-argument-session-new": "no-four-argument-session-new",
    "D-secret-in-core": "secret-in-core",
    "D-no-git-writes-by-a-subagent": "no-git-writes-by-a-subagent",
    "D-bash-line-cap": "bash-line-cap",
    "D-plugin-does-not-classify-itself": "plugin-does-not-classify-itself",
    "D-provider-extension-verified-outside": "provider-extension-verified-outside",
    "D-bash-scrubs-credentials": "bash-scrubs-credentials",
    "D-plugin-trust-policy": "plugin-trust-policy",
    "D-bash-no-path-confinement": "bash-no-path-confinement",
    "D-project-skill-needs-trust": "project-skill-needs-trust",
    "D-skill-allowed-tools-ignored": "skill-allowed-tools-ignored",
    "D-mcp-does-not-classify-itself": "mcp-does-not-classify-itself",
    "D-mcp-shared-by-default": "mcp-shared-by-default",
    "D-one-redaction-home": "one-redaction-home",
    "D-jcode-edit-lessons": "jcode-edit-lessons",
    "D-shared-working-tree": "shared-working-tree",
    "D-jcode-bash-lessons": "jcode-bash-lessons",
    "D-three-tiers": "three-tiers",
    "D-bash-os-sandbox": "bash-os-sandbox",
    "D-measured-cost-and-cache": "measured-cost-and-cache",
    "D-budget-governor": "budget-governor",
    "D-no-cross-session-cache": "no-cross-session-cache",
    "D-sandbox-is-correctness": "sandbox-is-correctness",
    "D-child-confined-by-composition": "child-confined-by-composition",
    "D-retry-numbers-configurable": "retry-numbers-configurable",
    "D-two-weak-tests": "two-weak-tests",
    "D-append-only-jsonl": "append-only-jsonl",
    "D-truncated-tail-warns": "truncated-tail-warns",
    "D-write-failure-degrades": "write-failure-degrades",
    "D-recorder-consumes-events": "recorder-consumes-events",
    "D-redact-tool-arguments": "redact-tool-arguments",
    "D-cap-a-large-tool-result": "cap-a-large-tool-result",
    "D-serde-json-default-codec": "serde-json-default-codec",
    "D-credential-command-allowlist": "credential-command-allowlist",
    "D-config-fails-closed": "config-fails-closed",
    "D-bounded-steering-queue": "bounded-steering-queue",
    "D-cancel-keeps-the-session-open": "cancel-keeps-the-session-open",
    "D-measured-codec-choice": "measured-codec-choice",
    "D-approval-default-ask": "approval-default-ask",
    "D-ask-policy-fails-closed": "ask-policy-fails-closed",
    "D-resume-never-widens": "resume-never-widens",
    "D-redact-json-secrets": "redact-json-secrets",
    "D-reader-line-cap": "reader-line-cap",
    "D-no-remembered-execute-allow": "no-remembered-execute-allow",
    "D-approval-option-not-enum": "approval-option-not-enum",
    "D-unmappable-pi-record-drops": "unmappable-pi-record-drops",
    "D-writer-holds-one-sink": "writer-holds-one-sink",
    "D-one-timestamp-format": "one-timestamp-format",
    "D-reopen-stated-on-disk": "reopen-stated-on-disk",
    "D-log-capture-proves-itself": "log-capture-proves-itself",
}

# Two feature rows describe one feature each, twice. The catalogue must not list a
# feature twice, so the tier-2 row merges into the tier-1 row.
FEATURE_MERGES = {"F-lifecycle-hook-points": "F-lifecycle-hook-points", "F-slash-commands": "F-slash-commands"}


def slugify(text: str) -> str:
    text = text.strip().lower()
    text = re.sub(r"[^a-z0-9]+", "-", text)
    return text.strip("-")


def git_added(path: str) -> str:
    out = subprocess.run(
        ["git", "log", "--diff-filter=A", "--format=%cd", "--date=format:%Y%m%d-%H%M%S", "--", path],
        capture_output=True,
        text=True,
        cwd=ROOT,
    ).stdout.split()
    return out[-1] if out else "unknown"


def git_added_line(needle: str, path: str) -> str:
    out = subprocess.run(
        ["git", "log", "--format=%cd", "--date=format:%Y%m%d-%H%M%S", "-S", needle, "--", path],
        capture_output=True,
        text=True,
        cwd=ROOT,
    ).stdout.split()
    return out[-1] if out else "unknown"


def feature_slugs() -> dict[str, str]:
    text = (ROOT / "docs/features.md").read_text()
    rows = re.findall(r"^\| (F-[0-9A-Za-z]+) \| ([^|]+?) \|", text, re.M)
    out = {}
    for fid, name in rows:
        if fid in FEATURE_MERGES:
            continue
        out[fid] = slugify(name)
    seen: dict[str, str] = {}
    for fid, slug in out.items():
        if slug in seen:
            raise SystemExit(f"duplicate feature slug {slug}: {seen[slug]} and {fid}")
        seen[slug] = fid
    return out


def build_plan() -> dict:
    plan: dict = {"files": [], "refs": {}, "decisions": [], "names": {}}

    for old, slug in SPEC_SLUGS.items():
        matches = sorted((ROOT / "docs/specs").glob(f"{old}-*.md"))
        if len(matches) != 1:
            raise SystemExit(f"expected one file for {old}, found {matches}")
        src = matches[0]
        ts = git_added(str(src.relative_to(ROOT)))
        dst = src.parent / f"{ts}-SPEC-{slug}.md"
        plan["files"].append((str(src.relative_to(ROOT)), str(dst.relative_to(ROOT))))
        plan["names"][src.name] = dst.name
        plan["refs"][old] = f"SPEC-{slug}"

    for old, slug in ADR_SLUGS.items():
        matches = sorted((ROOT / "docs/adr").glob(f"{old}-*.md"))
        if len(matches) != 1:
            raise SystemExit(f"expected one file for {old}, found {matches}")
        src = matches[0]
        ts = git_added(str(src.relative_to(ROOT)))
        dst = src.parent / f"{ts}-ADR-{slug}.md"
        plan["files"].append((str(src.relative_to(ROOT)), str(dst.relative_to(ROOT))))
        plan["names"][src.name] = dst.name
        plan["refs"][old] = f"ADR-{slug}"

    body = (ROOT / ".rho-work/DECISIONS.md").read_text()
    blocks = re.split(r"^## (D-\d+) — ", body, flags=re.M)
    header = blocks[0]
    pairs = list(zip(blocks[1::2], blocks[2::2]))
    if len(pairs) != len(DECISION_SLUGS):
        raise SystemExit(f"found {len(pairs)} decisions, mapped {len(DECISION_SLUGS)}")
    for old, chunk in pairs:
        slug = DECISION_SLUGS[old]
        ts = git_added_line(f"## {old} —", ".rho-work/DECISIONS.md")
        title, rest = chunk.split("\n", 1)
        plan["decisions"].append(
            {
                "old": old,
                "slug": slug,
                "ts": ts,
                "title": title.strip(),
                "body": rest.rstrip() + "\n",
                "path": f".rho-work/decisions/{ts}-D-{slug}.md",
            }
        )
        plan["refs"][old] = f"D-{slug}"
    plan["header"] = header

    for old, slug in feature_slugs().items():
        plan["refs"][old] = f"F-{slug}"
    for old, target in FEATURE_MERGES.items():
        plan["refs"][old] = plan["refs"][target]

    return plan


def rewrite_text(text: str, refs: dict[str, str], names: dict[str, str] | None = None) -> tuple[str, int]:
    count = 0
    # A file name must be rewritten before an id, or `20260818-014343-SPEC-sessions.md` would become
    # `SPEC-sessions-sessions.md`. So the longest, most specific form goes first.
    for old_name in sorted(names or {}, key=len, reverse=True):
        new_name = names[old_name]
        n = text.count(old_name)
        if n:
            text = text.replace(old_name, new_name)
            count += n
    # Longest ids next, so D-1 never eats D-16.
    for old in sorted(refs, key=len, reverse=True):
        new = refs[old]
        pattern = re.compile(rf"(?<![0-9A-Za-z_-]){re.escape(old)}(?![0-9A-Za-z])")
        text, n = pattern.subn(new, text)
        count += n
    return text, count


def target_files() -> list[pathlib.Path]:
    out: list[pathlib.Path] = []
    patterns = (
        "docs/**/*.md",
        "crates/**/*.rs",
        "crates/**/*.md",
        "crates/**/Cargo.toml",
        ".rho-work/*.md",
        ".rho-work/**/*.md",
        "*.md",
        "*.yaml",
        "web/**/*.astro",
        "web/**/*.css",
        "web/**/*.md",
        "tools/*.py",
        "bench/**/*.md",
    )
    # `.github/` is owned by another agent, so this migration leaves it alone. The id
    # guard skips it for the same reason, and the controller reports the one reference.
    for pattern in patterns:
        out.extend(p for p in ROOT.glob(pattern) if p.is_file() and "target/" not in str(p))
    return sorted(set(out))


def main() -> None:
    apply = "--apply" in sys.argv
    plan = build_plan()
    print(f"file moves: {len(plan['files'])}")
    print(f"decisions to split: {len(plan['decisions'])}")
    print(f"reference mappings: {len(plan['refs'])}")

    total = 0
    touched = 0
    for path in target_files():
        text = path.read_text()
        new_text, n = rewrite_text(text, plan["refs"], plan["names"])
        if n:
            touched += 1
            total += n
            if apply:
                path.write_text(new_text)
    print(f"references rewritten: {total} across {touched} files")

    if not apply:
        print("dry run, nothing changed. Pass --apply to write.")
        (ROOT / "/tmp/id-plan.json").write_text("")
        return

    for src, dst in plan["files"]:
        subprocess.run(["git", "mv", src, dst], check=True, cwd=ROOT)

    # The catalogue listed two features twice. A restated row is a duplicate, and after the
    # slug rewrite it would be a duplicate id too. So the restated rows go, and one
    # sentence points at the canonical rows.
    features_path = ROOT / "docs/features.md"
    text = features_path.read_text()
    kept = []
    dropped = 0
    for line in text.split("\n"):
        if line.startswith("| F-lifecycle-hook-points |") and "A hook observes a session start" in line:
            dropped += 1
            continue
        if line.startswith("| F-slash-commands |") and "an extension answers" in line:
            dropped += 1
            continue
        kept.append(line)
    text = "\n".join(kept)
    text = text.replace(
        "## Subagents",
        "A hook point and a slash command are one feature each, so this table no longer\n"
        "restates them. See `F-lifecycle-hook-points` and `F-slash-commands` above.\n\n"
        "## Subagents",
        1,
    )
    features_path.write_text(text)
    print(f"duplicate feature rows dropped: {dropped}")

    decisions_dir = ROOT / ".rho-work/decisions"
    decisions_dir.mkdir(exist_ok=True)
    index = [
        "# rho decisions",
        "",
        "One decision, one file. A shared append-only file made two agents in two",
        "worktrees collide on every entry, so each decision now owns its own file.",
        "",
        "The reference form is `D-<slug>`. `docs/ids.md` maps every old numeric id.",
        "",
        "| Decision | Date | Title |",
        "| --- | --- | --- |",
    ]
    for item in plan["decisions"]:
        slug_ref = f"D-{item['slug']}"
        body, _ = rewrite_text(item["body"], plan["refs"], plan["names"])
        title, _ = rewrite_text(item["title"], plan["refs"], plan["names"])
        path = ROOT / item["path"]
        path.write_text(f"# {slug_ref} — {title}\n\n{body}")
        date = item["ts"].split("-")[0]
        index.append(f"| [{slug_ref}]({pathlib.Path(item['path']).name}) | {date} | {title} |")
    (decisions_dir / "README.md").write_text("\n".join(index) + "\n")
    (ROOT / ".rho-work/DECISIONS.md").unlink()

    with open("/tmp/id-plan.json", "w") as handle:
        json.dump({"refs": plan["refs"], "files": plan["files"], "names": plan["names"]}, handle, indent=2)
    print("applied. Plan written to /tmp/id-plan.json")


if __name__ == "__main__":
    main()
