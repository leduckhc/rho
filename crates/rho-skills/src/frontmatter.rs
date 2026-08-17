//! Read and validate skill frontmatter, without reading a whole body.
//!
//! `discover` calls this for every candidate file. The read is bounded, so a
//! large body never enters memory. Only the frontmatter is parsed.

use serde::Deserialize;
use std::path::Path;
use tokio::io::AsyncReadExt;

/// The most bytes to read from a skill file for frontmatter.
///
/// The frontmatter is small by design. This cap stops a huge or hostile file
/// from filling memory during discovery. A file whose frontmatter does not close
/// inside this many bytes does not load.
const MAX_FRONTMATTER_BYTES: u64 = 16 * 1024;

/// The most characters allowed in a name.
const MAX_NAME_LENGTH: usize = 64;

/// The most characters allowed in a description.
const MAX_DESCRIPTION_LENGTH: usize = 1024;

/// The glyph that replaces a stripped control character.
const REPLACEMENT: char = '\u{fffd}';

/// The validated result of reading a skill file.
pub(crate) enum SkillFields {
    /// The file is a valid skill and may load.
    Load {
        name: String,
        description: String,
        model_invocation_disabled: bool,
        warnings: Vec<String>,
    },
    /// The file does not load. The reason is for the user, not the model.
    Skip { reason: String },
}

/// The raw frontmatter, before validation. Unknown fields are ignored.
#[derive(Debug, Deserialize)]
struct RawFrontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(rename = "allowed-tools")]
    allowed_tools: Option<serde_yaml::Value>,
    #[serde(rename = "disable-model-invocation")]
    disable_model_invocation: Option<bool>,
}

/// Read the frontmatter of `path` and validate it.
///
/// `fallback_name` is the skill directory name. It replaces a missing name, so a
/// skill without a name still loads with a warning.
pub(crate) async fn parse_skill_file(path: &Path, fallback_name: &str) -> SkillFields {
    let text = match read_bounded(path).await {
        Ok(text) => text,
        Err(error) => {
            return SkillFields::Skip {
                reason: format!("cannot read {}: {error}.", path.display()),
            };
        }
    };

    let Some(yaml) = extract_frontmatter(&text) else {
        return SkillFields::Skip {
            reason: format!(
                "the skill file {} has no frontmatter. Add a name and a description.",
                path.display()
            ),
        };
    };

    let raw: RawFrontmatter = match serde_yaml::from_str(&yaml) {
        Ok(raw) => raw,
        Err(error) => {
            return SkillFields::Skip {
                reason: format!(
                    "the frontmatter in {} is not valid YAML: {error}.",
                    path.display()
                ),
            };
        }
    };

    validate(raw, fallback_name)
}

/// Validate raw frontmatter into a load decision.
///
/// The one hard failure is a missing description. The description is the only
/// text the model ever sees, so a skill without one can never be chosen.
fn validate(raw: RawFrontmatter, fallback_name: &str) -> SkillFields {
    let description = match raw.description {
        Some(description) if !description.trim().is_empty() => description,
        _ => {
            return SkillFields::Skip {
                reason: "the skill has no description. A description is required, because it is \
                         the only text the model sees."
                    .to_string(),
            };
        }
    };

    let mut warnings = Vec::new();

    // Sanitise the name, then validate it. A sanitised control character shows
    // as the replacement glyph, which the name check then reports as invalid.
    let name = match raw.name {
        Some(name) => sanitize(&name),
        None => {
            warnings.push(format!(
                "the skill has no name. The directory name \"{fallback_name}\" is used instead."
            ));
            sanitize(fallback_name)
        }
    };
    warnings.extend(name_warnings(&name));

    // Sanitise the description, then bound its length.
    let mut description = sanitize(&description);
    if description.chars().count() > MAX_DESCRIPTION_LENGTH {
        warnings.push(format!(
            "the description is longer than {MAX_DESCRIPTION_LENGTH} characters. It is truncated."
        ));
        description = description.chars().take(MAX_DESCRIPTION_LENGTH).collect();
    }

    if raw.allowed_tools.is_some() {
        warnings.push(
            "the frontmatter sets allowed-tools. rho ignores this field. The approval policy \
             stays the only authority."
                .to_string(),
        );
    }

    SkillFields::Load {
        name,
        description,
        model_invocation_disabled: raw.disable_model_invocation.unwrap_or(false),
        warnings,
    }
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
    if name.starts_with('-') || name.ends_with('-') {
        warnings.push("the name must not start or end with a hyphen.".to_string());
    }
    if name.contains("--") {
        warnings.push("the name must not contain two hyphens in a row.".to_string());
    }
    warnings
}

/// Make an untrusted string safe to draw. Replace every control character with
/// the replacement glyph. This mirrors the tool-output rule, because a name and
/// a description reach the terminal.
fn sanitize(input: &str) -> String {
    input
        .chars()
        .map(|c| if is_unsafe(c) { REPLACEMENT } else { c })
        .collect()
}

/// True when a character must not reach the terminal. A space is safe.
fn is_unsafe(c: char) -> bool {
    if c == ' ' {
        return false;
    }
    c.is_control() || c == '\u{7f}' || ('\u{80}'..='\u{9f}').contains(&c)
}

/// Read at most `MAX_FRONTMATTER_BYTES` from a file, as lossy UTF-8.
async fn read_bounded(path: &Path) -> std::io::Result<String> {
    let file = tokio::fs::File::open(path).await?;
    let mut reader = file.take(MAX_FRONTMATTER_BYTES);
    let mut buffer = Vec::new();
    reader.read_to_end(&mut buffer).await?;
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

/// Return the YAML text between the opening and closing `---` fences.
///
/// The file must start with a `---` line. The function returns `None` when there
/// is no frontmatter, or when the closing fence is not inside the bounded read.
fn extract_frontmatter(text: &str) -> Option<String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    let mut yaml = String::new();
    for line in lines {
        if line.trim_end() == "---" {
            return Some(yaml);
        }
        yaml.push_str(line);
        yaml.push('\n');
    }
    None
}
