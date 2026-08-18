//! Credential resolution from `SPEC-13` section 5 and section 7.
//!
//! A credential source resolves to a `Secret`. A `Literal` is a `Secret` from the
//! moment it is parsed. A command source runs a program and inherits only an
//! allowlist, per decision D-046. Every test uses an in-memory environment.

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
}
