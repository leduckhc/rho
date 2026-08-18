//! The failure rule from `SPEC-13` section 6 and section 7.
//!
//! `rho-config` fails closed. A malformed file, an unknown key, and an unreadable file
//! each return a typed `ConfigError`. A broken security key stops the run and never
//! falls back to a permissive default. See decisions D-047 and D-017.

mod common;

use common::{sources_with_project_file, temp_dir, write_file};
use rho_config::{Config, ConfigError};

#[test]
fn a_missing_file_is_ok_none() {
    // `read_file` on an absent path returns `Ok(None)`. A user with no project file
    // still runs.
    let dir = temp_dir();
    let path = dir.path().join("does-not-exist.toml");
    let layer = Config::read_file(&path).expect("a missing file is not an error");
    assert!(layer.is_none(), "an absent file must be Ok(None)");
}

#[test]
fn a_malformed_file_is_a_parse_error() {
    // Invalid TOML returns `ConfigError::Parse`. The run does not use a default in
    // place of the file.
    let dir = temp_dir();
    let path = write_file(&dir, "config.toml", "this is = = not toml");
    match Config::read_file(&path) {
        Err(err @ ConfigError::Parse { .. }) => {
            // The message must name the file that failed, not report a generic parse.
            assert!(
                err.to_string().contains("config.toml"),
                "the parse error must name the file that failed: {err}"
            );
        }
        other => panic!("malformed TOML must be a Parse error, got {other:?}"),
    }
}

#[test]
fn an_unknown_key_is_a_parse_error() {
    // An unknown key returns `ConfigError::Parse`, through `deny_unknown_fields`. A
    // typo in a security key must not pass unseen.
    let dir = temp_dir();
    let path = write_file(&dir, "config.toml", "not-a-real-key = \"value\"\n");
    match Config::read_file(&path) {
        Err(err @ ConfigError::Parse { .. }) => {
            // The message must name the offending key, so a typo is easy to fix and a
            // generic error cannot pass a test about an unknown key.
            assert!(
                err.to_string().contains("not-a-real-key"),
                "the parse error must name the unknown key: {err}"
            );
        }
        other => panic!("an unknown key must be a Parse error, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn an_unreadable_file_is_a_read_error() {
    // A file the user meant to apply, but that the process cannot read, is a hard
    // error, not an empty layer.
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir();
    let path = write_file(&dir, "config.toml", "model = \"m\"\n");
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o000);
    std::fs::set_permissions(&path, perms).unwrap();
    match Config::read_file(&path) {
        Err(ConfigError::Read { .. }) => {}
        other => panic!("an unreadable file must be a Read error, got {other:?}"),
    }
}

#[test]
fn a_broken_approval_key_stops_the_run() {
    // A bad `approval` value is an error, and the run never falls back to `allow-all`.
    // This is the decision D-017 fail-open family, in a new place.
    let dir = temp_dir();
    let path = write_file(&dir, "config.toml", "approval = \"bananas\"\n");
    let sources = sources_with_project_file(path);
    match Config::load(&sources) {
        // The value is bad, so the error must name the parse, not something else. Per
        // SPEC-13 section 6 the message names the key and the offending value, so a
        // generic parse error cannot pass a test about a security key.
        Err(err @ ConfigError::Parse { .. }) => {
            let rendered = err.to_string();
            assert!(
                rendered.contains("approval"),
                "the parse error must name the approval key: {rendered}"
            );
            assert!(
                rendered.contains("bananas"),
                "the parse error must name the offending value: {rendered}"
            );
        }
        Err(other) => panic!("a broken approval key must be a Parse error, got {other:?}"),
        Ok(config) => panic!(
            "a broken approval key must stop the run, but it resolved to {:?}. \
             It must never fall back to allow-all.",
            config.approval
        ),
    }
}

#[test]
fn a_broken_sandbox_key_stops_the_run() {
    // A bad `sandbox` value is an error, and the run never falls back to a weaker mode.
    // The security key gets the same guard as `approval`.
    let dir = temp_dir();
    let path = write_file(&dir, "config.toml", "sandbox = \"loose\"\n");
    let sources = sources_with_project_file(path);
    match Config::load(&sources) {
        // The value is bad, so the error must name the parse, not something else. Per
        // SPEC-13 section 6 the message names the key and the offending value, so a
        // generic parse error cannot pass a test about a security key.
        Err(err @ ConfigError::Parse { .. }) => {
            let rendered = err.to_string();
            assert!(
                rendered.contains("sandbox"),
                "the parse error must name the sandbox key: {rendered}"
            );
            assert!(
                rendered.contains("loose"),
                "the parse error must name the offending value: {rendered}"
            );
        }
        Err(other) => panic!("a broken sandbox key must be a Parse error, got {other:?}"),
        Ok(config) => panic!(
            "a broken sandbox key must stop the run, but it resolved to {:?}. \
             It must never fall back to a weaker mode.",
            config.sandbox
        ),
    }
}
