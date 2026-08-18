//! Defaults, layers, and parsing from `SPEC-13` section 4 and section 7.
//!
//! These tests pin the stated defaults, the single environment layer, and the
//! fail-closed parsers for the two security keys.

mod common;

use std::str::FromStr;

use common::{env_vars, temp_dir, write_file};
use rho_config::{ApprovalMode, Config, ConfigLayer, EnvLookup, Sources, SystemEnv};
use rho_core::SandboxMode;

#[test]
fn config_defaults_sets_the_stated_defaults() {
    // `Config::defaults` leaves `approval` unset, because the resolved default comes
    // from the SPEC-16 mode resolution. It sets `sandbox` to `off`, per D-031.
    let defaults = Config::defaults();
    assert!(
        defaults.approval.is_none(),
        "the approval default must stay unset, so the resolution table sets it"
    );
    assert_eq!(
        defaults.sandbox.as_deref(),
        Some("off"),
        "the sandbox default must be off, stated and not hidden"
    );
}

#[test]
fn from_env_builds_a_layer_from_rho_variables() {
    // `ConfigLayer::from_env` maps `RHO_MODEL` and `RHO_PROVIDER` into a layer.
    let layer = ConfigLayer::from_env(&[
        ("RHO_MODEL".to_string(), "env-model".to_string()),
        ("RHO_PROVIDER".to_string(), "env-provider".to_string()),
    ]);
    assert_eq!(layer.model.as_deref(), Some("env-model"));
    assert_eq!(layer.provider.as_deref(), Some("env-provider"));
}

#[test]
fn load_merges_and_resolves_end_to_end() {
    // `Config::load` reads the files, applies the profile, the environment, and the
    // flags, and resolves the credentials in one call.
    let dir = temp_dir();
    let global = write_file(
        &dir,
        "global.toml",
        "provider = \"openrouter\"\n\
         model = \"global-model\"\n",
    );
    let project = write_file(
        &dir,
        "project.toml",
        "model = \"project-model\"\n\
         \n\
         [profiles.fast]\n\
         model = \"fast-model\"\n",
    );
    let sources = Sources {
        global_file: Some(global),
        project_file: Some(project),
        profile: Some("fast".to_string()),
        env: env_vars(&[("RHO_PROVIDER", "env-provider")]),
        flags: ConfigLayer {
            model: Some("flag-model".to_string()),
            ..ConfigLayer::default()
        },
    };
    let config = Config::load(&sources).expect("the sources resolve");
    // The flag is the strongest layer that names `model`.
    assert_eq!(config.model.as_deref(), Some("flag-model"));
    // The environment names `provider`, and it beats both files.
    assert_eq!(config.provider.as_deref(), Some("env-provider"));
}

#[test]
fn approval_mode_parses_each_name_and_fails_closed() {
    // `ApprovalMode::from_str` parses each name and fails closed on an unknown name.
    assert_eq!(
        ApprovalMode::from_str("read-only"),
        Ok(ApprovalMode::ReadOnly)
    );
    assert_eq!(ApprovalMode::from_str("ask"), Ok(ApprovalMode::Ask));
    assert_eq!(
        ApprovalMode::from_str("allow-all"),
        Ok(ApprovalMode::AllowAll)
    );
    match ApprovalMode::from_str("bananas") {
        Err(message) => {
            // The message names the valid set, so a user can fix the value.
            assert!(message.contains("read-only"), "message: {message}");
            assert!(message.contains("ask"), "message: {message}");
            assert!(message.contains("allow-all"), "message: {message}");
        }
        Ok(mode) => panic!("an unknown approval name must fail, got {mode:?}"),
    }
}

#[test]
fn a_sandbox_key_parses_through_sandbox_mode() {
    // A `sandbox` value parses through `SandboxMode::from_str`, so the config and the
    // core share one parser.
    let dir = temp_dir();
    let path = write_file(&dir, "config.toml", "sandbox = \"confined\"\n");
    let sources = Sources {
        project_file: Some(path),
        ..Sources::default()
    };
    let config = Config::load(&sources).expect("a valid sandbox value");
    assert_eq!(config.sandbox, SandboxMode::from_str("confined").unwrap());
    assert_eq!(config.sandbox, SandboxMode::Confined);
}

#[test]
fn system_env_reads_the_real_environment() {
    // `SystemEnv` reads a real variable, so the production `EnvLookup` is proved. The
    // test never sets a process variable. It reads `PATH`, which the process already
    // holds, and it compares against `std::env::var`, so the result does not depend on
    // a machine-specific value.
    let system = SystemEnv;
    assert_eq!(system.get("PATH"), std::env::var("PATH").ok());
}

#[test]
fn the_environment_is_read_in_one_layer_only() {
    // `RHO_MODEL` with no `--model` flag reaches the config through the environment
    // layer. A `--model` flag beats it. The variable is not double-counted as a flag.
    let from_env = Sources {
        env: env_vars(&[("RHO_MODEL", "env-model")]),
        ..Sources::default()
    };
    let config = Config::load(&from_env).expect("the environment layer resolves");
    assert_eq!(
        config.model.as_deref(),
        Some("env-model"),
        "with no flag, the variable reaches the config through the environment layer"
    );

    let with_flag = Sources {
        env: env_vars(&[("RHO_MODEL", "env-model")]),
        flags: ConfigLayer {
            model: Some("flag-model".to_string()),
            ..ConfigLayer::default()
        },
        ..Sources::default()
    };
    let config = Config::load(&with_flag).expect("the flag layer resolves");
    assert_eq!(
        config.model.as_deref(),
        Some("flag-model"),
        "a flag beats the variable, and the variable is not double-counted"
    );
}

#[test]
fn an_unset_read_only_flag_does_not_override_a_file_approval() {
    // A run with a file `approval` of `read-only` and no `--read-only` flag keeps
    // `read-only`, because the flag layer sets `approval` only when the user passes
    // the flag.
    let dir = temp_dir();
    let path = write_file(&dir, "config.toml", "approval = \"read-only\"\n");
    let sources = Sources {
        project_file: Some(path),
        // The flags layer names no approval, exactly as an unset `--read-only` flag.
        flags: ConfigLayer::default(),
        ..Sources::default()
    };
    let config = Config::load(&sources).expect("a valid approval value");
    assert_eq!(
        config.approval,
        Some(ApprovalMode::ReadOnly),
        "an unset flag must not widen a stricter file approval"
    );
}
