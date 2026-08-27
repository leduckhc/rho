//! The merge order from `SPEC-config` section 2 and section 7.
//!
//! A later layer wins over an earlier one. The stronger layer replaces only a value
//! it sets, and it leaves a value it does not set. So the winner for one key is the
//! strongest layer that names that key.

mod common;

use std::path::{Path, PathBuf};

use common::{project_sources, temp_dir, write_file};
use rho_config::{ApprovalMode, Config, ConfigLayer, ProjectTrust};
use rho_core::SandboxMode;

/// A config file that sets every key `ConfigLayer` holds.
///
/// `every_scalar_key_merges_and_reaches_the_config` reads it. Add a key here when you add
/// a field, because the test fails until you do.
const EVERY_KEY: &str = "\
provider = \"openrouter\"\n\
model = \"a-model\"\n\
session-root = \"/tmp/root\"\n\
session-file = \"/tmp/root/session.jsonl\"\n\
ephemeral = true\n\
sandbox = \"confined\"\n\
approval = \"read-only\"\n\
skill-paths = [\"/skills/a\"]\n\
no-skills = true\n\
tui-mouse = true\n\
tui-reasoning = \"summary\"\n\
reasoning-effort = \"high\"\n\
mcp-config = \"/tmp/mcp.json\"\n\
base-url = \"https://models.example.com/v1\"\n\
tui-motion = false\n\
no-agents = true\n\
[subagents]\n\
max-depth = 3\n\
max-children-per-parent = 4\n\
max-live-total = 5\n\
child-timeout-secs = 30\n\
[credentials]\n\
openrouter = \"env:RHO_TEST_KEY\"\n\
[profiles.fast]\n\
model = \"profile-model\"\n";

/// Every `Option` field of a layer, as Debug text, with the profile map removed.
///
/// A profile is a nested layer that states only what it overrides, so its own unset keys
/// read as `None` and would defeat the sweep. The caller checks the profiles by name.
fn sweep(layer: &ConfigLayer) -> String {
    let mut without_profiles = layer.clone();
    let _ = std::mem::take(&mut without_profiles.profiles);
    format!("{without_profiles:?}")
}

#[test]
fn merge_prefers_the_project_file_over_the_global_file() {
    // A key set in both resolves to the project value.
    let global = ConfigLayer {
        model: Some("global-model".to_string()),
        provider: Some("openrouter".to_string()),
        ..ConfigLayer::default()
    };
    let project = ConfigLayer {
        model: Some("project-model".to_string()),
        ..ConfigLayer::default()
    };
    let merged = global.merge(project);
    assert_eq!(merged.model.as_deref(), Some("project-model"));
    // Invariant: the stronger layer omits `provider`, so the lower value survives.
    assert_eq!(merged.provider.as_deref(), Some("openrouter"));
}

#[test]
fn every_scalar_key_merges_and_reaches_the_config() {
    // `ConfigLayer::merge` assigns one field per line, by hand. A forgotten line drops
    // that value in silence, and no other test sees it, because each of them names one
    // key. So this test sets **every** key and then sweeps for a single `None`.
    //
    // The sweep reads the Debug text rather than a list of fields. A hand-kept list is
    // the very thing that failed here, so a new field needs no edit to this assertion:
    // an unmerged field prints `None` and fails, whichever field it is.
    //
    // See `SPEC-config-call-site` and `SPEC-config` section 2.
    let dir = temp_dir();
    let path = write_file(&dir, "config.toml", EVERY_KEY);
    let file = Config::read_file(&path)
        .expect("a present file is not an error")
        .expect("a present file is Some");

    // Guard one: the fixture is complete. Without this, a field added to `ConfigLayer`
    // would fail the merge assertion below and read as a merge defect, which it is not.
    assert!(
        !sweep(&file).contains("None"),
        "EVERY_KEY must set every key of ConfigLayer, and it leaves one unset: {}",
        sweep(&file)
    );

    // Guard two: the merge carries every one of them over an empty lower layer.
    let merged = ConfigLayer::default().merge(file);
    assert!(
        !sweep(&merged).contains("None"),
        "ConfigLayer::merge dropped a key it must carry: {}",
        sweep(&merged)
    );
    assert_eq!(
        merged.profiles.len(),
        1,
        "the profile union must survive the merge"
    );

    // Guard three: each value reaches the resolved `Config`, not merely the layer. A
    // merge that assigned the wrong field would pass the sweep and fail here.
    let sources = project_sources(path).with_project_trust(ProjectTrust::Trusted);
    let config = Config::load(&sources).expect("every stated key resolves");
    assert_eq!(config.provider.as_deref(), Some("openrouter"));
    assert_eq!(config.model.as_deref(), Some("a-model"));
    assert_eq!(config.session_root.as_deref(), Some(Path::new("/tmp/root")));
    assert_eq!(
        config.session_file.as_deref(),
        Some(Path::new("/tmp/root/session.jsonl"))
    );
    assert!(config.ephemeral);
    assert_eq!(config.sandbox, SandboxMode::Confined);
    assert_eq!(config.approval, Some(ApprovalMode::ReadOnly));
    assert_eq!(config.skill_paths, vec![PathBuf::from("/skills/a")]);
    assert!(
        !config.discover_skills,
        "no-skills = true disables discovery"
    );
    assert_eq!(
        config.mcp_config.as_deref(),
        Some(Path::new("/tmp/mcp.json"))
    );
}

#[test]
fn merge_prefers_a_profile_over_a_plain_file_value() {
    // A profile value beats a file value. The merge applies the profile after the file.
    let file = ConfigLayer {
        model: Some("file-model".to_string()),
        ..ConfigLayer::default()
    };
    let profile = ConfigLayer {
        model: Some("profile-model".to_string()),
        ..ConfigLayer::default()
    };
    let merged = file.merge(profile);
    assert_eq!(merged.model.as_deref(), Some("profile-model"));
}

#[test]
fn merge_prefers_the_environment_over_a_file() {
    // `RHO_MODEL` beats a file `model`.
    let file = ConfigLayer {
        model: Some("file-model".to_string()),
        ..ConfigLayer::default()
    };
    let env = ConfigLayer::from_env(&[("RHO_MODEL".to_string(), "env-model".to_string())]);
    let merged = file.merge(env);
    assert_eq!(merged.model.as_deref(), Some("env-model"));
}

#[test]
fn merge_prefers_a_flag_over_the_environment() {
    // A `--model` flag beats `RHO_MODEL`.
    let env = ConfigLayer::from_env(&[("RHO_MODEL".to_string(), "env-model".to_string())]);
    let flags = ConfigLayer {
        model: Some("flag-model".to_string()),
        ..ConfigLayer::default()
    };
    let merged = env.merge(flags);
    assert_eq!(merged.model.as_deref(), Some("flag-model"));
}

#[test]
fn merge_keeps_a_lower_value_the_stronger_layer_omits() {
    // A gap in the flags keeps the file value.
    let file = ConfigLayer {
        model: Some("file-model".to_string()),
        provider: Some("openrouter".to_string()),
        ..ConfigLayer::default()
    };
    let flags = ConfigLayer {
        model: Some("flag-model".to_string()),
        ..ConfigLayer::default()
    };
    let merged = file.merge(flags);
    assert_eq!(merged.model.as_deref(), Some("flag-model"));
    assert_eq!(
        merged.provider.as_deref(),
        Some("openrouter"),
        "a gap in the stronger layer must keep the lower value"
    );
}
