# Skills

This page shows you how to write a skill and get rho to load it (rho 0.1.0).

A skill is a folder of instructions stored in a Markdown file.
rho reads the file when the task matches, so the model gets procedure-level
detail only when it is relevant.

## Write your first skill

Create a skill directory and a `SKILL.md` inside it.

```
mkdir -p ~/.rho/skills/remind-dates
```

```
cat > ~/.rho/skills/remind-dates/SKILL.md << 'EOF'
---
name: remind-dates
description: >
  Use when the user asks to add or check dates in a calendar file.
  Read this file for the exact format to apply.
---

Always write dates as ISO 8601: 2026-01-15, not Jan 15 or 15/01.
Check for duplicate entries before adding a new one.
EOF
```

Start rho in any directory.

```
rho
```

rho tells you what it loaded, and what it withheld.
In `rho run` the notices go to stderr with a `rho:` prefix.
In the terminal interface they appear as transcript rows that start `! notice ·`.
A withheld skill gets a named notice like this:

```
rho: 1 project skill(s) were found and not loaded: remind-dates. …
```

A skill loaded from `~/.rho/skills` is a user skill and loads without any notice.
The model sees it in its `<available_skills>` block.

## What the model sees

rho builds this block in the system prompt:

```xml
<available_skills>
  <skill>
    <name>remind-dates</name>
    <description>Use when the user asks to add or check dates …</description>
    <location>/Users/you/.rho/skills/remind-dates/SKILL.md</location>
  </skill>
</available_skills>
```

rho reads only the front matter at load time, and never the body.
The model receives the `name`, the `description`, and the file path.
The model calls the `read` tool to load the body when it decides the skill
applies.
The five XML metacharacters (`&`, `<`, `>`, `"`, `'`) are escaped, so a
description cannot break the block.

## Front matter reference

Every field goes between `---` fences at the top of `SKILL.md`.
rho reads at most 16 KiB of the file.

| Field | Required | Limit | Behaviour when wrong |
|---|---|---|---|
| `name` | no | 64 characters, lowercase letters, digits, hyphens only | Missing: directory name is used with a warning. Too long or wrong characters: loads with a warning. |
| `description` | **yes** | 1024 characters | Missing or blank: skill is skipped. Over 1024 characters: truncated with a warning. |
| `disable-model-invocation` | no | boolean | When `true`, the skill is excluded from the prompt block entirely. |
| `allowed-tools` | — | — | rho ignores this field and warns you. The approval policy is the only authority. |

Name rules in full: 1 to 64 characters, lowercase ASCII letters, digits, and
hyphens only, no leading or trailing hyphen, no double hyphen.

rho sanitises names and descriptions before they reach the terminal.
Control characters become the replacement glyph `\u{fffd}`.

## Where rho looks

rho searches these locations in order:

1. `~/.rho/skills` — user skills, always trusted.
2. `~/.agents/skills` — shared user skills, always trusted.
3. `.rho/skills` in the session root — project skill, withheld until trusted.
4. `.agents/skills` in the session root and every ancestor up to the git root —
   project skills, withheld until trusted.

The first skill with a given name wins.
If a later skill shares that name, rho warns you and ignores the duplicate.

The skill walk stops at the git root. [Project instructions](project-instructions.md) walk
further, up to your home directory. So an `AGENTS.md` in a directory above your repository
still applies, and a skill in that directory does not.

## Project skills and trust

A project skill is withheld by default.

```
rho: 1 project skill(s) were found and not loaded: my-skill.
A skill can instruct the model and can carry scripts, so a skill from this
repository stays off until you trust it. Pass --trust-project to load them.
```

Pass `--trust-project` to load it:

```
rho --trust-project
```

See [permissions](permissions.md) for what trust means.

## Load one skill by path

```
rho --skill path/to/my-skill
```

`--skill` accepts a directory that holds `SKILL.md`, or a `.md` file directly.
It loads even when `--no-skills` is set.

## Stop discovery

```
rho --no-skills
```

This stops the directory search.
An explicit `--skill` still loads.

`--no-skills` stops the skill search only. It leaves subagents alone, and `--no-agents` stops
those. See [subagents](subagents.md).

## Other files beside SKILL.md

A skill directory may hold scripts, config files, or any other asset.
The body of `SKILL.md` can reference them with relative paths.
The model resolves a relative path against the skill directory, then calls
`read` or a shell tool to load the file.

## When a skill is the wrong tool

Use a skill when you want rho to follow a reusable procedure on demand.

Use `AGENTS.md` when you want a rule that applies to every session in a project,
not just when the model chooses a skill.
That is a different mechanism with different trust.

See [project instructions](project-instructions.md).
> See [permissions](permissions.md) for the trust boundary between project
> instructions and skills.

## What does not work yet

`allowed-tools` in a skill's front matter has no effect.
rho warns you when it finds the field and ignores it.
The approval policy is the only authority over which tools run.
