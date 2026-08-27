//! Agent definitions on disk.
//!
//! An agent definition has the same shape as a skill: a markdown file with YAML
//! frontmatter, then free-form instructions. It names the agent, describes when
//! to use it, and may narrow its tools, choose its model, cap its turns, and
//! narrow its sandbox. The loader is shared with the skill loader, because the
//! shape is the same. See `docs/specs/20260818-000223-SPEC-subagents.md` section 5.
//!
//! Discovery follows `SPEC-skills` exactly, including its trust rule. A project
//! agent is withheld until the user trusts the session root. An agent definition
//! is instructions plus a tool list plus a model choice, and it runs unattended,
//! so decision D-project-skill-needs-trust applies with more force here, not less.

use std::path::{Path, PathBuf};

use rho_core::{SandboxMode, ToolIntersection, intersect_tools};
use serde::{Deserialize, Deserializer};

use crate::discover::is_inside;
use crate::frontmatter::{extract_frontmatter, read_bounded, sanitize};
use crate::rejection::{Detail, MAX_LINES_PER_KIND, RejectedDefinition, RejectionReason, bounded};
use crate::types::SkillOrigin;

/// The most characters allowed in a name. The same rule as a skill.
const MAX_NAME_LENGTH: usize = 64;

/// One agent definition loaded from disk. The body is loaded on demand.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentDefinition {
    /// The agent name, from the frontmatter.
    pub name: String,
    /// The description the model reads to choose the agent.
    pub description: String,
    /// The absolute path of the definition file.
    pub path: PathBuf,
    /// Where the definition came from. A project origin is untrusted by default.
    pub origin: SkillOrigin,
    /// The tools the definition asks for. `None` means inherit the parent's set.
    /// A name is intersected with the parent's set at spawn time.
    ///
    /// The frontmatter keywords `all` and `*` resolve to `None`, and `none`
    /// resolves to an empty list. See [`resolve_tool_list`].
    pub tools: Option<Vec<String>>,
    /// The model to use, overriding the inherited model. `None` inherits.
    pub model: Option<String>,
    /// The per-run turn cap the definition asks for. Capped by the parent's
    /// remaining budget at spawn time.
    pub max_turns: Option<u32>,
    /// The sandbox mode the definition asks for. It may only narrow. `None`
    /// inherits the parent's mode.
    pub sandbox: Option<SandboxMode>,
    /// Warnings raised while validating. Shown to the user, never to the model.
    pub warnings: Vec<String>,
}

impl AgentDefinition {
    /// Resolve this definition's tool request against the parent's set.
    ///
    /// A name the parent does not hold is dropped and reported. This is the tool
    /// half of the security core. See `SPEC-subagents` section 3 and decision D-child-confined-by-composition.
    pub fn resolve_tools(&self, parent_tools: &[String]) -> ToolIntersection {
        intersect_tools(parent_tools, self.tools.as_deref())
    }

    /// The name, made safe to draw and short enough to read.
    ///
    /// A name comes from a file, so a repository chooses it. The loader sanitises it,
    /// and this bounds it as well. A live run printed 800 KB of one repository's names
    /// on a start-up line, because a name only warns above 64 characters.
    pub fn safe_name(&self) -> String {
        bounded(&self.name, MAX_NAME_LENGTH)
    }

    /// The lines this definition owes the user, and no more than a bounded number.
    ///
    /// A warning is rho's own sentence with the file's text inside it. `Detail` bounds
    /// that text, this bounds the name, and the cap bounds the count. So one definition
    /// cannot fill a terminal.
    pub fn notices(&self) -> Vec<String> {
        let name = self.safe_name();
        let mut lines: Vec<String> = self
            .warnings
            .iter()
            .take(MAX_LINES_PER_KIND)
            .map(|warning| format!("agent definition {name}: {warning}"))
            .collect();
        let hidden = self.warnings.len().saturating_sub(MAX_LINES_PER_KIND);
        if hidden > 0 {
            lines.push(format!(
                "agent definition {name} raised {hidden} more warning(s), not listed here."
            ));
        }
        lines
    }
}

/// Read one agent definition's body, the text after the frontmatter.
///
/// The body is the child's system prompt. It never reaches the parent's context.
pub async fn load_agent_body(def: &AgentDefinition) -> std::io::Result<String> {
    let text = tokio::fs::read_to_string(&def.path).await?;
    Ok(strip_frontmatter(&text))
}

/// Return the text after the closing frontmatter fence. When the file has no
/// frontmatter, the whole text is the body.
fn strip_frontmatter(text: &str) -> String {
    let stripped = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rest = match stripped.strip_prefix("---") {
        Some(rest) if rest.starts_with('\n') || rest.starts_with("\r\n") => rest,
        _ => return text.to_string(),
    };
    rest = rest
        .strip_prefix('\n')
        .or_else(|| rest.strip_prefix("\r\n"))
        .unwrap_or(rest);
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            let after = offset + line.len();
            return rest[after..].trim_start_matches(['\n', '\r']).to_string();
        }
        offset += line.len();
    }
    text.to_string()
}

/// The set an agent discovery pass found.
///
/// `#[non_exhaustive]`, so no crate outside this one writes the literal. This field set
/// has grown once already, and a struct literal in another crate would have broken on
/// that change. `discover_agents` and `Default` are the two ways to build one.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct AgentSet {
    /// Definitions that may be used now.
    pub loaded: Vec<AgentDefinition>,
    /// Project definitions found but withheld, because the project is not
    /// trusted.
    pub withheld: Vec<AgentDefinition>,
    /// Files that did not load at all, with the reason for each one.
    ///
    /// A rejected file used to vanish. See decision D-a-rejected-definition-is-reported.
    pub rejected: Vec<RejectedDefinition>,
}

/// Where to look for agent definitions, and what to trust.
#[derive(Clone, Debug)]
pub struct AgentConfig {
    /// Trusted user directories. Searched in order.
    pub user_dirs: Vec<PathBuf>,
    /// The session root. Project definitions are found under it.
    pub session_root: Option<PathBuf>,
    /// True when the user has trusted this session root.
    pub project_trusted: bool,
    /// False turns discovery off.
    pub discover: bool,
}

impl AgentConfig {
    /// The default user directories, `~/.rho/agents` and `~/.agents/agents`.
    pub fn with_default_user_dirs(session_root: impl Into<PathBuf>) -> Self {
        let mut user_dirs = Vec::new();
        if let Some(home) = home_dir() {
            user_dirs.push(home.join(".rho").join("agents"));
            user_dirs.push(home.join(".agents").join("agents"));
        }
        Self {
            user_dirs,
            session_root: Some(session_root.into()),
            project_trusted: false,
            discover: true,
        }
    }
}

/// Find every agent definition. Reads only frontmatter, never a whole body.
pub async fn discover_agents(config: &AgentConfig) -> AgentSet {
    let mut set = AgentSet::default();
    if !config.discover {
        return set;
    }

    // Resolve the root once. A user directory may hold a symlink that points inside
    // the session root, and that file is a project file whatever directory found it.
    let canonical_root = config
        .session_root
        .as_deref()
        .and_then(|root| root.canonicalize().ok());

    for dir in &config.user_dirs {
        for path in markdown_files(dir) {
            let origin = if is_inside(&path, canonical_root.as_deref()) {
                SkillOrigin::Project
            } else {
                SkillOrigin::User
            };
            let outcome = load_definition(&path, origin).await;
            admit(&mut set, outcome, config.project_trusted);
        }
    }

    if let Some(root) = &config.session_root {
        let project_dirs = [
            root.join(".rho").join("agents"),
            root.join(".agents").join("agents"),
        ];
        for dir in project_dirs {
            for path in markdown_files(&dir) {
                let outcome = load_definition(&path, SkillOrigin::Project).await;
                admit(&mut set, outcome, config.project_trusted);
            }
        }
    }

    set
}

/// File one load outcome into the set, under the trust rule.
///
/// One place decides trust, so the user pass and the project pass cannot drift apart.
/// The origin comes from the outcome itself, so no caller can pass one that disagrees
/// with the definition it files.
///
/// An untrusted rejection loses its detail, because a repository rho has not been told
/// to trust must not put its own prose on a start-up line.
fn admit(
    set: &mut AgentSet,
    outcome: Result<AgentDefinition, RejectedDefinition>,
    project_trusted: bool,
) {
    let origin = match &outcome {
        Ok(def) => def.origin,
        Err(rejected) => rejected.origin,
    };
    let trusted = matches!(origin, SkillOrigin::User) || project_trusted;
    match outcome {
        Ok(def) => {
            if trusted {
                set.loaded.push(def);
            } else {
                set.withheld.push(def);
            }
        }
        Err(rejected) => set.rejected.push(if trusted {
            rejected
        } else {
            rejected.without_detail()
        }),
    }
}

/// Load one definition from a path.
///
/// The error side carries the reason, so no caller can drop it by accident. It used to
/// be an `Option`, and every failure was one `None`.
pub async fn load_definition(
    path: &Path,
    origin: SkillOrigin,
) -> Result<AgentDefinition, RejectedDefinition> {
    let reject = |reason: RejectionReason| {
        Err(RejectedDefinition {
            path: path.to_path_buf(),
            origin,
            reason,
        })
    };

    let fallback_name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();

    let text = match read_bounded(path).await {
        Ok(text) => text,
        Err(error) => {
            return reject(RejectionReason::Unreadable {
                detail: Detail::new(error.to_string()),
            });
        }
    };
    let Some(yaml) = extract_frontmatter(&text) else {
        // Two different faults, and two different repairs. An unclosed block used to
        // report "no frontmatter", which asks for a description the file already holds.
        return reject(if opens_frontmatter(&text) {
            RejectionReason::UnclosedFrontmatter
        } else {
            RejectionReason::NoFrontmatter
        });
    };
    let raw: RawFrontmatter = match serde_yaml::from_str(&yaml) {
        Ok(raw) => raw,
        Err(error) => {
            return reject(RejectionReason::BadFrontmatter {
                detail: Detail::new(error.to_string()),
            });
        }
    };

    // A missing description does not load. The description is the only text the
    // model sees, so an agent without one can never be chosen.
    let description = match raw.description {
        Some(description) if !description.trim().is_empty() => sanitize(&description),
        _ => return reject(RejectionReason::NoDescription),
    };

    let mut warnings = Vec::new();
    let name = match raw.name {
        Some(name) => sanitize(&name),
        None => {
            // The stem is repository text, so it goes through the bounded type.
            warnings.push(format!(
                "the agent has no name. The file name \"{}\" is used instead.",
                Detail::new(&fallback_name)
            ));
            sanitize(&fallback_name)
        }
    };
    warnings.extend(name_warnings(&name));

    let tools = match raw.tools {
        Some(value) => match tool_tokens(&value) {
            Ok(tokens) => resolve_tool_list(&tokens, &mut warnings),
            // A broken tool list refuses the whole file. The only other reading is to
            // ignore the field, and an ignored field inherits every parent tool.
            Err(detail) => return reject(RejectionReason::BadToolsField { detail }),
        },
        None => None,
    };

    let sandbox = match raw.sandbox {
        Some(text) => match text.parse::<SandboxMode>() {
            Ok(mode) => Some(mode),
            // rho-core's message quotes the whole value, and a value is repository
            // text of any length. So the value goes through the bounded type here, and
            // the repair stays whole.
            Err(_) => {
                warnings.push(format!(
                    "the sandbox mode \"{}\" is unknown. Use off, confined, or strict.",
                    Detail::new(&text)
                ));
                None
            }
        },
        None => None,
    };

    Ok(AgentDefinition {
        name,
        description,
        path: path.to_path_buf(),
        origin,
        tools,
        model: raw.model,
        max_turns: raw.max_turns,
        sandbox,
        warnings,
    })
}

/// True when the text starts a frontmatter block.
///
/// `extract_frontmatter` returns `None` for a file with no opening fence and for a
/// file whose fence never closes. This tells the two apart.
fn opens_frontmatter(text: &str) -> bool {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    text.lines()
        .next()
        .is_some_and(|line| line.trim_end() == "---")
}

/// The raw frontmatter, before validation. Unknown fields are ignored.
#[derive(Debug, Deserialize)]
struct RawFrontmatter {
    name: Option<String>,
    description: Option<String>,
    /// The tool list, in whichever form the author wrote.
    ///
    /// A plain `Option<Value>` reads `tools:` with no value as `None`, which is the
    /// value an absent field gives. That difference matters, because an absent field
    /// inherits every parent tool. So an empty line arrives here as `Some(Null)`.
    #[serde(default, deserialize_with = "present_value")]
    tools: Option<serde_yaml::Value>,
    model: Option<String>,
    max_turns: Option<u32>,
    sandbox: Option<String>,
}

/// Read a present field as `Some`, even when it holds nothing.
fn present_value<'de, D>(deserializer: D) -> Result<Option<serde_yaml::Value>, D::Error>
where
    D: Deserializer<'de>,
{
    serde_yaml::Value::deserialize(deserializer).map(Some)
}

/// Turn the `tools` field into tokens, whichever form the author wrote.
///
/// A string is the comma or space form. A sequence is the ordinary YAML form. Both
/// mean the same thing. See decision D-a-tool-list-accepts-a-yaml-sequence.
fn tool_tokens(value: &serde_yaml::Value) -> Result<Vec<String>, Detail> {
    match value {
        serde_yaml::Value::String(line) => Ok(parse_tool_list(line)),
        serde_yaml::Value::Sequence(items) => {
            let mut names = Vec::new();
            for item in items {
                match item {
                    serde_yaml::Value::String(name) => names.extend(parse_tool_list(name)),
                    other => {
                        return Err(Detail::new(format!(
                            "one item in the list is {}, and a tool name is a word",
                            yaml_kind(other)
                        )));
                    }
                }
            }
            Ok(names)
        }
        serde_yaml::Value::Null => Err(Detail::new(
            "the line holds no value, and an empty line would inherit every parent tool",
        )),
        other => Err(Detail::new(format!(
            "the field is {}, and a tool list is a line of words or a sequence",
            yaml_kind(other)
        ))),
    }
}

/// A plain name for a YAML value, for a message a user reads.
fn yaml_kind(value: &serde_yaml::Value) -> &'static str {
    match value {
        serde_yaml::Value::Null => "empty",
        serde_yaml::Value::Bool(_) => "a true or false value",
        serde_yaml::Value::Number(_) => "a number",
        serde_yaml::Value::String(_) => "a word",
        serde_yaml::Value::Sequence(_) => "a list",
        serde_yaml::Value::Mapping(_) => "a map",
        serde_yaml::Value::Tagged(_) => "a tagged value",
    }
}

/// The keywords that mean "every tool the parent holds".
const KEYWORDS_ALL: [&str; 2] = ["all", "*"];

/// The keyword that means "no tool at all".
const KEYWORD_NONE: &str = "none";

/// Resolve a frontmatter tool list into the field's meaning.
///
/// `None` means inherit the parent's set, which is what an absent field means.
/// `Some(list)` means intersect that list with the parent's set, so an empty list
/// means no tools.
///
/// A keyword must stand alone. `all` and `*` inherit. `none` is an empty set, and
/// no other spelling states that on purpose. A keyword beside a real name is a
/// contradiction, so the keyword is dropped and the names stand. That narrows,
/// and widening on an unclear line is the fail-open shape. See `SPEC-subagents`
/// section 5 and decision D-a-tool-keyword-stands-alone.
fn resolve_tool_list(tokens: &[String], warnings: &mut Vec<String>) -> Option<Vec<String>> {
    let mut names = Vec::new();
    let mut keywords = Vec::new();
    let mut wants_all = false;
    let mut wants_none = false;
    for name in tokens {
        let lower = name.to_ascii_lowercase();
        if KEYWORDS_ALL.contains(&lower.as_str()) {
            wants_all = true;
            keywords.push(name.clone());
        } else if lower == KEYWORD_NONE {
            wants_none = true;
            keywords.push(name.clone());
        } else {
            names.push(name.clone());
        }
    }

    if keywords.is_empty() {
        return Some(names);
    }
    if !names.is_empty() {
        // Both lists hold repository text, so both go through the bounded type.
        warnings.push(format!(
            "a tool keyword must stand alone. \"{}\" was dropped. These named tools stand: {}.",
            Detail::new(keywords.join(", ")),
            Detail::new(names.join(", "))
        ));
        return Some(names);
    }
    if wants_all && wants_none {
        warnings.push(
            "the tool list asks for all and for none. The narrower one wins, so this agent \
             gets no tools."
                .to_string(),
        );
        return Some(Vec::new());
    }
    if wants_none {
        return Some(Vec::new());
    }
    None
}

/// Parse a comma-or-space separated tool list into names.
fn parse_tool_list(raw: &str) -> Vec<String> {
    raw.split([',', ' ', '\n', '\t'])
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

/// Warnings for a name that breaks a rule. A bad name still loads.
fn name_warnings(name: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    let length = name.chars().count();
    if length == 0 || length > MAX_NAME_LENGTH {
        warnings.push(format!(
            "the name must be 1 to {MAX_NAME_LENGTH} characters. This name has {length}."
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        warnings.push("the name must use only lowercase letters, digits, and hyphens.".to_string());
    }
    warnings
}

/// Every `.md` file directly inside `dir`, sorted for a stable order.
///
/// A regular file counts, and so does a **dangling symlink**: a link whose target does
/// not exist. `is_file()` follows a link, so a dangling one used to be skipped here and
/// reached nobody, while an unreadable regular file reached the user as a rejection. That
/// silence is the defect this module exists to end, so the link goes through the loader
/// and fails into `Unreadable` with its own path.
///
/// A link whose target **does** exist and is not a regular file stays out. A symlink to a
/// fifo is the case that matters: opening one can block until a writer appears.
fn markdown_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        if path.is_file() || is_dangling_symlink(&path) {
            files.push(path);
        }
    }
    files.sort();
    files
}

/// True when the path is a symlink and its target does not exist.
fn is_dangling_symlink(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink())
        && !path.exists()
}

/// The user home directory, read from the environment. It never reads the
/// filesystem, so it is safe in a test.
fn home_dir() -> Option<PathBuf> {
    for name in ["HOME", "USERPROFILE"] {
        if let Some(value) = std::env::var_os(name)
            && !value.is_empty()
        {
            return Some(PathBuf::from(value));
        }
    }
    None
}
