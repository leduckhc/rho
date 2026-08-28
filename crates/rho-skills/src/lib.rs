//! Filesystem skills for rho.
//!
//! A skill is a directory with a `SKILL.md` file. The file carries YAML
//! frontmatter with a name and a description, then free-form instructions. Only
//! the name and the description reach the system prompt. The body loads on
//! demand, with the ordinary `read` tool.
//!
//! See `docs/specs/20260817-215003-SPEC-skills.md` for the contract.

mod agent;
mod discover;
mod error;
mod frontmatter;
mod prompt;
mod rejection;
mod types;

pub use agent::{
    AgentConfig, AgentDefinition, AgentSet, discover_agents, load_agent_body, load_definition,
};
pub use discover::discover;
pub use error::SkillError;
pub use prompt::prompt_block;
pub use rejection::{Detail, MAX_LINES_PER_KIND, RejectedDefinition, RejectionReason};
pub use types::{Skill, SkillConfig, SkillOrigin, SkillSet};

/// Read one skill's full body, on demand.
///
/// The body is the text after the frontmatter. This is the only place a whole
/// skill file is read.
pub async fn load_body(skill: &Skill) -> Result<String, SkillError> {
    let text = tokio::fs::read_to_string(&skill.path)
        .await
        .map_err(|source| SkillError::Io {
            path: skill.path.clone(),
            source,
        })?;
    Ok(strip_frontmatter(&text))
}

/// Return the text after the closing frontmatter fence.
///
/// When the file has no frontmatter, the whole text is the body.
fn strip_frontmatter(text: &str) -> String {
    let stripped = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rest = match stripped.strip_prefix("---") {
        Some(rest) if rest.starts_with('\n') || rest.starts_with("\r\n") => rest,
        _ => return text.to_string(),
    };
    // Skip the opening line break.
    rest = rest
        .strip_prefix('\n')
        .or_else(|| rest.strip_prefix("\r\n"))
        .unwrap_or(rest);

    // Find the closing fence at the start of a line.
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            let after = offset + line.len();
            return rest[after..].trim_start_matches(['\n', '\r']).to_string();
        }
        offset += line.len();
    }
    // No closing fence. Treat the whole file as the body.
    text.to_string()
}
