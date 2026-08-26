//! The provider endpoint, and the rule that keeps a credential off the wire.
//!
//! A base url redirects the key. `https` anywhere is allowed, because the transport is
//! encrypted. Plain `http` is allowed only to a loopback literal, where the traffic never
//! leaves the machine. See `D-a-provider-base-url-is-a-config-key`.

mod common;

use common::{project_sources, temp_dir, write_file};
use rho_config::{Config, ProjectTrust};

/// Resolve one base url from a trusted project file.
fn load(url: &str) -> Result<Config, rho_config::ConfigError> {
    let dir = temp_dir();
    let path = write_file(&dir, "p.toml", &format!("base-url = \"{url}\"\n"));
    let sources = project_sources(path).with_project_trust(ProjectTrust::Trusted);
    Config::load(&sources)
}

#[test]
fn an_https_base_url_is_allowed() {
    let config = load("https://models.example.com/v1").expect("https is allowed anywhere");
    assert_eq!(
        config.base_url.as_deref(),
        Some("https://models.example.com/v1")
    );
}

#[test]
fn a_loopback_http_base_url_is_allowed() {
    // How a local model host works. The traffic never leaves the machine.
    for url in [
        "http://localhost:11434/v1",
        "http://127.0.0.1:8080/v1",
        "http://127.0.0.2:8080/v1",
        "http://[::1]:8080/v1",
    ] {
        let config = load(url).unwrap_or_else(|error| panic!("{url} must pass: {error}"));
        assert_eq!(config.base_url.as_deref(), Some(url));
    }
}

#[test]
fn a_plain_http_remote_base_url_is_refused() {
    let error = load("http://models.example.com/v1").expect_err("plaintext to a remote host");
    let text = error.to_string();
    assert!(
        text.contains("base-url"),
        "the refusal names the key: {text}"
    );
}

#[test]
fn a_userinfo_host_cannot_pose_as_loopback() {
    // `http://127.0.0.1@evil.example` has host `evil.example`. A substring check passes it
    // and the key goes to the attacker. A parser is the only honest reader of a url.
    let error = load("http://127.0.0.1@evil.example/v1").expect_err("the host is evil.example");
    assert!(error.to_string().contains("base-url"));
}

#[test]
fn a_look_alike_host_cannot_pose_as_loopback() {
    for url in [
        "http://localhost.evil.example/v1",
        "http://127.0.0.1.evil.example/v1",
    ] {
        load(url).unwrap_err();
    }
}

#[test]
fn an_encoded_loopback_address_is_read_by_the_parser() {
    // These are the forms that defeat a string check: decimal, octal, hex, and short. A
    // parser normalises every one to 127.0.0.1, so each really is loopback and each is
    // allowed. A `contains("127.0.0.1")` check would have refused all four while passing
    // `http://127.0.0.1@evil.example`, which is the exact inversion of what matters.
    for url in [
        "http://2130706433/v1",
        "http://0177.0.0.1/v1",
        "http://0x7f.0.0.1/v1",
        "http://127.1/v1",
    ] {
        let config = load(url).unwrap_or_else(|error| panic!("{url} is loopback: {error}"));
        assert_eq!(config.base_url.as_deref(), Some(url));
    }
}

#[test]
fn a_trailing_dot_localhost_is_refused() {
    // `localhost.` is the absolute form, and rho compares the exact name. Refusing is the
    // strict side of the choice, and a user drops the dot. A wrong allow would be a host
    // rho never checked.
    load("http://localhost./v1").unwrap_err();
}

#[test]
fn the_unspecified_address_is_refused() {
    // `0.0.0.0` is every interface, not a loopback destination.
    load("http://0.0.0.0:8080/v1").unwrap_err();
    load("http://[::]:8080/v1").unwrap_err();
}

#[test]
fn a_url_with_no_scheme_is_refused() {
    load("models.example.com/v1").unwrap_err();
    load("//models.example.com/v1").unwrap_err();
}

#[test]
fn a_base_url_with_a_query_or_fragment_is_refused() {
    // The endpoint is built by appending a path, so a query would land mid-url. A review
    // printed `https://host/v1?token=leak/v1/chat/completions`, which is a request to a place
    // the user never named.
    load("https://host.example/v1?api-version=2024").unwrap_err();
    load("https://host.example/v1#frag").unwrap_err();
}
