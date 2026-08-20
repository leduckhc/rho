//! The environment layer from `SPEC-config` section 2 (layer 5) and F-environment-variable-override.
//!
//! F-environment-variable-override says every config key can be set with an environment variable, in the
//! pattern `RHO_<KEY>`. These tests pin the full mapping, the boolean rule, and the
//! fail-closed guard for the two security keys. No test mutates the process
//! environment. `Sources.env` is the injected seam, so every value is in memory.

mod common;

use std::path::PathBuf;

use common::env_vars;
use rho_config::{ApprovalMode, Config, ConfigError, ConfigLayer, Sources};
use rho_core::SandboxMode;

/// Every `RHO_<KEY>` variable for the scalar keys, with a value for each.
fn every_scalar_env() -> Vec<(String, String)> {
    env_vars(&[
        ("RHO_PROVIDER", "env-provider"),
        ("RHO_MODEL", "env-model"),
        ("RHO_SESSION_ROOT", "/tmp/root"),
        ("RHO_SESSION_FILE", "/tmp/root/session.jsonl"),
        ("RHO_EPHEMERAL", "true"),
        ("RHO_SANDBOX", "confined"),
        ("RHO_APPROVAL", "read-only"),
        ("RHO_SKILL_PATHS", "/skills/a:/skills/b"),
        ("RHO_NO_SKILLS", "yes"),
        ("RHO_MCP_CONFIG", "/tmp/mcp.json"),
    ])
}

#[test]
fn from_env_maps_every_scalar_key() {
    // `ConfigLayer::from_env` maps each `RHO_<KEY>` variable to its layer field. Two
    // keys were mapped before this stage: provider and model. The rest are new.
    let layer = ConfigLayer::from_env(&every_scalar_env());
    assert_eq!(layer.provider.as_deref(), Some("env-provider"));
    assert_eq!(layer.model.as_deref(), Some("env-model"));
    assert_eq!(layer.session_root, Some(PathBuf::from("/tmp/root")));
    assert_eq!(
        layer.session_file,
        Some(PathBuf::from("/tmp/root/session.jsonl"))
    );
    assert_eq!(layer.ephemeral, Some(true));
    assert_eq!(layer.sandbox.as_deref(), Some("confined"));
    assert_eq!(layer.approval.as_deref(), Some("read-only"));
    assert_eq!(
        layer.skill_paths,
        Some(vec![PathBuf::from("/skills/a"), PathBuf::from("/skills/b")])
    );
    assert_eq!(layer.no_skills, Some(true));
    assert_eq!(layer.mcp_config, Some(PathBuf::from("/tmp/mcp.json")));
}

#[test]
fn load_reads_every_scalar_key_from_the_environment() {
    // A full `Config::load` reads each scalar key from the environment layer. This
    // proves the mapping reaches the resolved `Config`, not merely the raw layer.
    let sources = Sources {
        env: every_scalar_env(),
        ..Sources::default()
    };
    let config = Config::load(&sources).expect("the environment layer resolves");
    assert_eq!(config.provider.as_deref(), Some("env-provider"));
    assert_eq!(config.model.as_deref(), Some("env-model"));
    assert_eq!(config.session_root, Some(PathBuf::from("/tmp/root")));
    assert_eq!(
        config.session_file,
        Some(PathBuf::from("/tmp/root/session.jsonl"))
    );
    assert!(config.ephemeral, "RHO_EPHEMERAL=true must set ephemeral");
    assert_eq!(config.sandbox, SandboxMode::Confined);
    assert_eq!(config.approval, Some(ApprovalMode::ReadOnly));
    assert_eq!(
        config.skill_paths,
        vec![PathBuf::from("/skills/a"), PathBuf::from("/skills/b")]
    );
    assert!(
        !config.discover_skills,
        "RHO_NO_SKILLS=yes must disable discovery"
    );
    assert_eq!(config.mcp_config, Some(PathBuf::from("/tmp/mcp.json")));
}

#[test]
fn env_sandbox_bad_value_fails_closed() {
    // `RHO_SANDBOX=loose` must stop the run with a `ConfigError::Parse` that names the
    // key and the value. An environment value must not be a softer path into the same
    // security setting than a file value. See D-plugin-does-not-classify-itself and D-config-fails-closed.
    let sources = Sources {
        env: env_vars(&[("RHO_SANDBOX", "loose")]),
        ..Sources::default()
    };
    match Config::load(&sources) {
        Err(ConfigError::Parse { message, .. }) => {
            assert!(message.contains("sandbox"), "message: {message}");
            assert!(message.contains("loose"), "message: {message}");
        }
        other => panic!("a bad env sandbox value must fail closed, got {other:?}"),
    }
}

#[test]
fn env_approval_bad_value_fails_closed() {
    // `RHO_APPROVAL=bananas` must stop the run with a `ConfigError::Parse` that names
    // the key and the value. The environment must never widen a security key.
    let sources = Sources {
        env: env_vars(&[("RHO_APPROVAL", "bananas")]),
        ..Sources::default()
    };
    match Config::load(&sources) {
        Err(ConfigError::Parse { message, .. }) => {
            assert!(message.contains("approval"), "message: {message}");
            assert!(message.contains("bananas"), "message: {message}");
        }
        other => panic!("a bad env approval value must fail closed, got {other:?}"),
    }
}

#[test]
fn env_ephemeral_accepts_the_stated_truthy_and_falsy_values() {
    // The boolean rule: `1`, `true`, and `yes` are true. `0`, `false`, and `no` are
    // false. The rule is case-insensitive and trims surrounding space.
    for value in ["1", "true", "yes", "TRUE", " Yes "] {
        let sources = Sources {
            env: env_vars(&[("RHO_EPHEMERAL", value)]),
            ..Sources::default()
        };
        let config = Config::load(&sources).expect("an accepted truthy value loads");
        assert!(config.ephemeral, "value {value:?} must be true");
    }
    for value in ["0", "false", "no", "NO"] {
        let sources = Sources {
            env: env_vars(&[("RHO_EPHEMERAL", value)]),
            ..Sources::default()
        };
        let config = Config::load(&sources).expect("an accepted falsy value loads");
        assert!(!config.ephemeral, "value {value:?} must be false");
    }
}

#[test]
fn env_ephemeral_unaccepted_value_fails_closed() {
    // A value the rule does not accept fails closed, rather than reading as false. The
    // error names the key and the value.
    let sources = Sources {
        env: env_vars(&[("RHO_EPHEMERAL", "nonsense")]),
        ..Sources::default()
    };
    match Config::load(&sources) {
        Err(ConfigError::Parse { message, .. }) => {
            assert!(message.contains("ephemeral"), "message: {message}");
            assert!(message.contains("nonsense"), "message: {message}");
        }
        other => panic!("an unaccepted boolean must fail closed, got {other:?}"),
    }
}

#[test]
fn env_no_skills_unaccepted_value_fails_closed() {
    // The `no-skills` boolean obeys the same rule as `ephemeral`, and fails closed.
    let sources = Sources {
        env: env_vars(&[("RHO_NO_SKILLS", "maybe")]),
        ..Sources::default()
    };
    match Config::load(&sources) {
        Err(ConfigError::Parse { message, .. }) => {
            assert!(message.contains("no-skills"), "message: {message}");
            assert!(message.contains("maybe"), "message: {message}");
        }
        other => panic!("an unaccepted boolean must fail closed, got {other:?}"),
    }
}

#[test]
fn from_env_leaves_an_unaccepted_boolean_unset() {
    // `ConfigLayer::from_env` is infallible per SPEC-config section 3. It omits an
    // unaccepted boolean, and `Config::load` is the fail-closed authority that rejects
    // it. This pins the split, so neither side drifts into a soft default.
    let layer = ConfigLayer::from_env(&env_vars(&[("RHO_EPHEMERAL", "nonsense")]));
    assert_eq!(
        layer.ephemeral, None,
        "from_env must omit an unaccepted boolean, not read it as false"
    );
}
