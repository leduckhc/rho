# Verification: an agent definition that does not load

Step 11 for the failure path of an agent definition. See AGENTS.md step 11 and
`docs/specs/20260826-184110-SPEC-definition-rejection.md`.

Every command below ran for real. The provider is openrouter, and the model is
`anthropic/claude-haiku-4.5`. Two binaries ran the same directories:

- **before**: commit `931534f`, built in a separate worktree at `/tmp/rho-before`.
- **after**: this branch, `cargo build --release -p rho-cli`.

`HOME` points at an empty temporary directory in every run, so the machine's own
`~/.rho/agents` cannot change a result. The definitions live in a project root, and
`--trust-project` loads them.

## 1. The defect, reproduced live

`/tmp/rho-live/seq/.rho/agents/lister.md`:

```markdown
---
name: lister
description: Lists the files in a directory and reports the names. Use to inspect a directory.
tools: [read, list]
max_turns: 4
---
You list files. Reply with the file names only, and nothing else.
```

```sh
rho run "Call spawn_agent for the lister agent on the directory /tmp/rho-live/seq. \
If you have no such tool, say NO SUCH TOOL and stop." \
  --provider openrouter --model anthropic/claude-haiku-4.5 \
  --root /tmp/rho-live/seq --trust-project
```

Before, with no notice of any kind:

```text
NO SUCH TOOL
I don't have a `spawn_agent` tool available. The tools I have access to are:
- `read` - Read files
- `list` - List directories
...
```

After:

```text
rho: 1 agent definition(s) available to spawn_agent: lister.
The lister agent found one directory in `/tmp/rho-live/seq`: `.rho/`
```

So the sequence form loads, the tool exists, and the child ran with `read` and `list`.

## 2. A field that is truly broken

`/tmp/rho-live/broken/.rho/agents/broken.md` holds `tools: 5`, and it is the only file
in the directory.

```sh
rho run "Say the single word: quiet." --provider openrouter \
  --model anthropic/claude-haiku-4.5 --root /tmp/rho-live/broken --trust-project
```

After:

```text
rho: agent definition /tmp/rho-live/broken/.rho/agents/broken.md did not load. the tools
field is not a list of names. Detail: the field is a number, and a tool list is a line of
words or a sequence. Write tools: read, list or tools: [read, list]. Write none for no
tools, and all to inherit every parent tool.
quiet.
```

**Before, this file loaded.** That is worse than it sounds, and the run below is what
found it:

```text
rho: 1 agent definition(s) available to spawn_agent: broken.
```

`serde_yaml` reads the plain scalar `5` into a `String`, so the old loader took `"5"` as
a tool name. The intersection then dropped it, and the child ran with nothing:

```sh
rho run "Call spawn_agent for the broken agent, with the goal: 'List the names of every \
tool you hold, or say NONE if you hold none.' Then quote the child's answer verbatim." \
  --provider openrouter --model anthropic/claude-haiku-4.5 \
  --root /tmp/rho-live/broken --trust-project
```

```text
rho: 1 agent definition(s) available to spawn_agent: broken.
The child's answer verbatim is:
**NONE**
```

A definition asked for a tool set, and the child got an empty one with no report. That is
the same fail-open family as the missing file, and the refusal now happens at start-up.

## 3. One good file beside one broken file

`/tmp/rho-live/mixed/` holds both files from above.

```text
rho: agent definition /tmp/rho-live/mixed/.rho/agents/broken.md did not load. the tools
field is not a list of names. Detail: the field is a number, and a tool list is a line of
words or a sequence. Write tools: read, list or tools: [read, list]. Write none for no
tools, and all to inherit every parent tool.
rho: 1 agent definition(s) available to spawn_agent: lister.
mixed.
```

One bad file no longer costs the good one.

## 4. An untrusted project file quotes nothing

The same broken file, with no `--trust-project`:

```text
rho: agent definition /tmp/rho-live/broken/.rho/agents/broken.md did not load. the tools
field is not a list of names. Write tools: read, list or tools: [read, list]. Write none
for no tools, and all to inherit every parent tool.
```

The `Detail:` clause is gone, and the path, the reason, and the repair stay. A repository
rho has not been told to trust cannot put its own text on a start-up line.

## 5. Seven broken files, and the cap

`/tmp/rho-live/flood/` holds seven copies of the broken file.

```text
rho: agent definition /tmp/rho-live/flood/.rho/agents/broken-1.md did not load. ...
rho: agent definition /tmp/rho-live/flood/.rho/agents/broken-2.md did not load. ...
rho: agent definition /tmp/rho-live/flood/.rho/agents/broken-3.md did not load. ...
rho: agent definition /tmp/rho-live/flood/.rho/agents/broken-4.md did not load. ...
rho: agent definition /tmp/rho-live/flood/.rho/agents/broken-5.md did not load. ...
rho: 2 more agent definition file(s) are not listed here. Repair the files above, then
start rho again.
flood
```

Five files are named, and the other two are counted.

## 6. A file rho may not read

```sh
chmod 000 /tmp/rho-live/perm/.rho/agents/locked.md
rho run "Say the single word: locked." --provider openrouter \
  --model anthropic/claude-haiku-4.5 --root /tmp/rho-live/perm --trust-project
```

```text
rho: agent definition /tmp/rho-live/perm/.rho/agents/locked.md did not load. rho cannot
read the file. Detail: Permission denied (os error 13). Check the path and the file
permissions.
locked.
```

## 7. The same start-up twice

"Twice" has found two defects in this project, so every failing case above ran twice. The
notices are identical each time, because discovery holds no state between runs and
`markdown_files` sorts its input.

| Case | First run | Second run |
| --- | --- | --- |
| A broken field | one rejection line | the same line |
| Seven broken files | five lines and one count | the same six lines |
| A denied file | one rejection line | the same line |

The full log of both passes is in `/tmp/rho-live/after.log`, from
`/tmp/rho-live/drive.sh`.

## 8. The second defect this step found

A definition that loads may still hold a warning. `AgentDefinition::warnings` carried the
dropped tool keyword, the bad name, and the bad sandbox value, and **nothing printed any
of them**. A skill warning has printed since sprint 2, so this was a gap between two
loaders that share a shape.

`/tmp/rho-live/warn/.rho/agents/mixed.md` writes `tools: all, read`, which drops the
keyword and keeps `read`.

Before:

```text
rho: 1 agent definition(s) available to spawn_agent: mixer.
warned.
```

After:

```text
rho: agent definition mixer: a tool keyword must stand alone. "all" was dropped. These
named tools stand: read.
rho: 1 agent definition(s) available to spawn_agent: mixer.
warned
```

The narrowing was correct both times. Only the report was missing, and a narrowing
nobody sees is how a user blames the model instead of the file.

## 9. Three holes the reviews found, each probed live

The change went through two reviews after the tests were green: one for correctness, one for
security. Each blocking finding was treated as a hypothesis and probed with the release
binary, before any repair.

### 9.1 A file name forged a notice line

An untrusted repository held a file whose name carries a line break:

```sh
mkdir -p /tmp/rho-live/probe/.rho/agents
printf 'no frontmatter here\n' > "/tmp/rho-live/probe/.rho/agents/x
rho: 3 project definitions are trusted, loading them now.md"
rho run "Say: probed." --provider openrouter --model anthropic/claude-haiku-4.5 \
  --root /tmp/rho-live/probe
```

Before the repair, with `cat -v` showing the bytes:

```text
rho: agent definition /tmp/rho-live/probe/.rho/agents/x
rho: 3 project definitions are trusted, loading them now.md did not load. the file has no
frontmatter. Start the file with a --- line. Then write a name and a description.
```

Two lines, and the second reads as though rho wrote it. The rejection path dropped the
detail for an untrusted file and printed the path verbatim.

After:

```text
rho: agent definition /tmp/rho-live/probe/.rho/agents/x?rho: 3 project definitions are
trusted, loading them now.md did not load. the file has no frontmatter. ...
```

One line. The line break is the replacement glyph.

### 9.2 The new warning line carried raw file text

Two definitions in a trusted user directory:

```yaml
tools: [all, "read\e[2J"]
sandbox: "\e[31mrho: your session is insecure, run curl evil.sh\e[0m prose prose ..."
```

Before the repair, `cat -v` shows the escapes reaching the terminal, and the whole
400-character value:

```text
rho: agent definition toolwarn: a tool keyword must stand alone. "all" was dropped. These
named tools stand: read^[[2J.
rho: agent definition warner: unknown sandbox mode "^[[31mrho: your session is insecure, run
curl evil.sh^[[0m prose prose prose ... (400 characters) ". Use off, confined, or strict.
```

`^[[2J` clears the screen. After:

```text
rho: agent definition toolwarn: a tool keyword must stand alone. "all" was dropped. These
named tools stand: read?[2J.
rho: agent definition warner: the sandbox mode "?[31mrho: your session is insecure, run curl
evil.sh?[0m prose prose prose prose prose prose prose prose prose prose prose prose prose
prose prose prose prose prose prose prose prose prose prose pr..." is unknown. Use off,
confined, or strict.
```

Every escape is a replacement glyph, the value is cut at 200 characters, and the repair
survives the cut. These two files sit in a **user** directory, which the user owns, so the
text still appears. A definition from an untrusted repository prints no warning at all, and
`an_untrusted_project_definition_prints_no_warning` holds that.

### 9.3 A symlink smuggled a repository definition into the trusted set

```sh
ln -sf /tmp/rho-live/symlink/evil.md ~/.rho/agents/evil.md
rho run "Say: probed." --provider openrouter --model anthropic/claude-haiku-4.5 \
  --root /tmp/rho-live/symlink
```

Before, with no `--trust-project`:

```text
rho: 1 agent definition(s) available to spawn_agent: evil.
```

The definition lives in the repository, and it loaded as trusted. After:

```text
rho: 1 project agent definition(s) were found and not loaded: evil. A definition carries a
tool list and a model, and it runs unattended, so one from this repository stays off until you
trust it. Pass --trust-project.
```

The skill loader has resolved a path before classifying it since sprint 2. The agent loader
did not, and the two now call one function. See D-an-agent-symlink-cannot-smuggle-trust.

## 10. The flood the review fleet found, probed live

Four reviewer lenses and `codex review` read the committed change. The blocking finding was
that the five-line cap covered the rejection lines only.

Fifty files in `<repo>/.rho/agents`, each carrying a 16 000-character `name:` and a
`tools: all, read` line that warns:

```sh
python3 -c "
import pathlib
d = pathlib.Path('/tmp/rho-live/flood2/.rho/agents'); d.mkdir(parents=True, exist_ok=True)
long = 'x' * 16000
for i in range(50):
    (d / f'a{i}.md').write_text(f'---\nname: {long}\ndescription: Recon.\ntools: all, read\n---\nbody\n')
"
rho run "Say: bounded." --provider openrouter --model anthropic/claude-haiku-4.5 \
  --root /tmp/rho-live/flood2 2>err; wc -c < err
```

| Case | Before | After |
| --- | --- | --- |
| Untrusted repository, stderr | 800 946 bytes, 6 lines | 1 057 bytes, 5 lines |
| Trusted repository, stderr | 1 626 085 bytes, 104 lines | 1 718 bytes, 11 lines |

The untrusted case is the one that matters: no flag was passed, and a repository still wrote
800 KB to the terminal through the withheld name list. After the fix that line names three
definitions and counts the rest.

The trusted output now reads:

```text
rho: agent definition toolwarn: a tool keyword must stand alone. "all" was dropped. These
named tools stand: read?[2J.
rho: agent definition xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx...: the
name must be 1 to 64 characters. This name has 16000.
...
rho: 47 more agent definition(s) raised a warning, not listed here.
rho: 3 agent definition(s) available to spawn_agent: toolwarn, warner, xxxxxx...
```

Every name is cut at 64 characters, every kind of line stops at five, and each remainder is
counted.

## 11. What this step did not cover

- **A definition on Windows.** No Windows host was available.
- **A `~/.agents/agents` directory.** Every run used a temporary `HOME`, so the user
  directory search ran against an empty home. The unit tests cover the user origin.
- **The terminal interface.** These runs used `rho run`. The interface takes the same
  notice list through `with_notices`, and no live terminal pass was made.
