//! Public data types for skill discovery.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Where a skill came from, which decides whether it is trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillOrigin {
    /// A user directory, settings entry, or an explicit path.
    User,
    /// The session root or an ancestor. Untrusted until the project is trusted.
    Project,
}

/// One discovered skill. The body is not read yet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Skill {
    /// The skill name, from the frontmatter.
    pub name: String,
    /// The skill description, from the frontmatter.
    pub description: String,
    /// The absolute path of the `SKILL.md` file.
    pub path: PathBuf,
    /// The directory that holds the skill. A relative link inside the body
    /// resolves against this.
    pub root: PathBuf,
    /// Where the skill came from.
    pub origin: SkillOrigin,
    /// True when the skill may not enter the system prompt.
    pub model_invocation_disabled: bool,
    /// Warnings raised while validating. Shown to the user, never to the model.
    pub warnings: Vec<String>,
}

/// What a discovery pass found.
#[derive(Clone, Debug, Default)]
pub struct SkillSet {
    /// Skills that may be used now.
    pub loaded: Vec<Skill>,
    /// Project skills found but withheld, because the project is not trusted.
    pub withheld: Vec<Skill>,
}

/// Where to look, and what to trust.
#[derive(Clone, Debug)]
pub struct SkillConfig {
    /// Trusted user directories. Searched in order.
    pub user_dirs: Vec<PathBuf>,
    /// The session root. Project skills are found under it and its ancestors.
    pub session_root: Option<PathBuf>,
    /// Explicit paths from the command line. Always trusted, always loaded.
    pub explicit: Vec<PathBuf>,
    /// True when the user has trusted this session root.
    pub project_trusted: bool,
    /// False turns discovery off. An explicit path still loads.
    pub discover: bool,
}

impl SkillConfig {
    /// The default user directories, `~/.rho/skills` and `~/.agents/skills`.
    pub fn with_default_user_dirs(session_root: impl Into<PathBuf>) -> Self {
        let mut user_dirs = Vec::new();
        if let Some(home) = home_dir() {
            user_dirs.push(home.join(".rho").join("skills"));
            user_dirs.push(home.join(".agents").join("skills"));
        }
        Self {
            user_dirs,
            session_root: Some(session_root.into()),
            explicit: Vec::new(),
            project_trusted: false,
            discover: true,
        }
    }
}

/// The user home directory, read from the environment.
///
/// The function reads `HOME`, and then `USERPROFILE` for a non-Unix host. It
/// never reads the filesystem, so it is safe in a test.
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
