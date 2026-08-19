//! Live confinement tests for the `bash` tool. See `SPEC-bash-sandbox`.
//!
//! These tests need a real OS sandbox backend. When none is present, each test
//! prints that it skipped and returns. A test that silently passes when it did
//! nothing is the failure mode decision D-bash-line-cap records, so the skip is loud.
//!
//! Every test asserts the effect, not an error message. A write that must fail is
//! checked by the absence of the file, because an error string is easy to match
//! by accident.

mod common;

use common::Harness;
use rho_core::{CommandRunner, SandboxMode, Tool};
use rho_tools::BashTool;

fn text_of(output: &rho_core::ToolOutput) -> String {
    output
        .content
        .iter()
        .filter_map(|b| match b {
            rho_core::ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// True when a real backend exists. When false, a test prints a skip and returns.
fn sandbox_ready(test: &str) -> bool {
    if rho_tools::detect_backend().is_some() {
        return true;
    }
    println!("SKIP {test}: no OS sandbox backend on this host");
    false
}

#[tokio::test]
async fn sandbox_off_runs_a_command_unchanged() {
    // `Off` needs no backend, so this never skips.
    let mut h = Harness::new();
    let out = BashTool::new()
        .sandbox(SandboxMode::Off)
        .execute(serde_json::json!({ "command": "echo hello-off" }), h.ctx())
        .await
        .unwrap();
    assert!(!out.is_error, "output: {}", text_of(&out));
    assert!(text_of(&out).contains("hello-off"));
}

#[tokio::test]
async fn confined_allows_a_write_inside_the_session_root() {
    if !sandbox_ready("confined_allows_a_write_inside_the_session_root") {
        return;
    }
    let mut h = Harness::new();
    let out = BashTool::new()
        .sandbox(SandboxMode::Confined)
        .execute(
            serde_json::json!({ "command": "echo hi > inside.txt" }),
            h.ctx(),
        )
        .await
        .unwrap();
    assert!(
        !out.is_error,
        "a write inside the root must succeed: {}",
        text_of(&out)
    );
    assert!(
        h.root().join("inside.txt").exists(),
        "the file inside the root must exist"
    );
}

#[tokio::test]
async fn confined_refuses_a_write_outside_the_session_root() {
    if !sandbox_ready("confined_refuses_a_write_outside_the_session_root") {
        return;
    }
    let mut h = Harness::new();
    // A unique name under the real home. The assertion is the absence of the file,
    // not the error text.
    let home = std::env::var("HOME").expect("a test host has HOME");
    let probe = format!("{home}/rho-sandbox-probe-{}", std::process::id());
    let _ = std::fs::remove_file(&probe);
    let _ = BashTool::new()
        .sandbox(SandboxMode::Confined)
        .execute(
            serde_json::json!({ "command": format!("echo leak > {probe}") }),
            h.ctx(),
        )
        .await
        .unwrap();
    let exists = std::path::Path::new(&probe).exists();
    let _ = std::fs::remove_file(&probe);
    assert!(!exists, "a write outside the root must not create the file");
}

#[tokio::test]
async fn confined_refuses_a_write_even_after_cd() {
    if !sandbox_ready("confined_refuses_a_write_even_after_cd") {
        return;
    }
    // The case a pattern list misses. `cd` leaves the session root, then a write
    // to a writable directory outside the root must still be refused. A path check
    // could not stop this. The target is under the real home, which is writable, so
    // only the sandbox can stop the write.
    let home = std::env::var("HOME").expect("a test host has HOME");
    let probe = format!("{home}/rho-probe-cd-{}", std::process::id());
    let _ = std::fs::remove_file(&probe);
    let mut h = Harness::new();
    let _ = BashTool::new()
        .sandbox(SandboxMode::Confined)
        .execute(
            serde_json::json!({ "command": format!("cd {home} && touch rho-probe-cd-{}", std::process::id()) }),
            h.ctx(),
        )
        .await
        .unwrap();
    let exists = std::path::Path::new(&probe).exists();
    let _ = std::fs::remove_file(&probe);
    assert!(
        !exists,
        "a write after cd out of the root must not create the file"
    );
}

#[tokio::test]
async fn confined_refuses_an_absolute_path_write() {
    if !sandbox_ready("confined_refuses_an_absolute_path_write") {
        return;
    }
    let home = std::env::var("HOME").expect("a test host has HOME");
    let probe = format!("{home}/rho-abs-probe-{}", std::process::id());
    let _ = std::fs::remove_file(&probe);
    let mut h = Harness::new();
    let _ = BashTool::new()
        .sandbox(SandboxMode::Confined)
        .execute(
            serde_json::json!({ "command": format!("touch {probe}") }),
            h.ctx(),
        )
        .await
        .unwrap();
    let exists = std::path::Path::new(&probe).exists();
    let _ = std::fs::remove_file(&probe);
    assert!(
        !exists,
        "an absolute path write outside the root must be refused"
    );
}

#[tokio::test]
async fn confined_still_allows_reading_a_system_file() {
    if !sandbox_ready("confined_still_allows_reading_a_system_file") {
        return;
    }
    // A toolchain lives outside the root, so a read must work or the mode is
    // useless. `/etc/hosts` exists on every Unix host.
    let mut h = Harness::new();
    let out = BashTool::new()
        .sandbox(SandboxMode::Confined)
        .execute(serde_json::json!({ "command": "cat /etc/hosts" }), h.ctx())
        .await
        .unwrap();
    assert!(
        !out.is_error,
        "a read of a system file must succeed: {}",
        text_of(&out)
    );
}

#[tokio::test]
async fn strict_refuses_a_network_call() {
    if !sandbox_ready("strict_refuses_a_network_call") {
        return;
    }
    // Discriminate the network boundary with a local listener, so no external
    // network is used. `confined` connects; `strict` cannot. The target is our own
    // socket, so a success can only mean the connection went through.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        // Accept a few connections, then stop. A dropped stream closes at once.
        for _ in 0..8 {
            if listener.accept().is_err() {
                break;
            }
        }
    });
    let command = format!("exec 3<>/dev/tcp/127.0.0.1/{port}");

    let mut h = Harness::new();
    let confined = BashTool::new()
        .sandbox(SandboxMode::Confined)
        .execute(serde_json::json!({ "command": command.clone() }), h.ctx())
        .await
        .unwrap();
    assert!(
        !confined.is_error,
        "confined must reach a local socket: {}",
        text_of(&confined)
    );

    let mut h2 = Harness::new();
    let strict = BashTool::new()
        .sandbox(SandboxMode::Strict)
        .execute(serde_json::json!({ "command": command }), h2.ctx())
        .await
        .unwrap();
    assert!(
        strict.is_error,
        "strict must refuse a network call: {}",
        text_of(&strict)
    );
}

#[tokio::test]
async fn a_gate_command_obeys_the_parent_sandbox() {
    // `SPEC-agent-tasks` and decision D-an-acceptance-check-has-a-trusted-author both
    // say a gate check runs under the parent's confinement. The spec named this test
    // and the test did not exist, so the claim was unproven. It is proven here.
    //
    // The gate runner reuses the same `build_command` path as `bash`, so a check
    // cannot escape through a second, drifting code path.
    if !sandbox_ready("a_gate_command_obeys_the_parent_sandbox") {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let outside = std::env::temp_dir().join("rho-gate-escape.txt");
    let _ = std::fs::remove_file(&outside);

    let runner = rho_tools::SandboxedRunner::new(SandboxMode::Confined);
    let cancel = rho_core::CancelToken::new();

    // A write inside the root is allowed, so the runner really runs a command.
    let inside = runner
        .run("echo hi > inside.txt", root.path(), &cancel)
        .await
        .expect("a confined command still runs");
    assert_eq!(inside, 0, "a write inside the root must succeed");
    assert!(
        root.path().join("inside.txt").exists(),
        "the command must really have run"
    );

    // A write outside the root must fail, so a check cannot escape confinement.
    let escaped = runner
        .run(
            &format!("echo pwned > {}", outside.display()),
            root.path(),
            &cancel,
        )
        .await
        .expect("the command runs and reports its own exit code");
    assert_ne!(
        escaped, 0,
        "a gate command must not write outside the session root"
    );
    assert!(
        !outside.exists(),
        "nothing must appear outside the root, found {}",
        outside.display()
    );
    let _ = std::fs::remove_file(&outside);
}

#[tokio::test]
async fn a_cancelled_gate_command_stops() {
    // A gate check must stop with its parent, or a cancelled run leaves a test
    // suite running.
    let root = tempfile::tempdir().unwrap();
    let runner = rho_tools::SandboxedRunner::new(SandboxMode::Off);
    let cancel = rho_core::CancelToken::new();
    cancel.cancel();

    let result = runner.run("sleep 30", root.path(), &cancel).await;
    assert!(
        result.is_err(),
        "a cancelled gate check must not report a passing exit code"
    );
}
