# SPEC-definition-rejection — A definition file that does not load

Status: draft. It becomes delivered when the tests in section 6 exist.
Owning crates: `rho-skills` for the loader and the reason set, `rho-cli` for the notice.
Features: F-agent-definitions and F-tool-list-keywords. See `docs/features.md`.

Decisions: D-a-rejected-definition-is-reported and D-a-tool-list-accepts-a-yaml-sequence.

Reviewed once before any code. The review changed the contract in four places. See section 8.

## 1. The defect this spec repairs

`load_definition` returned `Option<AgentDefinition>`. Every failure was one `None`, so the
reason died at the call site. `discover_agents` then dropped the file, and `AgentSet` had no
place to record it.

The visible result was worse than a lost file. rho registers `spawn_agent` only when at least
one definition loaded. So one bad line in one file removed five tools from the session, and
the model told the user it had no such tool. Two live runs were spent finding it. See
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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

`AgentSet` is built by `discover_agents` and by `Default`, never by a literal outside this
crate, so the new field breaks no construction inside the workspace. A reader that ignores
`rejected` loses the report. The changed `load_definition` is what stops that being quiet,
because a `Result` cannot be dropped without a compiler warning.

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

### Every other field

`model` and `sandbox` stay strings. `max_turns` stays a number. A wrong type in any of them
rejects the file with `BadFrontmatter`, and the detail names the field. A repeated key rejects
the file the same way, because `serde_yaml` refuses a duplicate field. An unknown key is still
ignored, and that rule does not change.

## 5. The notice

`rho-cli` builds one line per rejected file, from `RejectedDefinition::notice`. The line names
the path, the reason, and the repair. The lines print before the session starts, with the
other start-up notices.

Two rules:

- A rejection prints even when no definition loaded. That case removed the tool, so it is the
  case that most needs the report.
- At most five lines print. The rest become one counted line, so a directory of broken files
  cannot push the real output away.

## 6. Test cases

The tool list, in `crates/rho-skills/tests/agents.rs`:

- `a_yaml_sequence_tool_list_loads_the_same_as_a_comma_list` — the defect, reproduced. A file
  with `tools: [read, list]` loads, and it holds the two names.
- `a_block_sequence_tool_list_loads` — the indented `- name` form loads too.
- `a_keyword_in_a_sequence_still_stands_alone` — `[all]` inherits, and `[all, read]` keeps
  `read` and warns.
- `a_tools_field_that_is_not_a_list_is_rejected_and_says_so` — `tools: 5` rejects with
  `BadToolsField`, and the notice names the file.
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
  closes it rejects with `UnclosedFrontmatter`. Its repair names the closing line and the
  bounded read. It never asks the user to add a description they already wrote.
- `a_wrong_type_in_a_scalar_field_is_rejected_with_the_field_name` — `max_turns: "12"` rejects
  with `BadFrontmatter`, and the detail names `max_turns`.
- `a_repeated_key_is_rejected` — two `tools` lines reject with `BadFrontmatter`.
- `frontmatter_that_is_not_a_mapping_is_rejected` — a bare YAML list rejects with
  `BadFrontmatter`.
- `a_missing_file_is_rejected_as_unreadable` — `Unreadable`, and the path is the missing one.
- `every_rejection_reason_states_a_repair` — every variant returns a non-empty repair, and
  `notice` holds the path, the reason, and the repair.
- `a_rejection_detail_is_sanitised_and_bounded` — a control character in the file cannot reach
  the terminal through a detail, and a long detail is capped at 200 characters.

Discovery, in `crates/rho-skills/tests/agents.rs`:

- `a_broken_definition_is_reported_and_the_good_one_still_loads` — one bad file and one good
  file in one directory. The good one loads, and the bad one is in `rejected`.
- `a_broken_project_definition_is_rejected_and_not_hidden` — an untrusted project file that
  does not parse lands in `rejected` with `SkillOrigin::Project`.
- `an_untrusted_project_file_quotes_nothing_in_its_rejection` — the security rule. The
  detail is dropped for an untrusted project file, and the path, the reason, and the repair
  stay. A trusted project file keeps its detail.

The notice, in `crates/rho-cli/src/subagents.rs`:

- `a_rejected_definition_reaches_the_user_as_a_notice` — the notice list holds the path, the
  reason, and the repair.
- `a_rejection_is_reported_even_when_no_definition_loaded` — the silence this spec repairs.
  Every file is broken, so no tool is registered, and the user is still told why.
- `a_flood_of_rejections_is_capped_and_counted` — six broken files print five lines and one
  count.

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
