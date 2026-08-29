//! A provider asks the merged configuration for its credential.
//!
//! Before this, `provider.rs` called `std::env::var(KEY).unwrap_or_default()`, so an absent
//! key became an empty string and the user read a provider 401. `Config` now answers, and an
//! absent credential is a typed error that names what to set.
//!
//! See `SPEC-config-call-site` section 7, `D-a-provider-names-its-own-credential`, and
//! `D-an-untrusted-clone-supplies-no-credential`.
//!
//! No test reads a real config path or the real process environment.

mod common;

use common::{env_map, project_sources, temp_dir, write_file};
use rho_config::{Config, ConfigPaths, ProjectTrust, Sources};

/// The credential name and the fallback variable are deliberately different words in every
/// test here, so a swap of the two arguments of `resolve_credential_or_env` fails a test
/// instead of passing one. `from_paths_maps_the_two_paths` was written for the same reason.
const NAME: &str = "openrouter";
const FALLBACK: &str = "OPENROUTER_API_KEY";

/// Load a config from one global file. A global file is the user's own, so nothing is gated.
fn load_global(contents: &str) -> Config {
    let dir = temp_dir();
    let global = write_file(&dir, "global.toml", contents);
    let sources = Sources::from_paths(ConfigPaths {
        global: Some(global),
        project: None,
        ..Default::default()
    });
    Config::load(&sources).expect("the file is valid TOML")
}

#[test]
fn an_absent_credential_is_an_error_not_an_empty_key() {
    // The whole point of I15. No entry names the credential and the fallback variable is
    // unset, so the run stops with a sentence naming both. It never returns an empty
    // `Secret`, because an empty key reaches the provider and returns 401.
    let config = load_global("provider = \"openrouter\"\n");
    let error = config
        .resolve_credential_or_env(NAME, FALLBACK, &env_map(&[]))
        .expect_err("an absent credential must be an error, never an empty key");
    let message = error.to_string();
    assert!(
        message.contains(NAME),
        "the message must name the credentials entry: {message}"
    );
    assert!(
        message.contains(FALLBACK),
        "and it must name the variable to set: {message}"
    );
}

#[test]
fn an_empty_credential_is_an_error() {
    // An exported-but-empty variable is not a key. Both the entry and the fallback are
    // empty here, so the test cannot pass because the fallback was merely absent.
    let config = load_global("[credentials]\nopenrouter = \"env:OPENROUTER_API_KEY\"\n");
    let error = config
        .resolve_credential_or_env(NAME, FALLBACK, &env_map(&[(FALLBACK, "")]))
        .expect_err("an empty credential must be an error");
    assert!(
        error.to_string().contains(NAME),
        "the message names the credential: {error}"
    );
}

#[test]
fn a_credentials_entry_beats_the_fallback_variable() {
    // A config file really chooses the key. The entry value and the variable value differ,
    // so an implementation that read the variable first fails here.
    let config = load_global("[credentials]\nopenrouter = \"sk-from-the-file\"\n");
    let secret = config
        .resolve_credential_or_env(NAME, FALLBACK, &env_map(&[(FALLBACK, "sk-from-the-env")]))
        .expect("the entry resolves");
    assert_eq!(secret.expose(), "sk-from-the-file");
}

#[test]
fn the_fallback_variable_resolves_when_no_entry_names_it() {
    // U2(a) of `.rho-work/i15-credential-expansion.md`. No user has a `[credentials]` table
    // today, so an absent entry must still work through the documented variable.
    let config = load_global("provider = \"openrouter\"\n");
    let secret = config
        .resolve_credential_or_env(NAME, FALLBACK, &env_map(&[(FALLBACK, "sk-from-the-env")]))
        .expect("the fallback resolves");
    assert_eq!(secret.expose(), "sk-from-the-env");
}

#[test]
fn a_refused_project_credential_does_not_fall_back() {
    // The refusal must not degrade into an absence. **The fallback variable is set here on
    // purpose.** With it unset, correct code and an `.or_else` fallback both fail, so the
    // test would pass while proving nothing. A review named that trap.
    let dir = temp_dir();
    let project = write_file(
        &dir,
        "project.toml",
        "[credentials]\nopenrouter = \"!echo leaked\"\n",
    );
    let config =
        Config::load(&project_sources(project).with_project_trust(ProjectTrust::Untrusted))
            .expect("the file is valid TOML");

    let error = config
        .resolve_credential_or_env(NAME, FALLBACK, &env_map(&[(FALLBACK, "sk-from-the-env")]))
        .expect_err("a refused project credential must stay refused, and never fall back");
    let message = error.to_string();
    assert!(
        message.contains("--trust-project"),
        "the message must name the flag that fixes it: {message}"
    );
}

#[test]
fn resolving_a_credential_twice_gives_the_same_answer() {
    // I9. A provider is built once per process, and a helper may run more than once. The
    // second run must give the same answer, or fail the same way.
    let dir = temp_dir();
    let global = write_file(
        &dir,
        "global.toml",
        "[credentials]\nopenrouter = \"!printf %s sk-from-the-helper\"\n",
    );
    let sources = Sources::from_paths(ConfigPaths {
        global: Some(global),
        project: None,
        ..Default::default()
    });
    let config = Config::load(&sources).expect("the file is valid TOML");

    let first = config
        .resolve_credential_or_env(NAME, FALLBACK, &env_map(&[("PATH", path_value())]))
        .expect("the helper runs");
    let second = config
        .resolve_credential_or_env(NAME, FALLBACK, &env_map(&[("PATH", path_value())]))
        .expect("and it runs again");
    assert_eq!(first.expose(), second.expose());
    assert_eq!(first.expose(), "sk-from-the-helper");
}

#[test]
fn a_credential_error_never_holds_the_resolved_value() {
    // `ProviderError::Credential` passes a `rho-config` message through whole, so no
    // `ConfigError::Credential` message may hold a credential value. `Secret` has no
    // `Display`, so this can only break through a string built by hand.
    let secret = "SUP3RSECRET-VALUE";
    let dir = temp_dir();

    // A helper that prints the value and then fails. The value is on stdout, and the error
    // must name the status only.
    //
    // The helper is a script file, not an inline `sh -c '...'`. `CredentialSource::parse`
    // splits a command on whitespace and honours no quoting, so an inline script would run
    // as `sh -c "'printf"` and fail for the wrong reason. It did, and this test passed
    // against it.
    let helper = write_script(&dir, "fails.sh", &format!("printf %s {secret}\nexit 3\n"));
    let global = write_file(
        &dir,
        "global.toml",
        &format!("[credentials]\nopenrouter = \"!{helper}\"\n"),
    );
    let sources = Sources::from_paths(ConfigPaths {
        global: Some(global),
        project: None,
        ..Default::default()
    });
    let config = Config::load(&sources).expect("the file is valid TOML");
    let error = config
        .resolve_credential_or_env(NAME, FALLBACK, &env_map(&[("PATH", path_value())]))
        .expect_err("a helper that exits non-zero fails the resolve");
    assert!(
        error.to_string().contains("status"),
        "the helper really ran and really failed: {error}"
    );
    assert!(
        !error.to_string().contains(secret),
        "an error must not hold the credential value: {error}"
    );

    // An interpolation whose value is present but whose template is broken.
    let global = write_file(
        &dir,
        "global2.toml",
        "[credentials]\nopenrouter = \"Bearer ${UNSET_TOKEN_NAME}\"\n",
    );
    let sources = Sources::from_paths(ConfigPaths {
        global: Some(global),
        project: None,
        ..Default::default()
    });
    let config = Config::load(&sources).expect("the file is valid TOML");
    let error = config
        .resolve_credential_or_env(NAME, FALLBACK, &env_map(&[(FALLBACK, secret)]))
        .expect_err("an unset interpolation span fails the resolve");
    assert!(
        !error.to_string().contains(secret),
        "an interpolation error must not hold the fallback value either: {error}"
    );
}

#[test]
fn a_userinfo_base_url_error_hides_the_password() {
    // The leak a review found beside this path. A base url may carry a password, and the
    // refusal echoed the value whole onto stderr. The message keeps the host, so the user
    // still knows which endpoint they set.
    let dir = temp_dir();
    let global = write_file(
        &dir,
        "global.toml",
        "base-url = \"https://alice:sup3rsecret@models.example.com/v1\"\n",
    );
    let sources = Sources::from_paths(ConfigPaths {
        global: Some(global),
        project: None,
        ..Default::default()
    });
    let error = Config::load(&sources).expect_err("a url with userinfo is refused");
    let message = error.to_string();
    assert!(
        !message.contains("sup3rsecret"),
        "the refusal must not print the password: {message}"
    );
    assert!(
        message.contains("models.example.com"),
        "and it must still name the host the user set: {message}"
    );
}

#[test]
fn a_credential_command_stderr_does_not_reach_the_parent() {
    // A chatty helper, or a `set -x` in a wrapper script, could print a key onto rho's own
    // stderr. `resolve_command` piped stdout and left stderr inherited.
    //
    // The child here is this same test binary, re-run with a marker variable, so its stderr
    // can be captured with a pipe. Redirecting this process's own stderr needs `libc`, and a
    // test must not add a dependency for one assertion.
    const MARKER: &str = "KEY-ON-STDERR";
    if std::env::var("RHO_CREDENTIAL_STDERR_PROBE").is_ok() {
        let dir = temp_dir();
        let helper = write_script(
            &dir,
            "chatty.sh",
            &format!("echo {MARKER} >&2\nprintf %s ok\n"),
        );
        let global = write_file(
            &dir,
            "global.toml",
            &format!("[credentials]\nopenrouter = \"!{helper}\"\n"),
        );
        let sources = Sources::from_paths(ConfigPaths {
            global: Some(global),
            project: None,
            ..Default::default()
        });
        let config = Config::load(&sources).expect("the file is valid TOML");
        let secret = config
            .resolve_credential_or_env(NAME, FALLBACK, &env_map(&[("PATH", path_value())]))
            .expect("the helper runs");
        assert_eq!(secret.expose(), "ok");
        return;
    }

    let exe = std::env::current_exe().expect("the test binary");
    let output = std::process::Command::new(exe)
        .args([
            "a_credential_command_stderr_does_not_reach_the_parent",
            "--exact",
            "--nocapture",
        ])
        .env("RHO_CREDENTIAL_STDERR_PROBE", "1")
        .output()
        .expect("the child test runs");
    assert!(
        output.status.success(),
        "the child probe must pass: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains(MARKER),
        "a helper's stderr must not reach rho's stderr, got: {stderr}"
    );
}

/// The real `PATH`, so a helper can find `sh` and `printf`. It is read once, here, and never
/// mutated, because a test must not change the process environment.
fn path_value() -> &'static str {
    // A fixed, minimal set. Reading the real `PATH` would make the result depend on the
    // machine, which is exactly what the isolation rule forbids.
    "/bin:/usr/bin"
}

/// Write an executable shell script under `dir`, and return the command line that runs it.
///
/// `CredentialSource::parse` splits a command on whitespace and honours no quoting, so an
/// inline `sh -c '...'` becomes nonsense argv. A script file has no spaces to split.
fn write_script(dir: &tempfile::TempDir, name: &str, body: &str) -> String {
    let path = dir.path().join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}")).expect("write the helper script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .expect("make the helper executable");
    }
    let command = path.display().to_string();
    assert!(
        !command.contains(' '),
        "the temp path must hold no space, because parse splits on whitespace: {command}"
    );
    command
}
