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
use serde::Deserialize;

use crate::frontmatter::{extract_frontmatter, read_bounded, sanitize};
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
#[derive(Clone, Debug, Default)]
pub struct AgentSet {
    /// Definitions that may be used now.
    pub loaded: Vec<AgentDefinition>,
    /// Project definitions found but withheld, because the project is not
    /// trusted.
    pub withheld: Vec<AgentDefinition>,
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

    for dir in &config.user_dirs {
        for path in markdown_files(dir) {
            if let Some(def) = load_definition(&path, SkillOrigin::User).await {
                set.loaded.push(def);
            }
        }
    }

    if let Some(root) = &config.session_root {
        let project_dirs = [
            root.join(".rho").join("agents"),
            root.join(".agents").join("agents"),
        ];
        for dir in project_dirs {
            for path in markdown_files(&dir) {
                if let Some(def) = load_definition(&path, SkillOrigin::Project).await {
                    if config.project_trusted {
                        set.loaded.push(def);
                    } else {
                        set.withheld.push(def);
                    }
                }
            }
        }
    }

    set
}

/// Load one definition from a path. Return `None` when it does not load.
pub async fn load_definition(path: &Path, origin: SkillOrigin) -> Option<AgentDefinition> {
    let fallback_name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();

    let text = read_bounded(path).await.ok()?;
    let yaml = extract_frontmatter(&text)?;
    let raw: RawFrontmatter = serde_yaml::from_str(&yaml).ok()?;

    // A missing description does not load. The description is the only text the
    // model sees, so an agent without one can never be chosen.
    let description = match raw.description {
        Some(description) if !description.trim().is_empty() => sanitize(&description),
        _ => return None,
    };

    let mut warnings = Vec::new();
    let name = match raw.name {
        Some(name) => sanitize(&name),
        None => {
            warnings.push(format!(
                "the agent has no name. The file name \"{fallback_name}\" is used instead."
            ));
            sanitize(&fallback_name)
        }
    };
    warnings.extend(name_warnings(&name));

    let tools = raw.tools.map(|tools| parse_tool_list(&tools));

    let sandbox = match raw.sandbox {
        Some(text) => match text.parse::<SandboxMode>() {
            Ok(mode) => Some(mode),
            Err(error) => {
                warnings.push(error);
                None
            }
        },
        None => None,
    };

    Some(AgentDefinition {
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

/// The raw frontmatter, before validation. Unknown fields are ignored.
#[derive(Debug, Deserialize)]
struct RawFrontmatter {
    name: Option<String>,
    description: Option<String>,
    tools: Option<String>,
    model: Option<String>,
    max_turns: Option<u32>,
    sandbox: Option<String>,
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
fn markdown_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            files.push(path);
        }
    }
    files.sort();
    files
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
