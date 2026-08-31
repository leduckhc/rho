//! The failure rule from `SPEC-config` section 6 and section 7.
//!
//! `rho-config` fails closed. A malformed file, an unknown key, and an unreadable file
//! each return a typed `ConfigError`. A broken security key stops the run and never
//! falls back to a permissive default. See decisions D-config-fails-closed and D-plugin-does-not-classify-itself.

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
    // This is the decision D-plugin-does-not-classify-itself fail-open family, in a new place.
    let dir = temp_dir();
    let path = write_file(&dir, "config.toml", "approval = \"bananas\"\n");
    let sources = sources_with_project_file(path);
    match Config::load(&sources) {
        // The value is bad, so the error must name the bad value, not something else. Per
        // SPEC-config section 6 the message names the key and the offending value, so a
        // generic parse error cannot pass a test about a security key.
        //
        // The variant is `Value`, because a merged value has no file to name. See
        // `D-a-merged-value-error-names-no-file`.
        Err(err @ ConfigError::Value { .. }) => {
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
        // The value is bad, so the error must name the bad value, not something else. Per
        // SPEC-config section 6 the message names the key and the offending value, so a
        // generic parse error cannot pass a test about a security key.
        //
        // The variant is `Value`, because a merged value has no file to name. See
        // `D-a-merged-value-error-names-no-file`.
        Err(err @ ConfigError::Value { .. }) => {
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

/// A refusal that came from the merge must read as a sentence.
///
/// It printed "cannot parse the config file the merged configuration: ...". A live run
/// found it, first for `tui-reasoning` and then again for `reasoning-effort`. The merge
/// has no path to name, so the error must not pretend it does. See
/// `D-the-merge-cannot-name-a-values-source`.
#[test]
fn a_merged_value_error_reads_as_a_sentence() {
    for (key, value) in [
        ("reasoning-effort", "ludicrous"),
        ("tui-reasoning", "loud"),
        ("sandbox", "sideways"),
        ("approval", "maybe"),
    ] {
        let dir = temp_dir();
        let project = write_file(&dir, "project.toml", &format!("{key} = \"{value}\"\n"));
        let sources = sources_with_project_file(project);
        let error = Config::load(&sources).expect_err("a bad value fails closed");
        let text = error.to_string();
        assert!(
            !text.contains("the config file the merged configuration"),
            "the merge has no file to name: {text}"
        );
        assert!(
            text.contains(key) && text.contains(value),
            "the error names the key and the value: {text}"
        );
    }
}

/// A project file arrives with a clone, so its **size** is attacker-controlled.
///
/// `read_file` used `std::fs::read_to_string`, which has no bound. A review measured it: a
/// 400 MB `.rho/config.toml` took 428 MB of resident memory, and an 800 MB one took 848 MB,
/// both before any trust gate ran. That is the defect this crate already caps everywhere
/// else, and the same family as the 8 MB of bash output that took 805 MB.
///
/// See `D-a-config-file-is-read-under-a-cap`.
#[test]
fn a_file_over_the_cap_is_refused_and_names_the_limit() {
    let dir = temp_dir();
    // One byte over. A cap tested far outside its window passes for a cap twice as large.
    let over = "#".repeat(1024 * 1024 + 1);
    let path = write_file(&dir, "config.toml", &over);
    match Config::read_file(&path) {
        Err(err @ ConfigError::TooLarge { .. }) => {
            let text = err.to_string();
            assert!(
                text.contains("config.toml"),
                "the error names the file: {text}"
            );
            assert!(
                text.contains("1048576"),
                "the error names the limit, so a user can act on it: {text}"
            );
        }
        other => panic!("a file over the cap must be TooLarge, got {other:?}"),
    }
}

/// The other side of the bound. A file **at** the cap is still a valid config, so the cap
/// refuses one byte and not one byte less.
#[test]
fn a_file_at_the_cap_is_accepted() {
    let dir = temp_dir();
    let key = "sandbox = \"off\"\n";
    let padding = "#".repeat(1024 * 1024 - key.len());
    let path = write_file(&dir, "config.toml", &format!("{padding}{key}"));
    let layer = Config::read_file(&path).expect("a file at the cap is still read");
    assert!(
        layer.is_some(),
        "a file at the cap parses, or the cap refuses one byte too many"
    );
}

/// The bound is on the **read**, and not on what the read kept.
///
/// This is the trap the bash line cap fell into: a test asserted the size of the kept
/// output while the buffer still grew without limit. A file cannot prove the difference,
/// because a test file is as small as the cap. A source with no end can: an unbounded read
/// never returns, and a bounded one refuses at once.
///
/// The read runs on its own thread and the assertion waits with a timeout, so an unbounded
/// read fails this test instead of hanging the suite.
#[cfg(unix)]
#[test]
fn a_source_with_no_end_is_refused_and_the_read_ends() {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = Config::read_file(std::path::Path::new("/dev/zero"));
        // A closed receiver means the assertion already gave up. Say nothing.
        let _ = tx.send(matches!(outcome, Err(ConfigError::TooLarge { .. })));
    });
    match rx.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(refused) => assert!(
            refused,
            "an endless source must be refused as TooLarge, not read to the end"
        ),
        Err(_) => panic!(
            "the read did not end in ten seconds, so it is not bounded: \
             `read_file` must cap the read itself, not the value it keeps"
        ),
    }
}
