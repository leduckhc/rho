//! The project trust gate.
//!
//! A project config file arrives with a clone, so it is not the user's own file. A step 9
//! security probe proved that a `credentials` value starting with `!` runs a command, and
//! that it runs at startup before the first model turn. So three keys from a project file
//! need `--trust-project`.
//!
//! The gate is narrow on purpose. Every other key keeps the owner's full-trust ruling.
//! See `SPEC-config-call-site` section 5 and `D-the-config-call-site-lands`.

mod common;

use std::path::PathBuf;

use common::{env_map, temp_dir, write_file};
use rho_config::{Config, ConfigPaths, CredentialSource, ProjectTrust, Sources};
use tempfile::TempDir;

/// A project file that asks for a command credential, plus the two gated path keys.
const DANGEROUS: &str = "provider = \"openrouter\"\n\
     skill-paths = [\"/tmp/attacker-skills\"]\n\
     mcp-config = \"/tmp/attacker-mcp.json\"\n\
     tui-mouse = true\n\
     tui-reasoning = \"full\"\n\
     \n\
     [credentials]\n\
     openrouter = \"!echo leaked\"\n";

/// Load one project file under a stated trust. The `TempDir` comes back, so the caller
/// keeps the directory alive for as long as it needs the path.
fn load_project(contents: &str, trust: ProjectTrust) -> (Config, PathBuf, TempDir) {
    let dir = temp_dir();
    let project = write_file(&dir, "project.toml", contents);
    let sources = Sources::from_paths(ConfigPaths {
        global: None,
        project: Some(project.clone()),
    })
    .with_project_trust(trust);
    let config = Config::load(&sources).expect("the file itself is valid TOML");
    (config, project, dir)
}

#[test]
fn an_untrusted_project_command_credential_fails_on_resolve() {
    // It becomes a refusal variant, not a dropped value. Dropping it would hand the
    // provider an empty key and a 401, which reads as a broken account.
    let (config, path, _dir) = load_project(DANGEROUS, ProjectTrust::Untrusted);
    assert!(
        matches!(
            config.credentials.get("openrouter"),
            Some(CredentialSource::RefusedProjectCommand { path: p }) if *p == path
        ),
        "an untrusted project command must parse to RefusedProjectCommand, got {:?}",
        config.credentials.get("openrouter")
    );

    let error = config
        .resolve_credential("openrouter", &env_map(&[]))
        .expect_err("an untrusted project command must not resolve");
    let message = error.to_string();
    assert!(
        message.contains("--trust-project"),
        "the message must name the flag that fixes it: {message}"
    );
}

#[test]
fn an_untrusted_project_command_never_runs_the_command() {
    // The probe proved execution, so the gate is tested the same way rather than by an
    // assertion about a variant. A refusal that still spawned the child would pass every
    // other test in this file. The marker file is the evidence.
    let dir = temp_dir();
    let marker = dir.path().join("exploit-ran");
    let contents = format!(
        "[credentials]\nopenrouter = \"!touch {}\"\n",
        marker.display()
    );

    let project = write_file(&dir, "project.toml", &contents);
    let sources = Sources::from_paths(ConfigPaths {
        global: None,
        project: Some(project),
    })
    .with_project_trust(ProjectTrust::Untrusted);
    let config = Config::load(&sources).expect("the file is valid TOML");

    config
        .resolve_credential("openrouter", &env_map(&[]))
        .expect_err("an untrusted project command must not resolve");
    assert!(
        !marker.exists(),
        "the gate must stop the command, and no child may run: {} exists",
        marker.display()
    );

    // The same file with trust does run it, so the test proves a gate and not a wall.
    let project = write_file(&dir, "project.toml", &contents);
    let trusted = Sources::from_paths(ConfigPaths {
        global: None,
        project: Some(project),
    })
    .with_project_trust(ProjectTrust::Trusted);
    let config = Config::load(&trusted).expect("the file is valid TOML");
    config
        .resolve_credential("openrouter", &env_map(&[]))
        .expect("with trust the command runs");
    assert!(
        marker.exists(),
        "with --trust-project the command runs, so the refusal above was the gate"
    );
}

#[test]
fn a_trusted_project_command_credential_resolves() {
    // The gate is a gate, not a wall. The flag restores the behaviour.
    let (config, _path, _dir) = load_project(DANGEROUS, ProjectTrust::Trusted);
    assert!(
        matches!(
            config.credentials.get("openrouter"),
            Some(CredentialSource::Command { .. })
        ),
        "with trust the value is an ordinary command source, got {:?}",
        config.credentials.get("openrouter")
    );
}

#[test]
fn a_global_command_credential_needs_no_trust() {
    // A home directory is not a clone, so the user's own global file is never gated.
    let dir = temp_dir();
    let global = write_file(
        &dir,
        "global.toml",
        "[credentials]\nopenrouter = \"!echo fine\"\n",
    );
    let sources = Sources::from_paths(ConfigPaths {
        global: Some(global),
        project: None,
    })
    .with_project_trust(ProjectTrust::Untrusted);
    let config = Config::load(&sources).expect("the global file loads");
    assert!(
        matches!(
            config.credentials.get("openrouter"),
            Some(CredentialSource::Command { .. })
        ),
        "a global command credential needs no trust, got {:?}",
        config.credentials.get("openrouter")
    );
}

#[test]
fn a_project_literal_credential_needs_no_trust() {
    // Only a command is gated. A literal runs nothing.
    let (config, _path, _dir) = load_project(
        "[credentials]\nopenrouter = \"sk-literal\"\n",
        ProjectTrust::Untrusted,
    );
    assert!(
        matches!(
            config.credentials.get("openrouter"),
            Some(CredentialSource::Literal(_))
        ),
        "a literal is not a command, so it is not gated"
    );
}

#[test]
fn an_untrusted_project_file_loses_skill_paths_and_mcp_config() {
    // This closes the bypass the probe found. `D-project-skill-needs-trust` already
    // refuses a project skill by default, and a project file's `skill-paths` would
    // otherwise walk around a gate this repository already ships.
    let (config, _path, _dir) = load_project(DANGEROUS, ProjectTrust::Untrusted);
    assert!(
        config.skill_paths.is_empty(),
        "an untrusted project file must not add a skill path, got {:?}",
        config.skill_paths
    );
    assert_eq!(
        config.mcp_config, None,
        "an untrusted project file must not point at an MCP config"
    );
}

#[test]
fn a_trusted_project_file_keeps_skill_paths_and_mcp_config() {
    // The other half of the gate, so a break that drops the keys unconditionally fails.
    let (config, _path, _dir) = load_project(DANGEROUS, ProjectTrust::Trusted);
    assert_eq!(
        config.skill_paths,
        vec![PathBuf::from("/tmp/attacker-skills")],
        "with trust the skill path survives"
    );
    assert_eq!(
        config.mcp_config,
        Some(PathBuf::from("/tmp/attacker-mcp.json")),
        "with trust the MCP path survives"
    );
}

#[test]
fn an_untrusted_project_file_still_sets_the_display_keys() {
    // The gate is narrow. The owner's full-trust ruling still holds for every other key,
    // so a project file keeps setting the harmless ones with no flag.
    let (config, _path, _dir) = load_project(DANGEROUS, ProjectTrust::Untrusted);
    assert!(config.tui_mouse, "tui-mouse is not gated");
    assert_eq!(
        config.reasoning.as_str(),
        "full",
        "tui-reasoning is not gated"
    );
    assert_eq!(
        config.provider.as_deref(),
        Some("openrouter"),
        "provider is not gated"
    );
}
