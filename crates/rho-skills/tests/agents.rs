//! Tests for agent definition loading. See `SPEC-subagents` section 5.

use rho_skills::{AgentConfig, SkillOrigin, discover_agents, load_definition};
use std::path::Path;

/// Write an agent definition file into `dir/name.md`.
fn write_agent(dir: &Path, file: &str, body: &str) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(file);
    std::fs::write(&path, body).unwrap();
    path
}

const SCOUT: &str = "---\n\
name: scout\n\
description: Fast recon. Locates code and returns a compressed summary.\n\
tools: read, glob, grep\n\
model: anthropic/claude-haiku-4.5\n\
max_turns: 12\n\
---\n\
You locate code and report where things are. You do not change files.\n";

#[tokio::test]
async fn an_agent_definition_loads_from_a_user_directory() {
    let dir = tempfile::tempdir().unwrap();
    let user = dir.path().join("home").join(".rho").join("agents");
    write_agent(&user, "scout.md", SCOUT);

    let config = AgentConfig {
        user_dirs: vec![user],
        session_root: None,
        project_trusted: false,
        discover: true,
    };
    let set = discover_agents(&config).await;
    assert_eq!(set.loaded.len(), 1);
    let scout = &set.loaded[0];
    assert_eq!(scout.name, "scout");
    assert_eq!(scout.origin, SkillOrigin::User);
    assert_eq!(
        scout.tools.as_deref(),
        Some(&["read".to_string(), "glob".to_string(), "grep".to_string()][..])
    );
    assert_eq!(scout.model.as_deref(), Some("anthropic/claude-haiku-4.5"));
    assert_eq!(scout.max_turns, Some(12));
}

#[tokio::test]
async fn a_project_agent_definition_is_withheld_until_the_project_is_trusted() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_agent(&root.join(".rho").join("agents"), "scout.md", SCOUT);

    // Untrusted: the project agent is withheld, and still listed.
    let untrusted = AgentConfig {
        user_dirs: Vec::new(),
        session_root: Some(root.to_path_buf()),
        project_trusted: false,
        discover: true,
    };
    let set = discover_agents(&untrusted).await;
    assert!(
        set.loaded.is_empty(),
        "an untrusted project agent must not load"
    );
    assert_eq!(set.withheld.len(), 1, "it is withheld, not hidden");
    assert_eq!(set.withheld[0].origin, SkillOrigin::Project);

    // Trusted: the same agent loads.
    let trusted = AgentConfig {
        project_trusted: true,
        ..untrusted
    };
    let set = discover_agents(&trusted).await;
    assert_eq!(set.loaded.len(), 1);
    assert!(set.withheld.is_empty());
}

#[tokio::test]
async fn a_definition_without_a_description_does_not_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "nodesc.md",
        "---\nname: nodesc\ntools: read\n---\nbody\n",
    );
    let loaded = load_definition(&path, SkillOrigin::User).await;
    assert!(
        loaded.is_none(),
        "a definition without a description does not load"
    );
}

#[tokio::test]
async fn an_unknown_tool_name_in_a_definition_is_dropped_with_a_warning() {
    // The definition asks for `write`, which the parent never had. The
    // intersection drops it and reports the drop. This is the tool half of the
    // security core, applied to a loaded definition.
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "scout.md",
        "---\nname: scout\ndescription: Recon.\ntools: read, write\n---\nbody\n",
    );
    let def = load_definition(&path, SkillOrigin::User)
        .await
        .expect("the definition loads");

    let parent_tools = vec!["read".to_string(), "glob".to_string()];
    let intersection = def.resolve_tools(&parent_tools);
    assert_eq!(intersection.allowed, vec!["read".to_string()]);
    assert_eq!(
        intersection.dropped,
        vec!["write".to_string()],
        "the unknown tool is dropped and reported"
    );
}
