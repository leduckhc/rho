//! What the command line does with a session.
//!
//! See `SPEC-session-store-wiring` sections 8, 8c, and 9.
//!
//! Every test isolates the filesystem with `tempfile` and passes its own `home`, so no test
//! reads the real `~/.rho`. No test sleeps, and no test touches the network.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use clap::Parser;

#[path = "../src/recording.rs"]
mod recording;

use recording::{RecordingRequest, SessionSelector};
use rho_core::{
    ContentBlock, Message, Record, Role, SessionError, SessionId, SessionReader, SessionRow, Usage,
};

/// The one session flag, exactly as `rho run` declares it.
///
/// The parser is duplicated here on purpose. `rho-cli` has no library target, so a test cannot
/// build the real `Cli`. The attributes are the contract, and
/// `the_run_command_declares_the_session_flag_with_require_equals` proves this copy matches the
/// real one by reading `cli.rs`.
#[derive(Debug, Parser)]
#[command(name = "rho")]
struct RunOnly {
    prompt: String,
    #[arg(
        long = "continue",
        short = 'c',
        alias = "resume",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "",
        value_name = "ID"
    )]
    continue_session: Option<String>,
    #[arg(long)]
    ephemeral: bool,
    #[arg(long, requires = "continue_session")]
    allow_widen: bool,
}

fn parse(args: &[&str]) -> Result<RunOnly, clap::Error> {
    RunOnly::try_parse_from(std::iter::once("rho").chain(args.iter().copied()))
}

/// A project root with a `.git` directory, so the project key is stable.
fn project(dir: &Path) -> PathBuf {
    let root = dir.join("project");
    std::fs::create_dir_all(root.join(".git")).expect("the project root");
    root
}

/// The request a test uses. Every input is a parameter.
fn request<'a>(home: &'a Path, root: &'a Path, selector: SessionSelector) -> RecordingRequest<'a> {
    RecordingRequest {
        project_root: root,
        home,
        session_file: None,
        ephemeral: false,
        selector,
        allow_widen: false,
        approval: "read-only",
        sandbox: "off",
        provider: "testkit",
        model: "test-model",
        now_millis: 1_756_000_000_000,
    }
}

/// The store a run would use for this project.
fn rows(home: &Path, root: &Path) -> Vec<SessionRow> {
    let (store, _key) = recording::store_for(home, root);
    store.rows().expect("the rows build")
}

// ---------------------------------------------------------------------------
// Section 8. One flag, two spellings, an optional value.
// ---------------------------------------------------------------------------

#[test]
fn resume_is_an_alias_for_continue() {
    // One argument, two spellings, so they can never disagree.
    let bare_continue = parse(&["--continue", "a prompt"]).expect("parses");
    let bare_resume = parse(&["--resume", "a prompt"]).expect("parses");
    let named_continue = parse(&["--continue=20260825-09", "a prompt"]).expect("parses");
    let named_resume = parse(&["--resume=20260825-09", "a prompt"]).expect("parses");

    assert_eq!(
        SessionSelector::from_flag(bare_continue.continue_session.as_deref()),
        SessionSelector::from_flag(bare_resume.continue_session.as_deref())
    );
    assert_eq!(
        SessionSelector::from_flag(named_continue.continue_session.as_deref()),
        SessionSelector::from_flag(named_resume.continue_session.as_deref())
    );
}

#[test]
fn a_bare_flag_takes_the_newest_session() {
    let parsed = parse(&["--continue", "a prompt"]).expect("parses");

    assert_eq!(
        SessionSelector::from_flag(parsed.continue_session.as_deref()),
        SessionSelector::Newest
    );
    assert_eq!(parsed.prompt, "a prompt");
}

#[test]
fn a_flag_with_a_value_takes_that_session() {
    let parsed = parse(&["--resume=20260825-094512-a3f9", "a prompt"]).expect("parses");

    assert_eq!(
        SessionSelector::from_flag(parsed.continue_session.as_deref()),
        SessionSelector::Named("20260825-094512-a3f9".to_string())
    );
    assert_eq!(parsed.prompt, "a prompt");
}

#[test]
fn a_short_flag_with_a_value_takes_that_session() {
    let parsed = parse(&["-c=20260825-094512-a3f9", "a prompt"]).expect("parses");

    assert_eq!(
        SessionSelector::from_flag(parsed.continue_session.as_deref()),
        SessionSelector::Named("20260825-094512-a3f9".to_string())
    );
    assert_eq!(parsed.prompt, "a prompt");
}

#[test]
fn no_flag_starts_a_new_session() {
    let parsed = parse(&["a prompt"]).expect("parses");

    assert_eq!(
        SessionSelector::from_flag(parsed.continue_session.as_deref()),
        SessionSelector::New
    );
}

#[test]
fn the_flag_does_not_swallow_the_prompt() {
    // Without `require_equals` clap takes the prompt as the flag's value, and then the prompt is
    // missing. That was measured against clap 4, not guessed. See section 8c.
    let parsed = parse(&["--continue", "fix the bug"]).expect("the prompt must survive");

    assert_eq!(parsed.prompt, "fix the bug");
    assert_eq!(
        SessionSelector::from_flag(parsed.continue_session.as_deref()),
        SessionSelector::Newest
    );
}

#[test]
fn the_session_flag_given_twice_is_an_error() {
    // The alias makes this refusal free. Two spellings of one argument used twice is a clap
    // error, so no custom conflict rule exists to get wrong.
    parse(&["--continue", "--resume=20260825-09", "a prompt"])
        .expect_err("one argument used twice must be refused");
}

#[test]
fn a_session_id_as_the_prompt_is_refused_with_a_hint() {
    // Measured as a silent wrong run: a space instead of an equals sign continued the newest
    // session and sent the id to the model as a question.
    let parsed = parse(&["--resume", "20260825-09"]).expect("clap accepts it");
    let selector = SessionSelector::from_flag(parsed.continue_session.as_deref());
    assert_eq!(selector, SessionSelector::Newest, "the flag went bare");
    assert_eq!(parsed.prompt, "20260825-09", "the id became the prompt");

    let error = recording::refuse_an_id_shaped_prompt(&parsed.prompt, &selector)
        .expect_err("rho must refuse it rather than guess");

    let message = error.to_string();
    assert!(
        message.contains("--resume=20260825-09"),
        "the message must name the flag with an equals sign, got {message}"
    );
}

#[test]
fn an_ordinary_prompt_is_not_refused() {
    let parsed = parse(&["--continue", "fix the parser"]).expect("parses");
    let selector = SessionSelector::from_flag(parsed.continue_session.as_deref());

    recording::refuse_an_id_shaped_prompt(&parsed.prompt, &selector)
        .expect("an ordinary prompt must run");
}

#[test]
fn allow_widen_alone_is_an_error() {
    parse(&["--allow-widen", "a prompt"])
        .expect_err("--allow-widen with no session flag must be refused");
    parse(&["--continue", "--allow-widen", "a prompt"]).expect("with a session flag it parses");
}

#[test]
fn the_run_command_declares_the_session_flag_with_require_equals() {
    // The parser above is a copy, so this reads the real declaration. Without it a change to
    // `cli.rs` would leave every flag test passing against a parser nobody ships.
    let source = std::fs::read_to_string("src/cli.rs").expect("the cli source");
    let start = source
        .find("continue_session")
        .expect("the run command must declare the session flag");
    let block = &source[start.saturating_sub(600)..start];

    for needle in [
        "long = \"continue\"",
        "short = 'c'",
        "alias = \"resume\"",
        "num_args = 0..=1",
        "require_equals = true",
        "default_missing_value",
    ] {
        assert!(
            block.contains(needle),
            "the real flag must declare {needle}, so this test file matches what rho ships"
        );
    }
    assert!(
        source.contains("requires = \"continue_session\""),
        "--allow-widen must require a session flag"
    );
}

// ---------------------------------------------------------------------------
// Recording, and the default.
// ---------------------------------------------------------------------------

#[test]
fn a_run_writes_a_session_file_by_default() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());

    let recording = recording::open(request(dir.path(), &root, SessionSelector::New))
        .expect("a new session opens");

    let id = recording.id.as_ref().expect("a new session has an id");
    let path = recording.path.as_ref().expect("a new session has a file");
    assert!(path.exists(), "the run must write a session file");
    assert_eq!(rows(dir.path(), &root).len(), 1);
    assert!(
        path.to_string_lossy().contains(id.as_str()),
        "the file is named for the session"
    );
    // The store sits under the passed home, never the real one.
    assert!(path.starts_with(dir.path()), "got {}", path.display());
}

#[test]
fn the_ephemeral_flag_writes_no_file() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    let mut asked = request(dir.path(), &root, SessionSelector::New);
    asked.ephemeral = true;

    let recording = recording::open(asked).expect("an ephemeral run opens");

    assert!(recording.id.is_none(), "an ephemeral run has no session id");
    assert!(recording.path.is_none(), "an ephemeral run writes no file");
    assert!(recording.recorder.is_ephemeral());
    assert!(rows(dir.path(), &root).is_empty(), "the store stays empty");
}

#[test]
fn the_ephemeral_config_key_writes_no_file() {
    // The key does what the flag does. `rho-config` already carries `ephemeral`, so this lane
    // reads it and changes nothing there.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    let defaults = rho_config::Config::defaults();
    assert_ne!(
        defaults.ephemeral,
        Some(true),
        "the default writes a file, so the key has to be set on purpose"
    );

    // The command line merges the key and the flag into one boolean, and this is that boolean.
    let mut asked = request(dir.path(), &root, SessionSelector::New);
    asked.ephemeral = true;
    let recording = recording::open(asked).expect("an ephemeral run opens");

    assert!(recording.path.is_none());
    assert!(rows(dir.path(), &root).is_empty());
}

#[test]
fn the_session_file_key_overrides_the_store() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    let named = dir.path().join("elsewhere").join("my-session.jsonl");
    let mut asked = request(dir.path(), &root, SessionSelector::New);
    asked.session_file = Some(&named);

    let recording = recording::open(asked).expect("a named file opens");

    assert_eq!(
        recording.path.as_deref(),
        Some(named.as_path()),
        "the key names one exact file"
    );
    assert!(named.exists());
    assert!(
        rows(dir.path(), &root).is_empty(),
        "the store holds nothing, because the key overrode it"
    );
}

#[test]
fn ephemeral_and_continue_together_are_refused() {
    // --ephemeral writes no file, so there is nothing to continue. Choosing one silently would
    // either lose the session or lose the ephemeral promise.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    let mut asked = request(dir.path(), &root, SessionSelector::Newest);
    asked.ephemeral = true;

    recording::open(asked).map(|_| ()).expect_err("refused");
}

#[test]
fn a_write_failure_degrades_and_the_run_finishes() {
    // A session file is not worth ending a run for. See `D-write-failure-degrades`.
    let degraded = recording::Recording::degraded("the disk is full".to_string());

    assert!(degraded.recorder.is_ephemeral());
    assert!(degraded.path.is_none());
    assert_eq!(degraded.notices, vec!["the disk is full".to_string()]);
}

// ---------------------------------------------------------------------------
// Continue, and the trust boundary.
// ---------------------------------------------------------------------------

/// Write one session with a prompt and an answer, and return its id.
fn seed_session(home: &Path, root: &Path, prompt: &str, answer: &str) -> SessionId {
    let mut recording =
        recording::open(request(home, root, SessionSelector::New)).expect("a new session");
    recording.recorder.record_prompt(&[ContentBlock::Text {
        text: prompt.to_string(),
    }]);
    recording.recorder.observe(&rho_core::AgentEvent::TurnStart);
    recording.recorder.observe(&rho_core::AgentEvent::Stream(
        rho_core::StreamEvent::TextStart { index: 0 },
    ));
    recording.recorder.observe(&rho_core::AgentEvent::Stream(
        rho_core::StreamEvent::TextDelta {
            index: 0,
            delta: answer.to_string(),
        },
    ));
    recording.recorder.observe(&rho_core::AgentEvent::TurnEnd {
        stop_reason: rho_core::StopReason::EndTurn,
    });
    recording.id.expect("an id")
}

#[test]
fn continue_takes_the_newest_session_for_the_project() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    let older = seed_session(dir.path(), &root, "the older", "an older answer");
    // A second session, minted one minute later, so the order is deterministic.
    let newer = {
        let mut asked = request(dir.path(), &root, SessionSelector::New);
        asked.now_millis += 60_000;
        let recording = recording::open(asked).expect("a second session");
        recording.id.expect("an id")
    };
    assert_ne!(older, newer);

    let resumed = recording::open(request(dir.path(), &root, SessionSelector::Newest))
        .expect("a resume opens");

    assert_eq!(resumed.id, Some(newer), "the bare flag takes the newer one");
}

#[test]
fn continue_states_which_session_it_took() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    let id = seed_session(dir.path(), &root, "fix the parser", "the answer");

    let resumed = recording::open(request(dir.path(), &root, SessionSelector::Newest))
        .expect("a resume opens");

    let notice = resumed.notices.join(" ");
    assert!(
        notice.contains(id.as_str()),
        "the output must name the session, got {notice}"
    );
    assert!(
        notice.contains(&root.display().to_string()),
        "the output must name the directory, got {notice}"
    );
}

#[test]
fn a_resume_replays_the_earlier_conversation() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    seed_session(dir.path(), &root, "fix the parser", "the bug is on line 42");

    let resumed = recording::open(request(dir.path(), &root, SessionSelector::Newest))
        .expect("a resume opens");

    let text = every_text(&resumed.messages);
    assert!(
        text.contains("fix the parser"),
        "the prompt must come back, got {text:?}"
    );
    assert!(
        text.contains("the bug is on line 42"),
        "the answer must come back, got {text:?}"
    );
    let roles: Vec<Role> = resumed.messages.iter().map(|m| m.role).collect();
    assert!(roles.contains(&Role::Assistant), "got {roles:?}");
}

#[test]
fn continue_with_no_session_is_an_error_that_says_what_to_do() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());

    let error = recording::open(request(dir.path(), &root, SessionSelector::Newest))
        .map(|_| ())
        .expect_err("an empty store cannot continue");

    let message = error.to_string();
    assert!(
        message.contains("no session to continue"),
        "the message must say what happened, got {message}"
    );
    assert!(
        message.contains("without --continue"),
        "the message must say what to do next, got {message}"
    );
}

#[test]
fn an_unknown_prefix_on_the_command_line_names_the_project() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    seed_session(dir.path(), &root, "a session", "an answer");

    let error = recording::open(request(
        dir.path(),
        &root,
        SessionSelector::Named("19700101-00".to_string()),
    ))
    .map(|_| ())
    .expect_err("an unknown prefix must be refused");

    let message = error.to_string();
    assert!(
        message.contains("19700101-00"),
        "the message must name the prefix, got {message}"
    );
}

#[test]
fn a_resume_that_would_widen_is_refused_on_the_command_line() {
    // The session was created read-only. This run asks for allow-all, which is wider.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    seed_session(dir.path(), &root, "a read-only session", "an answer");

    let mut asked = request(dir.path(), &root, SessionSelector::Newest);
    asked.approval = "allow-all";
    let error = recording::open(asked)
        .map(|_| ())
        .expect_err("a widening resume must be refused");

    let message = error.to_string();
    assert!(
        message.contains("--allow-widen"),
        "the message must name the flag that allows it, got {message}"
    );
}

#[test]
fn allow_widen_permits_the_wider_resume_on_the_command_line() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    seed_session(dir.path(), &root, "a read-only session", "an answer");

    let mut asked = request(dir.path(), &root, SessionSelector::Newest);
    asked.approval = "allow-all";
    asked.allow_widen = true;

    recording::open(asked).expect("with the flag the run starts");
}

#[test]
fn a_resume_takes_its_cwd_from_config_not_from_the_file() {
    // A session file is untrusted input. An attacker who can write into the store chooses what
    // rho replays, so a forged `cwd` must change no path the run uses.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    let id = seed_session(dir.path(), &root, "a session", "an answer");
    let (store, _key) = recording::store_for(dir.path(), &root);
    let path = store.path_of(&id);
    let text = std::fs::read_to_string(&path).expect("the file");
    let forged = text.replacen(&root.display().to_string(), "/etc/rho-must-not-use-this", 1);
    std::fs::write(&path, forged).expect("the forged file");

    let resumed = recording::open(request(dir.path(), &root, SessionSelector::Newest))
        .expect("a forged cwd must not stop a resume");

    // The run's own root is what the caller passed, and the notice states the stored value for
    // the reader. No path rho uses comes from the file.
    let resumed_path = resumed.path.expect("a file");
    assert!(
        resumed_path.starts_with(dir.path()),
        "the run writes under the store the caller chose, got {}",
        resumed_path.display()
    );
    assert!(
        !resumed_path.starts_with("/etc"),
        "a forged cwd must never move the file rho writes"
    );
}

#[test]
fn a_forged_header_cannot_widen_a_run() {
    // A header claiming `allow-all` grants nothing the live config withheld. The stored mode may
    // only tighten a run, so a forged header can remove a warning and never add a permission.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    let id = seed_session(dir.path(), &root, "a session", "an answer");
    let (store, _key) = recording::store_for(dir.path(), &root);
    let path = store.path_of(&id);
    let text = std::fs::read_to_string(&path).expect("the file");
    std::fs::write(&path, text.replacen("read-only", "allow-all", 1)).expect("the forged file");

    // A narrower run is always allowed, so the forged header stops nothing.
    {
        let resumed = recording::open(request(dir.path(), &root, SessionSelector::Newest))
            .expect("a narrower run is always allowed");
        assert!(resumed.id.is_some());
        // The lock releases here, so the next open can reach the same session.
    }

    // The header cannot **grant** anything, and that is structural. The mode the run uses comes
    // from the live config, and this reads the real call site to prove it. An assertion that a
    // resume succeeded would pass against a build that took the mode from the file.
    let source = std::fs::read_to_string("src/cli.rs").expect("the cli source");
    let start = source
        .find("fn approval_name")
        .expect("the run path must resolve its own approval name");
    let body = &source[start..start + 500];
    assert!(
        body.contains("loaded.approval"),
        "the approval mode must come from the loaded config, got {body}"
    );
    for forbidden in ["header", "recording.", "read.header"] {
        assert!(
            !body.contains(forbidden),
            "the approval mode must never come from the session file, and it mentions {forbidden}"
        );
    }
    // And the policy the session runs under is built from the config alone.
    let policy_start = source
        .find("let approval: Arc<dyn ApprovalPolicy>")
        .expect("the policy is built at one place");
    let policy = &source[policy_start..policy_start + 600];
    assert!(
        policy.contains("config.approval"),
        "the policy must come from the merged config, got {policy}"
    );
    assert!(
        !policy.contains("header"),
        "the policy must never read a session header"
    );
}

#[test]
fn an_unknown_mode_name_still_parses_to_the_strictest_mode() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    let id = seed_session(dir.path(), &root, "a session", "an answer");
    let (store, _key) = recording::store_for(dir.path(), &root);
    let path = store.path_of(&id);
    let text = std::fs::read_to_string(&path).expect("the file");
    std::fs::write(&path, text.replacen("read-only", "wide-open", 1)).expect("the forged file");

    // `wide-open` is not a mode, so it parses to the strictest one, which is `read-only`. A run
    // that asks for `ask` is then wider than the stored mode, and the resume is refused.
    //
    // An unknown name that parsed to the most permissive mode would let a forged header grant
    // anything. That is the fail-open shape of `D-plugin-does-not-classify-itself`.
    let mut asked = request(dir.path(), &root, SessionSelector::Newest);
    asked.approval = "ask";
    let error = recording::open(asked)
        .map(|_| ())
        .expect_err("an unknown mode name must parse to the strictest mode");

    assert!(error.to_string().contains("--allow-widen"), "got {error}");
    let _ = id;
}

#[test]
fn a_forged_fork_origin_opens_no_file() {
    // `forked_from` is shown and never trusted. It opens no file and grants nothing.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    let id = seed_session(dir.path(), &root, "a session", "an answer");
    let (store, _key) = recording::store_for(dir.path(), &root);
    let path = store.path_of(&id);
    let sentinel = dir.path().join("sentinel-must-never-open.jsonl");
    let text = std::fs::read_to_string(&path).expect("the file");
    let forged = text.replacen(
        "\"type\":\"session\"",
        &format!(
            "\"type\":\"session\",\"forked_from\":{{\"session_id\":\"{}\",\"record_id\":\"r1\"}}",
            sentinel.display()
        ),
        1,
    );
    std::fs::write(&path, forged).expect("the forged file");

    let resumed = recording::open(request(dir.path(), &root, SessionSelector::Newest))
        .expect("a forged origin must not stop a resume");

    // The assertion is that the sentinel was never created or read. An assertion that the run
    // succeeded would pass against an implementation that follows the origin.
    assert!(
        !sentinel.exists(),
        "rho must never open a path a session file names"
    );
    assert!(resumed.id.is_some());
}

#[test]
fn a_resume_expires_a_stale_result_handle_from_the_file() {
    // The resume path, not the function. A test that called the function alone would pass while
    // the resume forgot to call it, and that is this project's signature defect.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    let id = {
        let mut recording =
            recording::open(request(dir.path(), &root, SessionSelector::New)).expect("a session");
        recording.start(&[ContentBlock::Text {
            text: "run the tests".to_string(),
        }]);
        // A turn whose tool result was stored outside the context, as a large result is.
        recording.observe(&rho_core::AgentEvent::TurnStart);
        recording.observe(&rho_core::AgentEvent::Stream(
            rho_core::StreamEvent::ToolCallStart {
                index: 0,
                id: "call-1".to_string(),
                name: "bash".to_string(),
            },
        ));
        recording.observe(&rho_core::AgentEvent::Stream(
            rho_core::StreamEvent::ToolCallEnd {
                index: 0,
                arguments: serde_json::json!({ "command": "cargo test" }),
                state: None,
            },
        ));
        recording.observe(&rho_core::AgentEvent::TurnEnd {
            stop_reason: rho_core::StopReason::ToolUse,
        });
        recording.observe(&rho_core::AgentEvent::ToolEnd {
            id: "call-1".to_string(),
            output: rho_core::ToolOutput {
                content: vec![ContentBlock::Text {
                    text: "<tool_result_preview handle=\"r-0001\" stored_bytes=\"98765\" \
                           preview_bytes=\"14\">\nthe first line\n</tool_result_preview>\nThe full \
                           result is stored outside the context. Use read_tool_result with this \
                           exact handle."
                        .to_string(),
                }],
                is_error: false,
            },
        });
        recording.id.clone().expect("an id")
    };

    let resumed = recording::open(request(dir.path(), &root, SessionSelector::Newest))
        .expect("a resume opens");
    assert_eq!(resumed.id, Some(id));

    let text = every_text(&resumed.messages);
    assert!(
        !text.contains("r-0001"),
        "the resume must expire the handle, got {text}"
    );
    assert!(
        !text.contains("read_tool_result"),
        "the promise must go, or the model spends a turn learning it failed"
    );
    assert!(
        text.contains("98765"),
        "the byte count must survive, got {text}"
    );
    assert!(
        text.contains("expired"),
        "the rewrite must say what happened"
    );
}

#[test]
fn the_run_path_records_the_prompt_the_turns_and_the_close() {
    // The three lifecycle calls, driven through `Recording`, on a real file.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    let mut recording =
        recording::open(request(dir.path(), &root, SessionSelector::New)).expect("a session");
    let path = recording.path.clone().expect("a file");

    recording.start(&[ContentBlock::Text {
        text: "fix the parser".to_string(),
    }]);
    recording.observe(&rho_core::AgentEvent::TurnStart);
    recording.observe(&rho_core::AgentEvent::Stream(
        rho_core::StreamEvent::TextStart { index: 0 },
    ));
    recording.observe(&rho_core::AgentEvent::Stream(
        rho_core::StreamEvent::TextDelta {
            index: 0,
            delta: "the bug is on line 42".to_string(),
        },
    ));
    recording.observe(&rho_core::AgentEvent::TurnEnd {
        stop_reason: rho_core::StopReason::EndTurn,
    });
    recording.close();

    let read = SessionReader::read(&path).expect("the file reads back");
    let roles: Vec<Role> = read
        .entries
        .iter()
        .filter_map(|entry| match &entry.record {
            Record::Message { message } => Some(message.role),
            _ => None,
        })
        .collect();
    assert_eq!(
        roles,
        vec![Role::User, Role::Assistant],
        "the file holds the prompt and then the answer"
    );
    assert!(
        matches!(read.entries.last().map(|e| &e.record), Some(Record::Closed)),
        "a run that ended states its own close"
    );
}

#[test]
fn the_headless_run_path_drives_every_lifecycle_call() {
    // The three calls above prove the behaviour. This proves `run_headless` really makes them.
    // Without it every test here would pass while the run path recorded nothing, and that is
    // exactly the defect this lane exists to fix: the store had no caller at all.
    let source = std::fs::read_to_string("src/cli.rs").expect("the cli source");
    let start = source
        .find("async fn run_headless")
        .expect("the headless run path");
    let end = source[start..]
        .find("\n/// Print one run onto two streams")
        .map(|offset| start + offset)
        .expect("the end of the headless run path");
    let body = &source[start..end];

    for needle in [
        "open_recording(",
        "recording.start(&input)",
        "Some(&mut recording)",
        "recording.close()",
    ] {
        assert!(
            body.contains(needle),
            "the headless run path must call {needle}, or the session records nothing"
        );
    }
    // And the printer must fold every event into the records.
    let print_start = source
        .find("/// Print one run onto two streams")
        .expect("the printer");
    let printer = &source[print_start..];
    assert!(
        printer.contains("recording.observe(event)"),
        "the printer must fold every event into the records"
    );
}

#[test]
fn a_resumed_context_holds_no_live_result_handle() {
    // The store behind a handle died with the earlier run, and the per-session nonce enforces
    // that on purpose. So the promise expires, and the byte count survives.
    let mut messages = vec![Message {
        role: Role::Tool,
        content: vec![ContentBlock::ToolResult {
            tool_call_id: "call-1".to_string(),
            content: vec![ContentBlock::Text {
                text: "<tool_result_preview handle=\"r-0001\" stored_bytes=\"98765\" \
                       preview_bytes=\"12\">\nthe first line\n</tool_result_preview>\nThe full \
                       result is stored outside the context. Use read_tool_result with this \
                       exact handle."
                    .to_string(),
            }],
            is_error: false,
        }],
    }];

    rho_core::expire_stale_result_handles(&mut messages);

    let text = every_text(&messages);
    assert!(
        !text.contains("r-0001"),
        "no live handle may survive a resume, got {text}"
    );
    assert!(
        !text.contains("read_tool_result"),
        "the promise must go, or the model spends a turn learning it failed"
    );
    assert!(
        text.contains("98765"),
        "the byte count must survive, got {text}"
    );
    assert!(
        text.contains("the first line"),
        "the preview head is real evidence, and it stays"
    );
    assert!(
        text.contains("expired"),
        "the rewrite must say what happened, got {text}"
    );
}

// ---------------------------------------------------------------------------
// Twice, because twice has caught two defects in this project.
// ---------------------------------------------------------------------------

#[test]
fn a_resume_of_a_resume_keeps_one_session() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    let id = seed_session(dir.path(), &root, "the first prompt", "the first answer");

    for round in 0..2 {
        let mut resumed = recording::open(request(dir.path(), &root, SessionSelector::Newest))
            .expect("a resume opens");
        assert_eq!(resumed.id.as_ref(), Some(&id), "round {round}");
        resumed.recorder.record_prompt(&[ContentBlock::Text {
            text: format!("round {round}"),
        }]);
        // The lock must be released before the next round opens the same session.
        drop(resumed);
    }

    assert_eq!(
        rows(dir.path(), &root).len(),
        1,
        "a resume appends, and never adds a session"
    );
    let (store, _key) = recording::store_for(dir.path(), &root);
    let read = SessionReader::read(&store.path_of(&id)).expect("the file still reads back");
    let mut ids: Vec<String> = read.entries.iter().map(|e| e.id.0.clone()).collect();
    ids.push(read.header_id.0.clone());
    let unique: std::collections::HashSet<&String> = ids.iter().collect();
    assert_eq!(ids.len(), unique.len(), "every record id stays unique");
}

#[test]
fn a_second_process_cannot_continue_a_live_session() {
    // The lock is held for as long as the recording lives, so a second open of the same session
    // is refused rather than interleaving lines into one file.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    seed_session(dir.path(), &root, "a session", "an answer");

    let held = recording::open(request(dir.path(), &root, SessionSelector::Newest))
        .expect("the first resume opens");
    let id = held.id.clone().expect("an id");

    // `newest_open` skips a live session, so a bare --continue finds nothing else to take.
    let second = recording::open(request(dir.path(), &root, SessionSelector::Newest))
        .map(|_| ())
        .expect_err("a bare --continue must not pick a live session");
    assert!(
        second.to_string().contains("no session to continue"),
        "got {second}"
    );

    // A named resume of the live session is refused, and the refusal names it.
    let named = recording::open(request(
        dir.path(),
        &root,
        SessionSelector::Named(id.as_str().to_string()),
    ))
    .map(|_| ())
    .expect_err("a named resume of a live session must be refused");
    assert!(
        matches!(
            named.downcast_ref::<SessionError>(),
            Some(SessionError::Busy { .. })
        ),
        "expected Busy, got {named}"
    );
    drop(held);
}

#[test]
fn two_runs_at_once_never_share_a_file() {
    // Two runs in one project reach one store. Each must end with its own file, and every record
    // id in each file must be unique. `session_binary.rs` drives the two-worktree half through
    // the real binary, in `two_worktrees_continuing_at_once_never_share_a_file`.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());

    let first = recording::open(request(dir.path(), &root, SessionSelector::New))
        .expect("the first run opens");
    let mut asked = request(dir.path(), &root, SessionSelector::New);
    asked.now_millis += 1000;
    let second = recording::open(asked).expect("the second run opens");

    assert_ne!(first.id, second.id, "two runs get two sessions");
    assert_ne!(first.path, second.path, "two runs get two files");
    for path in [first.path.clone().unwrap(), second.path.clone().unwrap()] {
        let read = SessionReader::read(&path).expect("the file reads back");
        let mut ids: Vec<String> = read.entries.iter().map(|e| e.id.0.clone()).collect();
        ids.push(read.header_id.0.clone());
        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(
            ids.len(),
            unique.len(),
            "{} has a duplicate id",
            path.display()
        );
    }
}

#[test]
fn a_crash_continue_takes_the_session_that_never_closed() {
    // A crash leaves a session with no `Closed` record. On the next start rho offers it, and a
    // closed session is never offered.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    // A closed session, older.
    {
        let mut recording =
            recording::open(request(dir.path(), &root, SessionSelector::New)).expect("a session");
        recording.recorder.record_prompt(&[ContentBlock::Text {
            text: "a finished session".to_string(),
        }]);
        recording.close();
    }
    // A crashed session, newer. It never closes, and the lock dies with the value.
    let crashed = {
        let mut asked = request(dir.path(), &root, SessionSelector::New);
        asked.now_millis += 60_000;
        let mut recording = recording::open(asked).expect("a session");
        recording.recorder.record_prompt(&[ContentBlock::Text {
            text: "a crashed session".to_string(),
        }]);
        recording.id.clone().expect("an id")
    };

    // The crash offer is `newest_open`, and it means a session that really did not close. Using
    // the resume choice here would pass whatever the close records said, because the crashed
    // session is also the newest.
    let (store, _key) = recording::store_for(dir.path(), &root);
    let offered = store.newest_open().expect("no error");

    assert_eq!(
        offered,
        Some(crashed.clone()),
        "a crash offers the session that never closed"
    );
    // And a resume really opens it.
    let resumed = recording::open(request(dir.path(), &root, SessionSelector::Newest))
        .expect("a crash continue opens");
    assert_eq!(resumed.id, Some(crashed));
}

#[test]
fn a_closed_session_is_continued_and_states_its_reopen() {
    // A run that ends on its own writes a `Closed` record. Continuing it is the common case, and
    // the reopen is stated on disk so a reader never finds `Closed` in the middle of a file.
    //
    // The earlier test asserted the opposite, and it asserted a defect. A live drive showed bare
    // --continue answering "no session to continue" right after a successful run. See
    // `D-continue-takes-the-newest-session-closed-or-not`.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    let id = {
        let mut recording =
            recording::open(request(dir.path(), &root, SessionSelector::New)).expect("a session");
        recording.start(&[ContentBlock::Text {
            text: "a finished session".to_string(),
        }]);
        recording.close();
        recording.id.clone().expect("an id")
    };

    let resumed = recording::open(request(dir.path(), &root, SessionSelector::Newest))
        .expect("a closed session must still be resumable");
    assert_eq!(resumed.id, Some(id.clone()));
    drop(resumed);

    let (store, _key) = recording::store_for(dir.path(), &root);
    let read = SessionReader::read(&store.path_of(&id)).expect("the file reads back");
    let reopened = read
        .entries
        .iter()
        .filter(|entry| matches!(entry.record, Record::Reopened))
        .count();
    assert_eq!(reopened, 1, "the reopen is stated on disk exactly once");
}

#[test]
fn an_empty_store_cannot_be_continued() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());

    let error = recording::open(request(dir.path(), &root, SessionSelector::Newest))
        .map(|_| ())
        .expect_err("an empty store offers nothing");

    assert!(
        error.to_string().contains("no session to continue"),
        "got {error}"
    );
}

#[test]
fn a_run_that_ends_closes_its_session() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    let mut recording =
        recording::open(request(dir.path(), &root, SessionSelector::New)).expect("a session");
    let path = recording.path.clone().expect("a file");
    recording.recorder.record_prompt(&[ContentBlock::Text {
        text: "a prompt".to_string(),
    }]);

    recording.close();

    let read = SessionReader::read(&path).expect("the file reads back");
    assert!(
        matches!(read.entries.last().map(|e| &e.record), Some(Record::Closed)),
        "a run that ended states its own close, so a crash continue never offers it"
    );
}

#[test]
fn a_secret_named_tool_argument_is_masked_on_the_run_path() {
    // It proves the **wiring**, not the function. The name states its own limit: redaction
    // matches a key name only, so a secret in a tool result or on a bash command line reaches
    // the file verbatim. See section 9.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    let mut recording =
        recording::open(request(dir.path(), &root, SessionSelector::New)).expect("a session");
    let path = recording.path.clone().expect("a file");

    recording.recorder.observe(&rho_core::AgentEvent::TurnStart);
    recording.recorder.observe(&rho_core::AgentEvent::Stream(
        rho_core::StreamEvent::ToolCallStart {
            index: 0,
            id: "call-1".to_string(),
            name: "http".to_string(),
        },
    ));
    recording.recorder.observe(&rho_core::AgentEvent::Stream(
        rho_core::StreamEvent::ToolCallEnd {
            index: 0,
            arguments: serde_json::json!({ "api_key": "sk-live-must-not-land" }),
            state: None,
        },
    ));
    recording.recorder.observe(&rho_core::AgentEvent::TurnEnd {
        stop_reason: rho_core::StopReason::ToolUse,
    });

    let text = std::fs::read_to_string(&path).expect("the file");
    assert!(
        !text.contains("sk-live-must-not-land"),
        "a credential-shaped argument key must be masked in the file a real run wrote"
    );
}

#[test]
fn a_row_of_a_recorded_run_reports_its_usage() {
    // The list surface reads what a run wrote, so this pins the two halves together.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = project(dir.path());
    let mut recording =
        recording::open(request(dir.path(), &root, SessionSelector::New)).expect("a session");
    recording.recorder.record_prompt(&[ContentBlock::Text {
        text: "fix the parser".to_string(),
    }]);
    recording
        .recorder
        .observe(&rho_core::AgentEvent::Stream(rho_core::StreamEvent::Usage(
            Usage {
                input_tokens: 1200,
                output_tokens: 340,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                cost_usd: Some(0.08),
            },
        )));

    let rows = rows(dir.path(), &root);
    let SessionRow::Session(summary) = &rows[0] else {
        panic!("a readable row");
    };
    assert_eq!(summary.title, "fix the parser");
    assert_eq!(summary.model, "test-model");
    assert_eq!(summary.usage.map(|u| u.input_tokens), Some(1200));
}

/// Every text block in a message list, joined.
fn every_text(messages: &[Message]) -> String {
    messages
        .iter()
        .flat_map(|message| message.content.iter())
        .flat_map(flatten)
        .collect::<Vec<String>>()
        .join(" ")
}

fn flatten(block: &ContentBlock) -> Vec<String> {
    match block {
        ContentBlock::Text { text } => vec![text.clone()],
        ContentBlock::ReasoningTrace { text } | ContentBlock::ReasoningReplay { text, .. } => {
            vec![text.clone()]
        }
        ContentBlock::ToolResult { content, .. } => content.iter().flat_map(flatten).collect(),
        ContentBlock::ToolCall { arguments, .. } => vec![arguments.to_string()],
        _ => Vec::new(),
    }
}
