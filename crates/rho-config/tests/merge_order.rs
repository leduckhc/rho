//! The merge order from `SPEC-13` section 2 and section 7.
//!
//! A later layer wins over an earlier one. The stronger layer replaces only a value
//! it sets, and it leaves a value it does not set. So the winner for one key is the
//! strongest layer that names that key.

mod common;

use rho_config::ConfigLayer;

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
