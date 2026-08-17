//! Render the prompt block for a set of skills.

use crate::types::Skill;

/// Render the prompt block for a set of skills, per the Agent Skills standard.
///
/// Only a loaded skill with model invocation enabled appears. The output is
/// byte-identical for the same input, because the block sits in the stable
/// prompt prefix. The skills are sorted by name to make this so.
pub fn prompt_block(skills: &[Skill]) -> String {
    let mut visible: Vec<&Skill> = skills
        .iter()
        .filter(|skill| !skill.model_invocation_disabled)
        .collect();
    if visible.is_empty() {
        return String::new();
    }
    visible.sort_by(|a, b| a.name.cmp(&b.name));

    let mut lines = vec![
        String::new(),
        String::new(),
        "The following skills provide specialized instructions for specific tasks.".to_string(),
        "Use the read tool to load a skill's file when the task matches its description."
            .to_string(),
        "When a skill file references a relative path, resolve it against the skill directory \
         (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands."
            .to_string(),
        String::new(),
        "<available_skills>".to_string(),
    ];

    for skill in visible {
        lines.push("  <skill>".to_string());
        lines.push(format!("    <name>{}</name>", escape_xml(&skill.name)));
        lines.push(format!(
            "    <description>{}</description>",
            escape_xml(&skill.description)
        ));
        lines.push(format!(
            "    <location>{}</location>",
            escape_xml(&skill.path.to_string_lossy())
        ));
        lines.push("  </skill>".to_string());
    }

    lines.push("</available_skills>".to_string());
    lines.join("\n")
}

/// Escape the five XML metacharacters, so a name or description cannot break the
/// block structure.
fn escape_xml(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}
