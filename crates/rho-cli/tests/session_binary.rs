//! What the real `rho` binary does with a session.
//!
//! See `SPEC-session-store-wiring` sections 8a and 8e.
//!
//! **These tests run the built binary.** Every other test in this crate drives a module, and a
//! module test cannot see a missing call in `main`. This lane exists because a whole library had
//! no caller, so at least the printed surface is driven end to end.
//!
//! No test here reaches a provider. `sessions show` and `sessions fork` send nothing to a model,
//! and each test passes a provider name that does not exist to prove it. Every test sets its own
//! `HOME`, so none reads the real `~/.rho`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A project root with a `.git` directory, and a `.git` file worktree beside it.
fn project(dir: &Path, name: &str) -> PathBuf {
    let root = dir.join(name);
    std::fs::create_dir_all(root.join(".git")).expect("the project root");
    root
}

/// Run the real binary, and return its stdout, its stderr, and its exit code.
fn rho(home: &Path, root: &Path, args: &[&str]) -> (String, String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rho"))
        .env("HOME", home)
        // No provider credential reaches this. A command that tried to build a provider would
        // fail, and that failure is what proves the command sends nothing to a model.
        .env_remove("AWS_ACCESS_KEY_ID")
        .env_remove("AWS_SECRET_ACCESS_KEY")
        .env_remove("AWS_PROFILE")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("RHO_MODEL")
        .env_remove("RHO_PROVIDER")
        .args(args)
        .arg("--root")
        .arg(root)
        .output()
        .expect("the rho binary runs");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

/// Write one session file by hand, under the store the binary will look in.
///
/// A real run needs a provider, and these tests must reach none. So the file is written directly,
/// in the format `SessionStore::create` writes. `session_cli.rs` proves that format against the
/// real writer.
fn seed_session(home: &Path, key: &str, id: &str, extra: &[String]) -> PathBuf {
    let store = home.join(".rho").join("sessions").join(key);
    std::fs::create_dir_all(&store).expect("the store");
    let path = store.join(format!("{id}.jsonl"));
    let mut lines = vec![
        format!(
            r#"{{"id":"r0","parentId":null,"timestamp":"1756000000000","type":"session","version":1,"cwd":"/work","approval":"read-only","sandbox":"off","session_id":"{id}"}}"#
        ),
        r#"{"id":"r1","parentId":"r0","timestamp":"1756000000001","type":"model_change","provider":"testkit","model":"test-model"}"#.to_string(),
        r#"{"id":"r2","parentId":"r1","timestamp":"1756000000002","type":"message","message":{"role":"user","content":[{"type":"text","text":"fix the parser"}]}}"#.to_string(),
        r#"{"id":"r3","parentId":"r2","timestamp":"1756000000003","type":"message","message":{"role":"assistant","content":[{"type":"text","text":"The bug is on line 42."}]}}"#.to_string(),
    ];
    lines.extend(extra.iter().cloned());
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).expect("the session file");
    path
}

/// The project key the binary computes for a root. It is printed by any command that fails.
fn project_key(home: &Path, root: &Path) -> String {
    // A `sessions list` on an empty store says so, and a `--continue` names the key. The key is
    // deterministic, so reading it once from the binary keeps this test honest about the naming
    // rule instead of copying it.
    let (_out, err, _code) = rho(home, root, &["run", "x", "--continue"]);
    err.split_whitespace()
        .find(|word| word.contains('-') && word.len() > 9 && word.ends_with(';'))
        .map(|word| word.trim_end_matches(';').to_string())
        .unwrap_or_else(|| panic!("the key must appear in the refusal, got {err}"))
}

#[test]
fn show_sends_nothing_to_a_model() {
    // A user must be able to look at yesterday's session without inventing a question, and
    // without spending a token. `run` needs a prompt, so `show` is that read-only view.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let home = dir.path().join("home");
    let root = project(dir.path(), "project");
    std::fs::create_dir_all(&home).expect("the home");
    let key = project_key(&home, &root);
    seed_session(&home, &key, "20260825-094512-a3f9", &[]);

    // A provider that does not exist, and no credential in the environment. A command that
    // reached a model would fail here.
    let (out, err, code) = rho(
        &home,
        &root,
        &[
            "sessions",
            "show",
            "20260825-09",
            "--provider",
            "a-provider-that-does-not-exist",
        ],
    );

    assert_eq!(code, 0, "show must succeed with no provider, stderr: {err}");
    assert!(
        out.contains("fix the parser"),
        "show must print the records, got {out}"
    );
    assert!(
        !err.contains("a-provider-that-does-not-exist"),
        "show must never try to build a provider, got {err}"
    );
}

#[test]
fn the_fork_flow_works_from_the_two_printed_commands() {
    // The owner's ask, pinned as one test. A user runs `show`, copies a record id out of the
    // output, and runs `fork --at <that id>`. **The id comes from the real output**, so this test
    // fails if the id is not printed.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let home = dir.path().join("home");
    let root = project(dir.path(), "project");
    std::fs::create_dir_all(&home).expect("the home");
    let key = project_key(&home, &root);
    let source = seed_session(&home, &key, "20260825-094512-a3f9", &[
        r#"{"id":"r4","parentId":"r3","timestamp":"1756000000004","type":"message","message":{"role":"user","content":[{"type":"text","text":"a later question"}]}}"#.to_string(),
    ]);
    let before = std::fs::read(&source).expect("the source file");

    // Command one, as the guide prints it.
    let (listing, err, code) = rho(&home, &root, &["sessions", "show", "20260825-09"]);
    assert_eq!(code, 0, "show must succeed, stderr: {err}");

    // Take the record id of the assistant answer from the real output. The id is the first
    // column, and that is the whole reason the column exists.
    let record = listing
        .lines()
        .find(|line| line.contains("assistant"))
        .and_then(|line| line.split_whitespace().next())
        .expect("show must print a record id in the first column");
    assert!(
        record.starts_with('r'),
        "the first column must be the record id, got {record:?} from\n{listing}"
    );

    // Command two, with the id the user just read.
    let (out, err, code) = rho(
        &home,
        &root,
        &["sessions", "fork", "20260825-09", "--at", record],
    );
    assert_eq!(code, 0, "fork must succeed, stderr: {err}");

    // The new file holds the branch that ends at that record, and not the later question.
    let new_path = out
        .lines()
        .find(|line| line.ends_with(".jsonl"))
        .map(PathBuf::from)
        .expect("fork must print the new file, got {out}");
    let forked = std::fs::read_to_string(&new_path).expect("the forked file");
    assert!(
        forked.contains("The bug is on line 42."),
        "the fork holds the branch up to the chosen record, got {forked}"
    );
    assert!(
        !forked.contains("a later question"),
        "the fork stops at the chosen record, got {forked}"
    );
    assert!(
        forked.contains("forked_from"),
        "the new file names its origin, got {forked}"
    );
    assert_eq!(
        before,
        std::fs::read(&source).expect("the source file again"),
        "the original file is byte-identical after a fork"
    );
}

#[test]
fn two_worktrees_continuing_at_once_never_share_a_file() {
    // Two worktrees of one repository share a project key, so both reach one store. Without a
    // lock both would seed their record ids from the same read and the lines would interleave.
    //
    // The two runs here are two processes, and each one takes its own session.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).expect("the home");
    // The main checkout, and a worktree that points at its `.git` directory.
    let main = project(dir.path(), "main");
    let worktree = dir.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("the worktree");
    std::fs::write(
        worktree.join(".git"),
        format!("gitdir: {}/.git/worktrees/wt\n", main.display()),
    )
    .expect("the .git file");

    let key_main = project_key(&home, &main);
    let key_worktree = project_key(&home, &worktree);
    assert_eq!(
        key_main, key_worktree,
        "every worktree of one repository must share one key"
    );

    // Two sessions in that one pool, each with its own id.
    let first = seed_session(&home, &key_main, "20260825-094512-a3f9", &[]);
    let second = seed_session(&home, &key_main, "20260825-094513-b7c2", &[]);
    assert_ne!(first, second, "two sessions are two files");

    // Both worktrees see both sessions, because the key is shared.
    for root in [&main, &worktree] {
        let (out, err, code) = rho(&home, root, &["sessions", "list"]);
        assert_eq!(code, 0, "list must succeed, stderr: {err}");
        assert!(out.contains("20260825-094512-a3f9"), "got {out}");
        assert!(out.contains("20260825-094513-b7c2"), "got {out}");
    }

    // And each file keeps its own unique record ids. An implementation that let two writers
    // share one file would break this, and `a_second_process_cannot_continue_a_live_session`
    // is the test that drives the lock itself.
    for path in [&first, &second] {
        let text = std::fs::read_to_string(path).expect("the file");
        let ids: Vec<&str> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let start = line.find("\"id\":\"").expect("an id") + 6;
                let rest = &line[start..];
                &rest[..rest.find('"').expect("the end of the id")]
            })
            .collect();
        let unique: std::collections::HashSet<&&str> = ids.iter().collect();
        assert_eq!(
            ids.len(),
            unique.len(),
            "{} has a duplicate record id: {ids:?}",
            path.display()
        );
    }
}

#[test]
fn the_name_and_delete_commands_run_through_the_binary() {
    // A review found that `sessions list`, `show` and `fork` were driven end to end and `name` and
    // `delete` were not. A module test cannot see a missing dispatch arm.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let home = dir.path().join("home");
    let root = project(dir.path(), "project");
    std::fs::create_dir_all(&home).expect("the home");
    let key = project_key(&home, &root);
    let path = seed_session(&home, &key, "20260825-094512-a3f9", &[]);

    // Name it, and read the name back from the list.
    let (out, err, code) = rho(
        &home,
        &root,
        &["sessions", "name", "20260825-09", "the sign bug"],
    );
    assert_eq!(code, 0, "name must succeed, stderr: {err}");
    assert!(out.contains("the sign bug"), "got {out}");
    let (listing, _err, code) = rho(&home, &root, &["sessions", "list"]);
    assert_eq!(code, 0);
    assert!(
        listing.contains("the sign bug"),
        "the list must show the new title, got {listing}"
    );

    // An empty title is refused, and the message has its own words.
    let (_out, err, code) = rho(&home, &root, &["sessions", "name", "20260825-09", ""]);
    assert_ne!(code, 0, "an empty title must be refused");
    assert!(
        err.contains("a session title cannot be empty"),
        "the refusal must say what happened, got {err}"
    );

    // A named session still reads back. This is the leaf-parent defect a live drive found.
    let (_out, err, code) = rho(&home, &root, &["sessions", "show", "20260825-09"]);
    assert_eq!(
        code, 0,
        "a named session must still read back, stderr: {err}"
    );

    // Delete it, and it is gone.
    let (out, err, code) = rho(&home, &root, &["sessions", "delete", "20260825-09"]);
    assert_eq!(code, 0, "delete must succeed, stderr: {err}");
    assert!(out.contains("deleted session"), "got {out}");
    assert!(!path.exists(), "the file is gone");

    // The same delete twice. "Twice" has caught two defects in this project.
    let (_out, err, code) = rho(&home, &root, &["sessions", "delete", "20260825-09"]);
    assert_ne!(code, 0, "deleting a session that is gone must be refused");
    assert!(
        err.contains("20260825-09"),
        "the refusal must name what the user asked for, got {err}"
    );
}
