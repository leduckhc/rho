//! The credential command timeout from `SPEC-13` section 5.
//!
//! Section 5 says the child has a timeout, and a hung helper fails the resolution.
//! This test uses a child that never exits on its own (`tail -f /dev/null`), so a
//! returned result proves the timeout killed the child rather than the child exiting.
//! The test uses no `sleep` and does not depend on wall-clock luck.

mod common;

use std::time::{Duration, Instant};

use common::env_map;
use rho_config::{ConfigError, CredentialSource};

#[test]
fn a_hung_credential_command_times_out() {
    let path = std::env::var("PATH").unwrap_or_default();
    // `tail -f /dev/null` blocks forever and never exits on its own. So the only way
    // the call can return is the timeout killing the child.
    let source = CredentialSource::Command {
        argv: vec![
            "sh".to_string(),
            "-c".to_string(),
            "tail -f /dev/null".to_string(),
        ],
        pass_env: vec![],
    };
    let env = env_map(&[("PATH", path.as_str())]);

    let start = Instant::now();
    let result = source.resolve_with_timeout("op-secret", &env, Duration::from_secs(1));
    let elapsed = start.elapsed();

    match result {
        Err(ConfigError::Credential { name, message }) => {
            assert_eq!(name, "op-secret", "the error names the credential");
            assert!(
                message.contains("timed out"),
                "the message must say the helper timed out, got: {message}"
            );
        }
        other => panic!("a hung helper must fail with a Credential error, got {other:?}"),
    }
    // The child never exits on its own, so a return under the child's lifetime proves
    // the timeout did the work. A generous bound keeps the test stable under load.
    assert!(
        elapsed < Duration::from_secs(10),
        "the call must return well under the child's lifetime, took {elapsed:?}"
    );
}
