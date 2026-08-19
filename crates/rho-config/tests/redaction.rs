//! The redaction invariant from `SPEC-config` section 5 and section 7.
//!
//! A resolved credential is a `Secret`, and `Secret` redacts by construction. These
//! tests assert the invariant, not one example field: no formatted output and no log
//! line holds the credential text. This satisfies F-no-secrets-in-logs and answers the sprint-1
//! "credential in a log" defect.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use common::env_map;
use rho_config::{ApprovalMode, Config, CredentialSource};
use rho_core::{SandboxMode, SubagentLimits};

/// The unique credential text. It must appear in no formatted value and no log line.
const SECRET_TEXT: &str = "sk-live-topsecret-value";

/// Build a `Config` that holds one parsed credential.
fn config_holding(source: CredentialSource) -> Config {
    let mut credentials = BTreeMap::new();
    credentials.insert("api".to_string(), source);
    Config {
        provider: Some("openrouter".to_string()),
        model: Some("some-model".to_string()),
        session_root: None,
        session_file: None,
        ephemeral: false,
        sandbox: SandboxMode::Off,
        approval: Some(ApprovalMode::ReadOnly),
        skill_paths: Vec::new(),
        discover_skills: true,
        tui_mouse: false,
        reasoning: rho_core::ReasoningDisplay::Summary,
        mcp_config: None,
        subagents: SubagentLimits::default(),
        credentials,
    }
}

#[test]
fn a_resolved_credential_never_appears_in_a_formatted_value() {
    // The `Debug` of a `Config` that holds a literal credential prints `Secret(***)`
    // and never the value. This tests every public type that can hold a credential,
    // through the one `Config` that owns them all.
    let source = CredentialSource::parse(SECRET_TEXT);
    let config = config_holding(source);

    let debug = format!("{config:?}");
    assert!(
        !debug.contains(SECRET_TEXT),
        "the Debug of a Config must never contain the credential text"
    );
    assert!(
        debug.contains("Secret(***)"),
        "the credential must render as the fixed mask"
    );

    // The invariant also holds for the source itself, not only inside a Config.
    let source = CredentialSource::parse(SECRET_TEXT);
    let source_debug = format!("{source:?}");
    assert!(
        !source_debug.contains(SECRET_TEXT),
        "the Debug of a CredentialSource must never contain the credential text"
    );
}

/// A writer that collects every log byte into a shared buffer.
#[derive(Clone)]
struct BufferWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for BufferWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufferWriter {
    type Writer = BufferWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn a_resolved_credential_never_reaches_a_log() {
    // A resolution at `trace` level writes no credential to the subscriber. Redaction
    // is by construction: no code in `rho-config` writes a credential to `tracing`.
    let buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
    let writer = BufferWriter(Arc::clone(&buffer));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_max_level(tracing::Level::TRACE)
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        let env = env_map(&[("CRED_VAR", SECRET_TEXT)]);

        // The literal source, kept from the original test.
        let literal = CredentialSource::parse(SECRET_TEXT);
        let _ = literal.resolve("api", &env);

        // The environment source reads the resolved value; it is a path that would log.
        let from_env = CredentialSource::Env("CRED_VAR".to_string());
        let _ = from_env.resolve("api", &env);

        // The command source runs a child and reads its output. It is the most likely
        // path to log a resolved credential, so it must be exercised under the
        // subscriber too.
        let path = std::env::var("PATH").unwrap_or_default();
        let cmd_env = env_map(&[("PATH", path.as_str())]);
        let from_command = CredentialSource::Command {
            argv: vec![
                "sh".to_string(),
                "-c".to_string(),
                format!("printf '%s' '{SECRET_TEXT}'"),
            ],
            pass_env: vec![],
        };
        let _ = from_command.resolve("api", &cmd_env);

        // Prove the capture works before trusting an empty result.
        //
        // This test asserts that something is absent. A broken capture would make it pass
        // against any implementation, including one that prints the credential. A sibling
        // capture in `rho-core` was flaky one run in twenty for exactly this reason: with
        // no global subscriber, `tracing` reports the level filter as `OFF`, and a macro
        // takes its fast path. So emit a known line and require it.
        tracing::warn!("capture-probe");
    });

    let logged = String::from_utf8(buffer.lock().unwrap().clone()).expect("utf8 log");
    assert!(
        logged.contains("capture-probe"),
        "the log capture is broken, so this test proves nothing: {logged:?}"
    );
    assert!(
        !logged.contains(SECRET_TEXT),
        "a credential must never reach a log, even at trace level: {logged:?}"
    );
}
