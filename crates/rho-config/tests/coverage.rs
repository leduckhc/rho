//! Coverage for public items that no existing test reaches.
//!
//! Each test drives a public surface named in `SPEC-config` that had no in-tree caller:
//! `Config::resolve_credential`, `SubagentLimitsLayer` with the `subagents` field on
//! `ConfigLayer`, the `no-skills` key inverting into `discover_skills`, and the
//! resolved default `sandbox` and `approval` after a full `Config::load`. Every test
//! routes through a `todo!()` body, so it fails on the unimplemented body, not a type
//! error, and every file lives under a `tempfile::TempDir`.

mod common;

use std::collections::BTreeMap;

use common::{empty_sources, env_map, sources_with_project_file, temp_dir, write_file};
use rho_config::{ApprovalMode, Config, CredentialSource, Sources};
use rho_core::{SandboxMode, Secret, SubagentLimits};

/// A `Config` that holds one named credential, and stated defaults everywhere else.
fn config_with_credentials(credentials: BTreeMap<String, CredentialSource>) -> Config {
    Config {
        provider: Some("openrouter".to_string()),
        model: Some("some-model".to_string()),
        session_root: None,
        session_file: None,
        ephemeral: false,
        sandbox: SandboxMode::Off,
        approval: Some(ApprovalMode::ReadOnly),
        skill_paths: Vec::new(),
        discover_skills: true,
        discover_agents: true,
        base_url: None,
        tui_motion: true,
        tui_mouse: false,
        reasoning: rho_core::ReasoningDisplay::Summary,
        reasoning_effort: None,
        mcp_config: None,
        subagents: SubagentLimits::default(),
        credentials,
    }
}

#[test]
fn resolve_credential_resolves_a_named_source() {
    // `Config::resolve_credential` looks up a source by name and resolves it. It had
    // no test and no in-tree caller.
    let mut credentials = BTreeMap::new();
    credentials.insert(
        "api".to_string(),
        CredentialSource::Literal(Secret::new("sk-live-named")),
    );
    let config = config_with_credentials(credentials);
    let env = env_map(&[]);
    let secret = config
        .resolve_credential("api", &env)
        .expect("the named credential resolves");
    assert_eq!(secret.expose(), "sk-live-named");
}

#[test]
fn read_file_parses_the_subagents_table() {
    // `SubagentLimitsLayer` and the `subagents` field on `ConfigLayer` had no
    // constructor and no reader. A `[subagents]` table exercises both.
    let dir = temp_dir();
    let path = write_file(
        &dir,
        "config.toml",
        "[subagents]\n\
         max-depth = 3\n\
         max-children-per-parent = 4\n\
         max-live-total = 5\n\
         child-timeout-secs = 30\n",
    );
    let layer = Config::read_file(&path)
        .expect("a present file is not an error")
        .expect("a present file is Some");
    let subagents = layer
        .subagents
        .expect("the subagents table is read into the layer");
    assert_eq!(subagents.max_depth, Some(3));
    assert_eq!(subagents.max_children_per_parent, Some(4));
    assert_eq!(subagents.max_live_total, Some(5));
    assert_eq!(subagents.child_timeout_secs, Some(30));
}

#[test]
fn no_skills_true_disables_discovery() {
    // `no-skills = true` inverts into `discover_skills = false`. A double negative is
    // easy to get backwards, so both directions are asserted.
    let dir = temp_dir();
    let path = write_file(&dir, "config.toml", "no-skills = true\n");
    let config = Config::load(&sources_with_project_file(path)).expect("a valid file loads");
    assert!(
        !config.discover_skills,
        "no-skills = true must disable skill discovery"
    );
}

#[test]
fn no_skills_false_enables_discovery() {
    // The other direction: `no-skills = false` must leave discovery on.
    let dir = temp_dir();
    let path = write_file(&dir, "config.toml", "no-skills = false\n");
    let config = Config::load(&sources_with_project_file(path)).expect("a valid file loads");
    assert!(
        config.discover_skills,
        "no-skills = false must enable skill discovery"
    );
}

#[test]
fn load_leaves_the_approval_unset_so_the_frontend_resolves_it() {
    // A bare load states no approval mode, so `approval` is `None`. `None` is not a
    // permissive default. It hands the choice to the table in SPEC-approval section 4, which
    // yields `ask` where a human or a client can answer, and `read-only` where nobody
    // can answer. A `Config` that collapsed the unset case into one value would make the
    // `ask` default unreachable. See decision D-approval-option-not-enum.
    let config = Config::load(&Sources::default()).expect("empty sources resolve");
    assert!(
        config.approval.is_none(),
        "a bare load must leave the approval unset, got {:?}",
        config.approval
    );
}

#[test]
fn a_stated_approval_survives_the_load() {
    // A user who states a mode keeps it. So the resolution table can tell an explicit
    // `read-only` from an absent value, which is the whole point of the `Option`.
    let dir = temp_dir();
    let path = write_file(&dir, "config.toml", "approval = \"read-only\"\n");
    let sources = sources_with_project_file(path);
    let config = Config::load(&sources).expect("a valid approval value");
    assert_eq!(
        config.approval,
        Some(ApprovalMode::ReadOnly),
        "a stated approval mode must survive the load"
    );
}

#[test]
fn load_resolves_the_default_security_keys() {
    // The resolved `Config` after a full `Config::load`, not merely the `ConfigLayer`
    // default. The sandbox default is `off` (SPEC-config section 4, D-bash-os-sandbox). The approval
    // default must fail closed: nothing in a bare config load can answer a prompt, so
    // per SPEC-approval section 4 the resolved default is read-only and never allow-all.
    let config = Config::load(&Sources::default()).expect("empty sources resolve");
    assert_eq!(
        config.sandbox,
        SandboxMode::Off,
        "the resolved sandbox default must be off"
    );
    assert!(
        !matches!(config.approval, Some(ApprovalMode::AllowAll)),
        "a bare load must never resolve to allow-all, got {:?}",
        config.approval
    );
}

#[test]
fn no_skills_leaves_agent_discovery_on() {
    // One switch used to govern both loaders, so `--no-skills` removed every subagent in
    // silence. See `D-skills-and-agents-are-two-switches`.
    let config = Config::load(&empty_sources().with_flags(rho_config::ConfigLayer {
        no_skills: Some(true),
        ..Default::default()
    }))
    .expect("flags resolve");
    assert!(!config.discover_skills, "the skill search is off");
    assert!(config.discover_agents, "and delegation is untouched");
}

#[test]
fn no_agents_leaves_skill_discovery_on() {
    let config = Config::load(&empty_sources().with_flags(rho_config::ConfigLayer {
        no_agents: Some(true),
        ..Default::default()
    }))
    .expect("flags resolve");
    assert!(!config.discover_agents, "the agent search is off");
    assert!(config.discover_skills, "and skills still load");
}
