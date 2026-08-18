//! Credential resolution from `SPEC-config` section 5 and section 7.
//!
//! A credential source resolves to a `Secret`. A `Literal` is a `Secret` from the
//! moment it is parsed. A command source runs a program and inherits only an
//! allowlist, per decision D-credential-command-allowlist. Every test uses an in-memory environment.

mod common;

use common::env_map;
use rho_config::CredentialSource;
use rho_core::Secret;

#[test]
fn a_literal_credential_resolves_to_its_value() {
    // `Literal` resolves to the written value.
    let source = CredentialSource::Literal(Secret::new("sk-live-abc"));
    let env = env_map(&[]);
    let secret = source.resolve("api", &env).expect("a literal resolves");
    assert_eq!(secret.expose(), "sk-live-abc");
}

#[test]
fn an_env_credential_reads_the_named_variable() {
    // `Env` reads the whole value of one variable.
    let source = CredentialSource::Env("OPENROUTER_API_KEY".to_string());
    let env = env_map(&[("OPENROUTER_API_KEY", "sk-from-env")]);
    let secret = source.resolve("api", &env).expect("the variable is set");
    assert_eq!(secret.expose(), "sk-from-env");
}

#[test]
fn a_missing_env_credential_is_an_error() {
    // An absent variable is a `Credential` error, not an empty secret.
    let source = CredentialSource::Env("OPENROUTER_API_KEY".to_string());
    let env = env_map(&[]);
    match source.resolve("api", &env) {
        Err(rho_config::ConfigError::Credential { name, .. }) => assert_eq!(name, "api"),
        other => panic!("a missing variable must be a Credential error, got {other:?}"),
    }
}

#[test]
fn an_interpolated_credential_fills_a_span() {
    // A `${NAME}` span is filled from the environment.
    let source = CredentialSource::Interpolate("Bearer ${TOKEN}".to_string());
    let env = env_map(&[("TOKEN", "xyz")]);
    let secret = source.resolve("api", &env).expect("the span is filled");
    assert_eq!(secret.expose(), "Bearer xyz");
}

#[test]
fn a_command_credential_reads_the_command_output() {
    // A stub command supplies the value. The trimmed standard output is the credential.
    let path = std::env::var("PATH").unwrap_or_default();
    let source = CredentialSource::Command {
        argv: vec![
            "sh".to_string(),
            "-c".to_string(),
            "printf 'sk-from-command'".to_string(),
        ],
        pass_env: vec![],
    };
    let env = env_map(&[("PATH", path.as_str())]);
    let secret = source.resolve("api", &env).expect("the command runs");
    assert_eq!(secret.expose(), "sk-from-command");
}

#[test]
fn a_command_child_inherits_only_the_allowlist() {
    // A secret-named variable is absent unless `pass_env` names it. The command echoes
    // the variable, so an empty output proves the child never saw it.
    let path = std::env::var("PATH").unwrap_or_default();

    // Without an allowlist entry, the secret-named variable must not reach the child.
    let denied = CredentialSource::Command {
        argv: vec![
            "sh".to_string(),
            "-c".to_string(),
            "printf '%s' \"${SECRET_TOKEN}\"".to_string(),
        ],
        pass_env: vec![],
    };
    let env = env_map(&[("PATH", path.as_str()), ("SECRET_TOKEN", "leak")]);
    let secret = denied.resolve("api", &env).expect("the command runs");
    assert!(
        !secret.expose().contains("leak"),
        "a secret-named variable must not reach the child without pass_env"
    );

    // With the allowlist entry, the same variable reaches the child.
    let allowed = CredentialSource::Command {
        argv: vec![
            "sh".to_string(),
            "-c".to_string(),
            "printf '%s' \"${SECRET_TOKEN}\"".to_string(),
        ],
        pass_env: vec!["SECRET_TOKEN".to_string()],
    };
    let secret = allowed.resolve("api", &env).expect("the command runs");
    assert_eq!(
        secret.expose(),
        "leak",
        "pass_env must let the named variable through"
    );

    // A variable the real process holds, that the allowlist never names, must not
    // reach the child. D-credential-command-allowlist says the child clears its environment and inherits only
    // PATH, HOME, and the pass_env names. This is the assertion the old test lacked:
    // it looked only for SECRET_TOKEN, which lives in the in-memory map and never in
    // the real process environment, so an implementation that forwarded the whole
    // process environment to the child passed against the very bug it should catch.
    let probe = ["TERM", "SHELL", "LANG", "USER"]
        .into_iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.is_empty())
                .map(|value| (name, value))
        });
    let Some((probe_name, probe_value)) = probe else {
        // A bare CI environment may hold none of these. Skip cleanly rather than fail
        // for a reason that has nothing to do with the allowlist.
        eprintln!("skipping the real-environment leak check: no probe variable is set");
        return;
    };
    // The in-memory environment does not name the probe variable, and pass_env does
    // not either. Only a child that inherited the real process environment could see
    // it. A correct child cannot.
    let leaks_real_env = CredentialSource::Command {
        argv: vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("printf '%s' \"${{{probe_name}}}\""),
        ],
        pass_env: vec![],
    };
    let env = env_map(&[("PATH", path.as_str())]);
    let secret = leaks_real_env
        .resolve("api", &env)
        .expect("the command runs");
    assert!(
        !secret.expose().contains(&probe_value),
        "the child saw a real-environment variable ({probe_name}) the allowlist never \
         named: the implementation must clear the environment and pass only PATH, HOME, \
         and pass_env"
    );
}
