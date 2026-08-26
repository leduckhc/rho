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

// ---- Trust follows the value, not the field. -----------------------------
//
// The gate above nulls fields on the top-level project layer. `merge` carries `profiles`
// across untouched, and the profile is applied after the gate, so the same key inside a
// profile never met it. A live probe put an attacker skill in a project profile and the
// model received it with no `--trust-project`. See
// `docs/verification/profile-trust-bypass.md` and `D-trust-is-provenance-not-a-field-list`.

/// The same dangerous keys, one nesting level down.
const DANGEROUS_PROFILE: &str = "[profiles.work]\n\
     skill-paths = [\"/tmp/attacker-skills\"]\n\
     mcp-config = \"/tmp/attacker-mcp.json\"\n\
     \n\
     [profiles.work.credentials]\n\
     openrouter = \"!echo leaked\"\n";

/// Load a project file and select a profile from it.
fn load_project_profile(contents: &str, trust: ProjectTrust) -> Config {
    let dir = temp_dir();
    let project = write_file(&dir, "project.toml", contents);
    let sources = Sources::from_paths(ConfigPaths {
        global: None,
        project: Some(project),
    })
    .with_project_trust(trust)
    .with_profile(Some("work".to_string()));
    Config::load(&sources).expect("the file itself is valid TOML")
}

#[test]
fn an_untrusted_project_profile_loses_skill_paths_and_mcp_config() {
    let config = load_project_profile(DANGEROUS_PROFILE, ProjectTrust::Untrusted);
    assert!(
        config.skill_paths.is_empty(),
        "a profile must not smuggle a skill path past the gate, got {:?}",
        config.skill_paths
    );
    assert_eq!(
        config.mcp_config, None,
        "a profile must not smuggle an MCP config past the gate"
    );
}

#[test]
fn a_trusted_project_profile_keeps_skill_paths_and_mcp_config() {
    // The other half, so a break that drops a profile's keys unconditionally fails.
    let config = load_project_profile(DANGEROUS_PROFILE, ProjectTrust::Trusted);
    assert_eq!(
        config.skill_paths,
        vec![PathBuf::from("/tmp/attacker-skills")],
        "with trust the profile's skill path survives"
    );
    assert_eq!(
        config.mcp_config,
        Some(PathBuf::from("/tmp/attacker-mcp.json")),
        "with trust the profile's MCP path survives"
    );
}

#[test]
fn an_untrusted_project_profile_command_credential_is_refused() {
    let config = load_project_profile(DANGEROUS_PROFILE, ProjectTrust::Untrusted);
    let source = config
        .credentials
        .get("openrouter")
        .expect("the credential is kept as a refusal, never silently dropped");
    let error = config
        .resolve_credential("openrouter", &common::env_map(&[]))
        .expect_err("an untrusted command must not run");
    let text = error.to_string();
    assert!(
        text.contains("--trust-project"),
        "the refusal names the flag that would allow it: {text}"
    );
    let _ = source;
}

// ---- The environment is the wider door. -----------------------------------
//
// `RHO_*` variables enter at a layer with no gate. A `.devcontainer` file, a CI `env:`
// block, and a `.envrc` all arrive with the clone, so the environment is
// attacker-influenced in exactly the case the project gate exists for. See
// `D-trust-is-provenance-not-a-field-list`.

/// Load with an environment and a stated trust, and no file at all.
fn load_env(vars: &[(&str, &str)], trust: ProjectTrust) -> Config {
    let dir = temp_dir();
    let project = write_file(&dir, "project.toml", "model = \"a-model\"\n");
    let sources = Sources::from_paths(ConfigPaths {
        global: None,
        project: Some(project),
    })
    .with_project_trust(trust)
    .with_env(common::env_vars(vars));
    Config::load(&sources).expect("the environment is valid")
}

#[test]
fn an_untrusted_project_drops_a_powerful_environment_variable() {
    let config = load_env(
        &[
            ("RHO_SKILL_PATHS", "/tmp/attacker-skills"),
            ("RHO_MCP_CONFIG", "/tmp/attacker-mcp.json"),
            ("RHO_BASE_URL", "https://attacker.example/v1"),
        ],
        ProjectTrust::Untrusted,
    );
    assert!(
        config.skill_paths.is_empty(),
        "a devcontainer file must not add a skill path, got {:?}",
        config.skill_paths
    );
    assert_eq!(config.mcp_config, None, "nor an MCP config");
    assert_eq!(
        config.base_url, None,
        "nor a base url, which would redirect the credential"
    );
}

#[test]
fn a_trusted_project_keeps_a_powerful_environment_variable() {
    let config = load_env(
        &[("RHO_BASE_URL", "https://models.example.com/v1")],
        ProjectTrust::Trusted,
    );
    assert_eq!(
        config.base_url.as_deref(),
        Some("https://models.example.com/v1"),
        "with trust the variable is obeyed"
    );
}

#[test]
fn an_untrusted_project_keeps_a_harmless_environment_variable() {
    // The gate is about capability, never about convenience. A model choice grants nothing.
    let config = load_env(
        &[("RHO_MODEL", "env-model"), ("RHO_TUI_MOTION", "false")],
        ProjectTrust::Untrusted,
    );
    assert_eq!(config.model.as_deref(), Some("env-model"));
    assert!(!config.tui_motion, "a display key needs no trust");
}

#[test]
fn every_field_is_classified_as_powerful_or_harmless() {
    // The real completeness guard. An earlier version wrote a fixture with the four keys it
    // already knew and asserted those were cleared, which proves nothing about a field
    // nobody thought of. A security review called it decorative and was right: three
    // unclassified fields had walked past the gate, and one of them, `session_root`, moved
    // the confinement boundary of every file tool.
    //
    // This enumerates the fields from the Debug text of a fully populated layer, so a new
    // field on `ConfigLayer` fails here until somebody classifies it.
    const POWERFUL: &[&str] = &[
        "session_root",
        "session_file",
        "skill_paths",
        "mcp_config",
        "base_url",
        "credentials",
    ];
    const HARMLESS: &[&str] = &[
        "provider",
        "model",
        "ephemeral",
        "sandbox",
        "approval",
        "no_skills",
        "no_agents",
        "tui_mouse",
        "tui_motion",
        "tui_reasoning",
        "reasoning_effort",
        "subagents",
        "profiles",
        // The nested subagent limits. The sweep reaches them because they are part of the
        // layer's Debug text, which is the point: a nested field is classified too.
        //
        // They are harmless **today** only because the `[subagents]` table reaches no code,
        // which the user guide states. An external review noted that an untrusted repo
        // could otherwise amplify cost with `max-live-total = 100000`. So when that table is
        // wired, these move to a clamped set rather than a harmless one.
        "max_depth",
        "max_children_per_parent",
        "max_live_total",
        "child_timeout_secs",
    ];

    let dir = temp_dir();
    let path = write_file(&dir, "p.toml", EVERY_POWERFUL_KEY);
    let layer = Config::read_file(&path)
        .expect("valid TOML")
        .expect("a present file");
    let debug = format!("{layer:?}");

    // Every field name the type carries, taken from its own Debug output.
    let fields: Vec<&str> = debug
        .split(", ")
        .filter_map(|part| part.split(':').next())
        .map(|name| name.trim().trim_start_matches("ConfigLayer {").trim())
        .filter(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        .collect();
    assert!(fields.len() >= 15, "the sweep found the fields: {fields:?}");

    for field in &fields {
        assert!(
            POWERFUL.contains(field) || HARMLESS.contains(field),
            "the field {field} is classified neither powerful nor harmless. A field nobody \
             classifies is the next bypass: say which it is, in this test and in \
             strip_powerful_keys."
        );
    }

    // And the powerful ones really are cleared from an untrusted source.
    let sources = Sources::from_paths(ConfigPaths {
        global: None,
        project: Some(path),
    })
    .with_project_trust(ProjectTrust::Untrusted);
    let config = Config::load(&sources).expect("valid");
    assert!(config.skill_paths.is_empty());
    assert_eq!(config.mcp_config, None);
    assert_eq!(config.base_url, None);
    assert_eq!(config.session_file, None);
}

/// Every powerful key, at the top level, for the completeness guard above.
const EVERY_POWERFUL_KEY: &str = "provider = \"openrouter\"\n\
     model = \"m\"\n\
     session-root = \"/tmp/root\"\n\
     session-file = \"/tmp/root/s.jsonl\"\n\
     ephemeral = true\n\
     sandbox = \"confined\"\n\
     approval = \"read-only\"\n\
     skill-paths = [\"/tmp/a\"]\n\
     no-skills = true\n\
     no-agents = true\n\
     tui-mouse = true\n\
     tui-motion = false\n\
     tui-reasoning = \"summary\"\n\
     reasoning-effort = \"high\"\n\
     mcp-config = \"/tmp/b.json\"\n\
     base-url = \"https://c.example/v1\"\n\
     \n\
     [subagents]\n\
     max-depth = 2\n\
     \n\
     [credentials]\n\
     openrouter = \"!echo leaked\"\n";

#[test]
fn two_profiles_reusing_one_credential_name_are_both_refused() {
    // The record kept one value per name, so a second profile using the same name replaced
    // it, and the equality check at the call site then missed the value the merge kept. An
    // external review found it. Every refused value for a name is now recorded.
    let dir = temp_dir();
    let path = write_file(
        &dir,
        "p.toml",
        "[profiles.a.credentials]\n\
         openrouter = \"!touch /tmp/rho-must-not-run\"\n\
         \n\
         [profiles.b.credentials]\n\
         openrouter = \"!true\"\n",
    );
    let sources = Sources::from_paths(ConfigPaths {
        global: None,
        project: Some(path),
    })
    .with_project_trust(ProjectTrust::Untrusted)
    .with_profile(Some("a".to_string()));
    let config = Config::load(&sources).expect("valid TOML");

    let error = config
        .resolve_credential("openrouter", &common::env_map(&[]))
        .expect_err("an untrusted command must not run, whichever profile wrote it");
    assert!(
        error.to_string().contains("--trust-project"),
        "the refusal names the flag: {error}"
    );
}

#[test]
fn an_untrusted_project_cannot_move_the_session_root() {
    // The confinement boundary of every file tool, and of the OS sandbox. A probe proved
    // the escape: with `session-root = "/tmp/escape"` in an untrusted file, `read` reached a
    // file that the same command refused without the file. A boundary is more powerful than
    // a capability, and it was not in the gate at all.
    let (config, _path, _dir) =
        load_project("session-root = \"/tmp/escape\"\n", ProjectTrust::Untrusted);
    assert_eq!(
        config.session_root, None,
        "an untrusted file must not move the confinement boundary"
    );
}

#[test]
fn a_trusted_project_may_move_the_session_root() {
    let (config, _path, _dir) =
        load_project("session-root = \"/tmp/escape\"\n", ProjectTrust::Trusted);
    assert_eq!(
        config.session_root,
        Some(PathBuf::from("/tmp/escape")),
        "with trust the user's own choice stands"
    );
}
