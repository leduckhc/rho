# SPEC-definition-rejection — A definition file that does not load

Status: delivered.
Owning crates: `rho-skills` for the loader and the reason set, `rho-cli` for the notice.
Features: F-agent-definitions and F-tool-list-keywords. See `docs/features.md`.

Decisions: D-a-rejected-definition-is-reported, D-a-tool-list-accepts-a-yaml-sequence, and
D-an-agent-symlink-cannot-smuggle-trust.

Reviewed once before any code. The review changed the contract in four places. See section 8.

## 1. The defect this spec repairs

`load_definition` returned `Option<AgentDefinition>`. Every failure was one `None`, so the
reason died at the call site. `discover_agents` then dropped the file, and `AgentSet` had no
place to record it.

The visible result was worse than a lost file. rho registers `spawn_agent` only when at least
one definition loaded. So one bad line in one file removed five tools from the session
(`spawn_agent`, `spawn_agents`, `steer_agent`, `agent_status`, and `cancel_agent`), and the
model told the user it had no such tool. Two live runs were spent finding it. See
`docs/verification/subagent-slot-queue.md` section 6.

The file that started it wrote `tools: [read, list]`. That is valid YAML and a reasonable
spelling. rho read the field as a string only.

## 2. The sides, and who owns each one

| Side | Crate | What it must agree on |
| --- | --- | --- |
| The loader | `rho-skills` | Which files load, and the name of each failure. |
| Discovery | `rho-skills` | Where a rejection is kept, and what a project file may quote. |
| The command line | `rho-cli` | How a rejection reaches the user. |
| A definition file on disk | the user | Which spellings of `tools` are valid. |

Contract kinds this change touches: the public API, the data model, the error taxonomy, the
persisted format of a definition file, and the behaviour rule that a refusal must teach.

## 3. The contract

```rust
/// A short piece of text that explains a rejection.
///
/// The text may quote a file inside the repository under edit, so it is untrusted. **The
/// type is the guarantee.** A `Detail` holds no control character and at most 200
/// characters, because no caller can build one any other way. Only `rho-skills` builds one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detail(String);

impl Detail {
    /// The text. It is safe to draw.
    pub fn as_str(&self) -> &str;

    /// True when there is nothing to show. A withheld detail is empty.
    pub fn is_empty(&self) -> bool;
}

impl std::fmt::Display for Detail { /* the text, verbatim */ }

/// Why one agent definition file did not load.
///
/// Every case is named. There is no catch-all variant, because a catch-all lets a new
/// reason ship with no message and no test. See decision D-a-rejected-definition-is-reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RejectionReason {
    /// The file could not be read. The detail is the io error.
    Unreadable { detail: Detail },
    /// The file does not start with a `---` line.
    NoFrontmatter,
    /// The file starts a frontmatter block and never closes it, inside the bounded read.
    UnclosedFrontmatter,
    /// The frontmatter is not valid YAML, or a field holds the wrong type, or a key repeats.
    BadFrontmatter { detail: Detail },
    /// The frontmatter has no `description`, and the model reads only that.
    NoDescription,
    /// The `tools` field is neither a string nor a sequence of strings.
    BadToolsField { detail: Detail },
}

impl RejectionReason {
    /// What the user must change. One sentence, from an exhaustive match.
    pub fn repair(&self) -> &'static str;

    /// The detail, or an empty one for a reason that carries none.
    pub fn detail(&self) -> &Detail;
}

/// One definition file that did not load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedDefinition {
    /// The file that did not load.
    pub path: PathBuf,
    /// Where the file came from. A project file is still reported.
    pub origin: SkillOrigin,
    /// Why it did not load.
    pub reason: RejectionReason,
}

impl RejectedDefinition {
    /// One line for the user. It names the file, the reason, and the repair.
    ///
    /// A frontend may render the fields instead. That is the extension point.
    pub fn notice(&self) -> String;

    /// The same rejection with the detail dropped.
    ///
    /// Discovery calls this for a project file that the user has not trusted, so an
    /// untrusted repository cannot put its own prose on a start-up line.
    pub fn without_detail(self) -> Self;
}

/// The set an agent discovery pass found.
#[derive(Clone, Debug, Default)]
pub struct AgentSet {
    /// Definitions that may be used now.
    pub loaded: Vec<AgentDefinition>,
    /// Project definitions found but withheld, because the project is not trusted.
    pub withheld: Vec<AgentDefinition>,
    /// Files that did not load at all, with the reason for each one.
    pub rejected: Vec<RejectedDefinition>,
}

/// Load one definition from a path.
///
/// The error side carries the reason, so no caller can drop it by accident.
pub async fn load_definition(
    path: &Path,
    origin: SkillOrigin,
) -> Result<AgentDefinition, RejectedDefinition>;
```

`discover_agents` keeps the same signature. It pushes every rejection onto
`AgentSet::rejected`. For a project file that the user has not trusted, it pushes
`without_detail` instead.

A definition that loads may also owe the user a line, so it renders its own:

```rust
/// The most lines of one kind a start-up report may print. The rest are counted.
pub const MAX_LINES_PER_KIND: usize = 5;

impl AgentDefinition {
    /// The name, made safe to draw and short enough to read.
    pub fn safe_name(&self) -> String;

    /// The lines this definition owes the user, and no more than a bounded number.
    pub fn notices(&self) -> Vec<String>;
}
```

A name comes from a file, so a repository chooses it. The loader sanitises it, `safe_name`
bounds it, and `notices` bounds the count. Section 11 says what that cost before it existed.

**A path is drawn safely too.** `notice` renders the path through the same sanitiser, and it
cuts a path longer than 200 characters **on the left**, so the file name at the end stays. A
file name is repository text: one holding a line break forged a second `rho:` line in a live
run.

**A warning is bounded by the same type.** A warning interpolates the file's own text: a tool
name, a sandbox value, a file stem. Each one goes through `Detail`, so no warning can carry a
control character or unbounded prose. The `sandbox` warning is rho's own sentence now, because
the message from `SandboxMode::from_str` quotes the whole value and the repair sat after it.

**What a second frontend can reuse, and what it cannot.** `RejectedDefinition`,
`RejectionReason`, and `Detail` live in `rho-skills`, so any frontend renders a rejection from
the data or calls `notice`. `AgentDefinition::notices` and `safe_name` are there too. The
**assembly** of a whole report is not: `notices_for` sits in `rho-cli`, and no crate may depend
on `rho-cli`. So a second frontend would repeat that function's ordering and its caps. Section
11 says why that is deliberate today.

### What the contract forbids

- No catch-all reason. A new case is a new variant, and `repair` does not compile without it.
- No partial load. A file with a broken field does not load with the field ignored, because an
  ignored `tools` field inherits the parent's whole tool set.
- No raw detail. `Detail` is the only way to carry one, and its constructor is private to
  `rho-skills`.
- No untrusted prose on a start-up line. An untrusted project file is reported by path,
  reason, and repair, and it quotes nothing.
- No silent rejection. A caller that holds an `AgentSet` holds every reason.

### What an old reader does with the new field

`AgentSet` carries `#[non_exhaustive]`. So only this crate writes the literal, and a later
field breaks no caller. `discover_agents` and `Default` build every set.

A reader that ignores `rejected` loses the report. Two things stop that. `Result` is
`#[must_use]`, so an accidental drop warns. And `discover_agents` files every outcome through
`admit`, which pushes each one onto the set. No call site discards a result.

## 4. The `tools` field, in both spellings

| Written | Meaning |
| --- | --- |
| `tools: read, list` | Two names. The comma form. |
| `tools: read list` | Two names. Space separated. |
| `tools: [read, list]` | Two names. A YAML flow sequence. |
| `tools:` then `  - read` | A YAML block sequence. |
| `tools: all` or `tools: "*"` | Inherit the parent's whole set. |
| `tools: none` | No tools. |
| `tools: []` | No tools. An empty list. |
| field absent | Inherit the parent's whole set. |

The keyword rules of D-a-tool-keyword-stands-alone apply after the form is resolved, so a
sequence and a line behave the same.

These reject with `BadToolsField`: an empty `tools:` line, a number, a boolean, a map, a
sequence that holds a value which is not a string, and a nested sequence.

An empty `tools:` line rejects on purpose. It looks like an absent field, and an absent field
inherits every parent tool. So the safe reading of an empty line is no reading at all.

### Which fault wins

A file may break two rules. The loader reports the first fault it meets, in this order: the
read, the frontmatter fences, the YAML parse, the description, then the `tools` field. So a
file with no description **and** a bad `tools` field reports the description. The user meets
the second fault on the next run. "The same thing twice" is a step in AGENTS.md for this
reason, and it is how a second fault surfaces.

### Every other field

`model` and `sandbox` stay strings. `max_turns` stays a number. A wrong type in any of them
rejects the file with `BadFrontmatter`, and the detail names the field. A repeated key rejects
the file the same way, because `serde_yaml` refuses a duplicate field. An unknown key is still
ignored, and that rule does not change.

## 5. The notice

`rho-cli` builds one line per rejected file, from `RejectedDefinition::notice`. The line names
the path, the reason, and the repair. The lines print before the session starts, with the
other start-up notices.

Three rules:

- A rejection prints even when no definition loaded. That case removed the tool, so it is the
  case that most needs the report.
- At most `MAX_LINES_PER_KIND` lines of **each kind** print. The rest become one counted
  line. A rejection, a warning, and a name list are all bounded, in count and in length,
  because a repository chooses how many files it holds and how long each name is.
- A **warning** on a definition that did load prints too, with the definition name. A
  warning is not a fault, so the file still loads. Nothing printed one before, and section 9
  says what that cost.
- A withheld definition prints **no** warning. It changes nothing in this session, and its
  warning would carry prose from a repository the user has not trusted. Once the user passes
  `--trust-project`, the same warning prints.

One function builds every line, `notices_for`. One place, because a second builder is how a
report gets lost.

## 6. Test cases

The tool list, in `crates/rho-skills/tests/agents.rs`:

- `a_yaml_sequence_tool_list_loads_the_same_as_a_comma_list` — the defect, reproduced. A file
  with `tools: [read, list]` loads, and it holds the two names.
- `a_block_sequence_tool_list_loads` — the indented `- name` form loads too.
- `a_keyword_in_a_sequence_still_stands_alone` — `[all]` inherits, and `[all, read]` keeps
  `read` and warns.
- `a_tools_field_that_is_not_a_list_is_rejected_and_says_so` — `tools: 5` rejects with
  `BadToolsField`, and the notice names the file and the type it found.
- `a_tool_name_that_is_not_a_string_is_rejected` — `tools: [read, 5]` rejects, and the detail
  names the item.
- `an_empty_tools_line_is_rejected_rather_than_inherited` — `tools:` with no value rejects,
  and the repair names `none` and `all`.

The reason set, in `crates/rho-skills/tests/agents.rs`:

- `a_definition_without_a_description_does_not_load` — the reason is `NoDescription`.
- `a_file_with_broken_yaml_is_rejected_with_the_reason` — an unclosed quote rejects with
  `BadFrontmatter`, and the notice holds the parser detail.
- `a_file_with_no_frontmatter_is_rejected_with_the_reason` — `NoFrontmatter`. An empty file
  gets the same reason.
- `an_unclosed_frontmatter_block_says_it_is_unclosed` — a file that opens `---` and never
  closes it rejects with `UnclosedFrontmatter`. A block larger than the bounded read is the
  same fault. The repair names the closing line, and it never asks for a description the
  file already holds.
- `a_wrong_type_in_a_scalar_field_is_rejected_with_the_field_name` — `max_turns: "12"`
  rejects with `BadFrontmatter`, and the detail names `max_turns`.
- `a_repeated_key_is_rejected` — two `tools` lines reject with `BadFrontmatter`.
- `frontmatter_that_is_not_a_mapping_is_rejected` — a bare YAML list rejects with
  `BadFrontmatter`.
- `a_missing_file_is_rejected_as_unreadable` — `Unreadable`, and the path is the missing one.
- `a_long_parser_message_is_capped_before_it_reaches_the_user` — a parser message that quotes
  400 characters of the file arrives capped, and it says it was cut.

The type that carries a detail, in `crates/rho-skills/src/rejection.rs`:

- `a_rejection_detail_is_sanitised_and_bounded` — the constructor strips every control
  character and caps the text. The test drives the constructor, because the type is the
  guarantee.
- `every_rejection_reason_states_a_repair` — every variant returns a non-empty repair and a
  non-empty explanation, and `notice` holds the path, the reason, and the repair. The match
  inside the test carries no wildcard, so a new variant does not compile until it is listed.
- `a_withheld_rejection_keeps_its_reason_and_drops_its_detail` — `without_detail` keeps the
  variant and empties the text.
- `every_reason_explains_its_own_subject` — each reason names the thing it is about, so two
  arms swapped by a copy and paste fail. `notice` is built from `explain`, so a test that only
  reads the notice cannot see that.

Discovery, in `crates/rho-skills/tests/agents.rs`:

- `a_broken_definition_is_reported_and_the_good_one_still_loads` — one bad file and one good
  file in one directory. The good one loads, and the bad one is in `rejected`.
- `a_broken_project_definition_is_rejected_and_not_hidden` — an untrusted project file that
  does not parse lands in `rejected` with `SkillOrigin::Project`, and not in `withheld`.
- `an_untrusted_project_file_quotes_nothing_in_its_rejection` — the security rule. The detail
  is dropped for an untrusted project file, and the path, the reason, and the repair stay. A
  trusted project file keeps its detail.

What a line may carry, in `crates/rho-skills/tests/agents.rs`:

- `a_warning_carries_no_control_character_and_no_unbounded_text` — a file with an escape in a
  tool name and 400 characters in `sandbox` produces warnings that hold neither.
- `a_hostile_file_name_cannot_forge_a_notice_line` — a file name with a line break yields one
  line, and no control character.
- `a_very_long_path_is_cut_on_the_left_so_the_file_name_stays` — a 250-character path keeps its
  file name, and the cut is marked.
- `a_symlink_from_a_user_dir_into_the_session_root_is_treated_as_a_project_agent` — the trust
  rule of D-an-agent-symlink-cannot-smuggle-trust. The file is withheld without
  `--trust-project`, and it loads with it.
- `a_bidi_override_cannot_reorder_a_line`, in `crates/rho-skills/src/rejection.rs` — a
  right-to-left override, a bidi isolate, and a line separator never reach the terminal.

What a line may hold, and how many, in `crates/rho-skills/tests/agents.rs`:

- `a_hostile_name_is_sanitised_when_the_definition_loads` — two layers, each with its own
  assertion. The loader strips a control character from the name, and `safe_name` bounds a
  16 000-character name to 64.
- `a_definition_bounds_the_number_of_lines_it_owes` — nine warnings yield five lines and one
  count.
- `a_valid_sandbox_value_narrows_the_child` — `sandbox: strict` reaches the definition. This
  change rewrote the invalid path, so the valid one needs a test.

The notice, in `crates/rho-cli/src/subagents.rs`:

- `a_rejected_definition_reaches_the_user_as_a_notice` — the notice list holds the path, the
  reason, and the repair.
- `a_rejection_is_reported_even_when_no_definition_loaded` — the silence this spec repairs.
  Every file is broken, so no tool is registered, and the user is still told why.
- `a_flood_of_rejections_is_capped_and_counted` — six broken files print five lines and one
  count.
- `a_definition_warning_reaches_the_user_too` — a definition that loads with a warning
  reports it, with the definition name.
- `an_untrusted_project_definition_prints_no_warning` — a withheld definition's warning stays
  unprinted, and the same warning prints once the project is trusted.
- `a_flood_of_warnings_is_capped_and_counted` — nine definitions that each warn produce five
  lines and one count.
- `a_hostile_name_cannot_flood_a_notice_line` — eight withheld definitions, each named with
  16 000 characters, and no line runs away.
- `load_reports_a_rejection_when_no_definition_loaded` — the **production** path. `load`
  returns no tool and still returns the reason. A test on the notice builder alone would pass
  even if this return dropped its notices.
- `load_lists_what_the_model_may_spawn` — the available line, which shipped untested.

## 7. Out of scope

- **The skill loader.** `parse_skill_file` returns `SkillFields::Skip { reason }`, and
  `discover` sends the reason to `tracing::warn!` only. So a skill that does not load is
  quiet for any user who did not set `RHO_LOG`. It is the same defect family, and it needs its
  own field on `SkillSet` and its own reporting path in `extensions.rs`. It stays open.
- **A sequence for any other field.** `model`, `sandbox`, and `max_turns` stay scalars.
- **A directory named `x.md`.** `markdown_files` keeps files only, so a directory with that
  name is skipped and never reported.
- **Registering `spawn_agent` with no definition.** A tool that can only refuse still costs
  context in every request. That rule does not change.
- **A name collision between two definitions.** The project definition still overwrites the
  user definition, and that is a separate open item.
- **Repairing the file for the user.** rho reports and continues. It never edits a
  definition.
- **An unknown `sandbox` value.** It warns, and the field falls to `None`, which inherits the
  parent's mode. A child can never widen past its parent, so this cannot escalate. The resume
  path fails closed to `strict` instead, and the two are inconsistent. That is a separate
  decision, and this change only makes the warning visible.
- **A directory named `x.md`.** `markdown_files` keeps files only, so such a directory is
  skipped and never reported.

## 8. What the review changed

The contract went through review before any code, per AGENTS.md step 3. Four changes came
back, and the first two were blocking.

1. **The sanitise rule was prose, not a type.** The detail fields were plain `String`, so the
   200-character cap and the control-character strip lived in a constructor that a caller
   could skip. That is the shape of `ToolKind::Other`. The detail is now the `Detail` newtype,
   and its constructor is private.
2. **An untrusted project file could put its own prose on a start-up line.** A parsed but
   withheld project file shows only its sanitised name today. A rejected one would have shown
   up to 200 characters of repository text, five times over. Discovery now drops the detail
   for an untrusted project file.
3. **An unclosed frontmatter block got the wrong repair.** It read as `NoFrontmatter`, whose
   repair asks for a description that the file already holds. It has its own variant now.
4. **A wrong-typed scalar and a repeated key had no stated behaviour.** Section 4 now states
   both, and section 6 names a test for each.

## 9. What driving it for real changed

Step 11 ran the fixed binary and the binary from the commit before it, over the same
directories. Two more silent failures came out of that, and both are fixed here.

1. **`tools: 5` used to load.** `serde_yaml` reads a plain scalar into a `String`, so the old
   loader took `"5"` as a tool name. The intersection dropped it, and the child ran with an
   empty tool set. A live child was asked to name its tools and answered `NONE`. So the
   original defect had a twin that no test and no reading found.
2. **A definition warning printed nowhere.** `AgentDefinition::warnings` held the dropped
   tool keyword, the bad name, and the bad sandbox value, and no code read the field. A skill
   warning has printed since sprint 2. So `tools: all, read` narrowed a child in silence.
   Section 5 now states the rule, and `a_definition_warning_reaches_the_user_too` holds it.

See `docs/verification/agent-definition-rejection.md` for the commands and the output.

## 10. What the second review changed

The implementation went through two reviews after it was green: one for correctness and one
for security. Three findings were blocking, and each one is now a test and a live probe. See
`docs/verification/agent-definition-rejection.md` section 9.

1. **A file name forged a notice line.** `notice` printed the path verbatim. A file called
   `x\nrho: 3 project definitions are trusted.md`, in an untrusted repository, produced two
   lines and the second read as though rho wrote it. The path is sanitised now.
2. **The new warning line carried raw file text.** The warning loop was the only new path that
   the `Detail` rule did not cover, so `tools: [all, "read\e[2J"]` sent a clear-screen escape
   to the terminal on the `rho run` path, and a 400-character `sandbox` value printed whole.
   Every warning interpolation goes through `Detail` now, and a withheld definition prints no
   warning at all.
3. **A symlink smuggled a repository definition into the trusted set.** The skill loader
   resolves a path before it classifies it, and the agent loader did not. A live probe loaded a
   repository definition with no `--trust-project`. Both loaders call one `is_inside` now, and
   one `admit` function decides trust for every pass. See
   D-an-agent-symlink-cannot-smuggle-trust.

`sanitize` also grew: it replaces a bidirectional override, a bidi isolate, a line separator,
and a byte order mark. A control character was never the only way to disguise a line.

## 11. What the review fleet found

Four reviewer lenses and `codex review` ran over the committed change. Three findings were
real, and each one is now a test and a live probe. See
`docs/verification/agent-definition-rejection.md` section 10.

1. **The cap protected one kind of line out of four.** The rejection lines stopped at five,
   and the withheld name list, the warning lines, and the available list did not. A name is
   never truncated either: it warns above 64 characters and then prints whole. A live probe
   with fifty files, each named with 16 000 characters, printed **800 KB** on one line from an
   untrusted repository, and 1.6 MB over 104 lines when trusted. `MAX_LINES_PER_KIND`,
   `safe_name`, and `summarise_names` now bound every kind, in count and in length.
2. **The production path had no test.** `a_rejection_is_reported_even_when_no_definition_loaded`
   called the notice builder, not `load`, and `load` owns the early return that this whole spec
   exists to protect. The return could have dropped its notices and the test would have passed.
   `load` builds one `Subagents` value now, `LoadRequest` carries the `AgentConfig` so a test
   can isolate discovery from the machine's own home directory, and two tests drive `load`
   itself.
3. **A test of mine passed against the bug it was written for.** The first
   `a_definition_bounds_the_number_of_lines_it_owes` used a file fixture that raises three
   warnings, and the cap is five, so removing the cap changed nothing. The mutation proof
   caught it. The test now builds a definition with nine warnings.

The test lens also found three weak assertions, and all three are repaired. A notice test
pinned a literal fragment of a repair string, so a reworded repair would have failed it; it
asserts `reason.repair()` now. A warning test pinned the word "keyword"; it asserts that
**every** warning the loader raised reaches the user. And `every_rejection_reason_states_a_repair`
was circular, because `notice` is built from the text it asserted; `every_reason_explains_its_own_subject`
ties each reason to its own subject instead.

Two tests are Unix only, and they carry `#[cfg(unix)]` now: one makes a symlink, and one puts
a line break in a file name. Neither is legal on Windows, and without the gate a Windows build
would fail to compile rather than skip them.

Recorded and not fixed here, each with its reason:

- **`is_inside` returns false when a path or root will not resolve**, which reads as "not
  inside" and therefore trusted. The direction is wrong for a trust boundary. It needs write
  access to the user's own `~/.rho/agents` to matter, and failing closed would withhold every
  user definition on an unrelated filesystem error. In production the session root is the
  working directory or `--root`, and both resolve.
- **A trusted project definition still overwrites a user definition of the same name**, with
  no warning, while the skill loader keeps the first and warns. That is a name-resolution
  change, not a failure-path change.
- **`markdown_files` reads every `.md` in a directory with no count cap.** Throughput only:
  each read is bounded to 16 KiB and a fifo or a device file is skipped.
- **An unknown `sandbox` value inherits the parent's mode**, where the resume path fails
  closed to `strict`. A child can never widen past its parent, so this cannot escalate.
- **`Eq` on the three new types is never exercised**, and `Display for Detail` is asserted
  only through `notice`.
- **`Unreadable` from a denied permission has live evidence only.** The unit tests cover the
  missing-file path. A `chmod 000` test would pass as an ordinary user and fail as root,
  because root reads the file anyway, and a test that depends on who runs it is worse than
  none.
- **The same start-up twice has live evidence only.** Discovery holds no state between runs
  and `markdown_files` sorts, so a unit test would pin no invariant that code could break.
- **The terminal frontend takes the same notice list and no test or live run covers that
  hop.**
- **`notices_for` assembles four kinds of line inside `rho-cli`, so a new kind edits one
  function.** The correctness lens called that an SRP violation, and it is one. The pieces it
  assembles already live in `rho-skills`, so the move would be small. It waits for a second
  consumer, because no crate may depend on `rho-cli` and no other frontend renders this report
  yet. Moving it now would design an interface for one caller, and AGENTS.md prefers the
  smaller interface. When the terminal interface renders these lines itself, `AgentSet` grows
  one method and `notices_for` becomes its caller.
