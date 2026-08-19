//! Tests for the subagent spawner: limits, the cycle guard, the retry cap, and
//! the result contract.
//!
//! See `docs/specs/20260818-000223-SPEC-subagents.md` sections 6, 7, and 8.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{ScriptedProvider, text_turn, tool_call_turn};
use rho_core::{
    AgentId, AgentOutcome, AgentRegistry, CancelToken, Context, HookChain, MAX_CHILD_RETRIES,
    Provider, RetryLedger, Session, StreamEvent, SubagentError, SubagentLimits, ToolRegistry,
    check_no_cycle, collect_report,
};

/// Build a child session that replays the scripted turns.
fn child_session(turns: Vec<Vec<StreamEvent>>) -> Session {
    child_session_with_tools(turns, ToolRegistry::new())
}

/// Build a child session with a scripted provider and a tool registry.
fn child_session_with_tools(turns: Vec<Vec<StreamEvent>>, tools: ToolRegistry) -> Session {
    let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::new(turns));
    let config = common::test_config();
    Session::with_config(
        config,
        provider,
        Arc::new(tools),
        Arc::new(HookChain::new()),
        Context::new(None, Vec::new()),
    )
}

// --- Limits ---

#[test]
fn a_depth_of_zero_forbids_spawning() {
    let limits = SubagentLimits {
        max_depth: 0,
        ..SubagentLimits::new()
    };
    let registry = AgentRegistry::new(limits);
    let root = registry.root();
    let error = root.spawn_child("scout", CancelToken::new()).unwrap_err();
    match error {
        SubagentError::DepthExceeded { limit, attempted } => {
            assert_eq!(limit, 0);
            assert_eq!(attempted, 1);
        }
        other => panic!("expected DepthExceeded, got {other:?}"),
    }
}

#[test]
fn depth_beyond_the_cap_is_refused_and_the_reason_names_the_limit() {
    let limits = SubagentLimits {
        max_depth: 2,
        ..SubagentLimits::new()
    };
    let registry = AgentRegistry::new(limits);
    let root = registry.root();
    let _s1 = root.spawn_child("scout", CancelToken::new()).unwrap();
    let child = &_s1.node;
    let _s2 = child.spawn_child("scout", CancelToken::new()).unwrap();
    let grandchild = &_s2.node;
    // The great-grandchild would be depth 3, past the cap of 2.
    let error = grandchild
        .spawn_child("scout", CancelToken::new())
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("depth limit is 2"), "{message}");
    assert!(message.contains("depth 3"), "{message}");
    // This assertion changed on purpose. It used to require the text
    // `--max-agent-depth`, and that flag never existed, so the test pinned a
    // refusal that sent the user after an impossible fix. A live sweep found it.
    // See docs/verification/subagents-bedrock.md and decision D-cli-depth-is-zero.
    // The refusal must still teach, so it now says what to do instead.
    assert!(message.contains("Do the work here"), "{message}");
    assert!(
        !message.contains("--max-agent-depth"),
        "the refusal must name no flag that cannot help: {message}"
    );
}

#[test]
fn more_children_than_the_per_parent_cap_is_refused() {
    let limits = SubagentLimits {
        max_children_per_parent: 2,
        ..SubagentLimits::new()
    };
    let registry = AgentRegistry::new(limits);
    let root = registry.root();
    let _s1 = root.spawn_child("scout", CancelToken::new()).unwrap();
    let _c1 = &_s1.node;
    let _s2 = root.spawn_child("scout", CancelToken::new()).unwrap();
    let _c2 = &_s2.node;
    let error = root.spawn_child("scout", CancelToken::new()).unwrap_err();
    match error {
        SubagentError::TooManyChildren { limit, current } => {
            assert_eq!(limit, 2);
            assert_eq!(current, 2);
        }
        other => panic!("expected TooManyChildren, got {other:?}"),
    }
    assert!(
        error.to_string().contains("--max-children-per-parent"),
        "{error}"
    );
}

#[test]
fn the_process_wide_cap_is_refused_across_two_parents() {
    // The case a per-parent cap misses. Two parents each under their own child
    // cap still cannot pass the process-wide cap.
    let limits = SubagentLimits {
        max_depth: 3,
        max_children_per_parent: 10,
        max_live_total: 3,
        ..SubagentLimits::new()
    };
    let registry = AgentRegistry::new(limits);
    let root = registry.root();
    // Two separate parents, each holding one live child. That is three live
    // agents once we add the two parents? No: the root does not count until it
    // is spawned. Build two sibling parents under the root, then a child of each.
    let _sa = root.spawn_child("scout", CancelToken::new()).unwrap();
    let parent_a = &_sa.node; // live total 1
    let _sb = root.spawn_child("scout", CancelToken::new()).unwrap();
    let parent_b = &_sb.node; // live total 2
    let _sca = parent_a.spawn_child("scout", CancelToken::new()).unwrap();
    let _child_a = &_sca.node; // live total 3
    // The process-wide cap of 3 is now reached. Parent B cannot spawn.
    let error = parent_b
        .spawn_child("scout", CancelToken::new())
        .unwrap_err();
    match error {
        SubagentError::TooManyLiveAgents { limit, current } => {
            assert_eq!(limit, 3);
            assert_eq!(current, 3);
        }
        other => panic!("expected TooManyLiveAgents, got {other:?}"),
    }
    assert!(error.to_string().contains("--max-live-agents"), "{error}");
}

#[test]
fn a_finished_child_frees_its_slot() {
    // A dropped slot frees both the per-parent and the process-wide count. This
    // is what lets a fan-out of fifty run in sequence without exhausting a cap.
    let limits = SubagentLimits {
        max_children_per_parent: 1,
        max_live_total: 1,
        ..SubagentLimits::new()
    };
    let registry = AgentRegistry::new(limits);
    let root = registry.root();
    {
        let _slot = root.spawn_child("scout", CancelToken::new()).unwrap();
        let _child = &_slot.node;
        assert_eq!(registry.live_total(), 1);
    }
    // The slot dropped, so the counts are free again.
    assert_eq!(registry.live_total(), 0);
    assert_eq!(root.live_children(), 0);
    let _slot = root
        .spawn_child("scout", CancelToken::new())
        .expect("a freed slot allows a new child");
    let _child = &_slot.node;
}

// --- The cycle guard ---

#[test]
fn a_cycle_in_the_parent_chain_is_refused_rather_than_looping() {
    // The tree cannot cycle by construction. A duplicate id means a bug. The
    // walk carries a visited set, so it refuses rather than loops forever.
    let no_cycle = [AgentId(0), AgentId(1), AgentId(2)];
    assert!(check_no_cycle(&no_cycle).is_ok());

    let cycle = [AgentId(0), AgentId(1), AgentId(0)];
    let error = check_no_cycle(&cycle).unwrap_err();
    assert!(matches!(error, SubagentError::CycleDetected));
}

// --- The retry cap ---

#[test]
fn retrying_a_dying_child_stops_at_the_retry_cap() {
    let ledger = RetryLedger::new();
    let key = "recon-task";
    // The first deaths report a growing count and allow a retry.
    for expected in 1..MAX_CHILD_RETRIES {
        let count = ledger
            .record_death(key)
            .expect("a retry is allowed below the cap");
        assert_eq!(count, expected);
    }
    // The death that reaches the cap is refused.
    let error = ledger.record_death(key).unwrap_err();
    match error {
        SubagentError::RetryCapReached { deaths, limit } => {
            assert_eq!(deaths, MAX_CHILD_RETRIES);
            assert_eq!(limit, MAX_CHILD_RETRIES);
        }
        other => panic!("expected RetryCapReached, got {other:?}"),
    }
}

// --- The result contract ---

#[tokio::test]
async fn a_parent_receives_a_summary_and_the_usage() {
    let mut turn = text_turn("The bug is in parser.rs at line 42.");
    // Add a usage report to the turn, so the report sums it.
    turn.insert(
        turn.len() - 1,
        StreamEvent::Usage(rho_core::Usage {
            input_tokens: 100,
            output_tokens: 20,
            ..rho_core::Usage::default()
        }),
    );
    let session = child_session(vec![turn]);
    let cancel = CancelToken::new();
    let events = session.prompt(Vec::new(), cancel.clone());
    let report = collect_report(
        "scout",
        events,
        cancel,
        rho_core::CollectOptions::with_timeout(Duration::from_secs(60)),
    )
    .await;

    assert_eq!(report.outcome, AgentOutcome::Done);
    assert_eq!(report.summary, "The bug is in parser.rs at line 42.");
    assert_eq!(report.usage.input_tokens, 100);
    assert_eq!(report.usage.output_tokens, 20);
    assert_eq!(report.agent, "scout");
    assert!(report.turns >= 1);
}

#[tokio::test]
async fn a_long_child_summary_is_capped() {
    let long = "x".repeat(rho_core::MAX_SUMMARY_CHARS + 500);
    let session = child_session(vec![text_turn(&long)]);
    let cancel = CancelToken::new();
    let events = session.prompt(Vec::new(), cancel.clone());
    let report = collect_report(
        "scout",
        events,
        cancel,
        rho_core::CollectOptions::with_timeout(Duration::from_secs(60)),
    )
    .await;
    assert_eq!(
        report.summary.chars().count(),
        rho_core::MAX_SUMMARY_CHARS,
        "the summary is capped"
    );
}

#[tokio::test]
async fn a_child_transcript_never_enters_the_parent_context() {
    // The child calls a tool, then answers. The transcript holds the tool call
    // and the intermediate events. The summary holds only the final answer. The
    // transcript goes to disk, never to the summary.
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("child.log");
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(common::RecordingTool::new("probe")));
    let turns = vec![
        tool_call_turn("c1", "probe", serde_json::json!({})),
        text_turn("final answer"),
    ];
    let session = child_session_with_tools(turns, tools);
    let cancel = CancelToken::new();
    let events = session.prompt(Vec::new(), cancel.clone());
    let report = collect_report(
        "scout",
        events,
        cancel,
        rho_core::CollectOptions::with_timeout(Duration::from_secs(60))
            .transcript(transcript.clone()),
    )
    .await;

    assert_eq!(report.summary, "final answer");
    assert!(
        !report.summary.contains("probe"),
        "the summary must not carry the intermediate tool call"
    );
    // The full transcript is on disk, and it does hold the intermediate events.
    assert_eq!(report.transcript.as_deref(), Some(transcript.as_path()));
    let written = std::fs::read_to_string(&transcript).unwrap();
    assert!(
        written.contains("probe"),
        "the transcript holds the tool call"
    );
}

#[tokio::test]
async fn a_transcript_writes_into_a_directory_that_does_not_exist_yet() {
    // The shipped caller puts transcripts in `<root>/.rho/agent-transcripts/`, and
    // nothing creates that directory. So every real run lost its transcript and
    // only logged a warning. The test above passed against this bug, because a
    // `tempdir` already exists. This test pins the real path shape.
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir
        .path()
        .join(".rho")
        .join("agent-transcripts")
        .join("agent-1.log");
    assert!(
        !transcript.parent().unwrap().exists(),
        "the parent directory must be absent, or this test proves nothing"
    );

    let session = child_session(vec![text_turn("done")]);
    let cancel = CancelToken::new();
    let events = session.prompt(Vec::new(), cancel.clone());
    let report = collect_report(
        "scout",
        events,
        cancel,
        rho_core::CollectOptions::with_timeout(Duration::from_secs(60))
            .transcript(transcript.clone()),
    )
    .await;

    assert_eq!(
        report.transcript.as_deref(),
        Some(transcript.as_path()),
        "the report must name the transcript it wrote"
    );
    assert!(
        transcript.exists(),
        "the transcript must exist, so the missing parent directory was created"
    );
}

#[tokio::test]
async fn a_child_failure_returns_a_result_and_the_parent_continues() {
    // The provider stream ends with no `Done` event, which the driver reports as
    // a decode fault. A child failure is a result, not the end of the run.
    let broken = vec![vec![StreamEvent::MessageStart {
        role: rho_core::Role::Assistant,
    }]];
    let session = child_session(broken);
    let cancel = CancelToken::new();
    let events = session.prompt(Vec::new(), cancel.clone());
    let report = collect_report(
        "scout",
        events,
        cancel,
        rho_core::CollectOptions::with_timeout(Duration::from_secs(60)),
    )
    .await;
    assert!(
        matches!(report.outcome, AgentOutcome::Failed { .. }),
        "a failed child yields Failed, got {:?}",
        report.outcome
    );
}

#[tokio::test]
async fn a_child_that_ends_without_reporting_is_reported_as_failed() {
    // An empty script yields a stream that ends with no `AgentEnd`. The child
    // died holding work. Silence must become a `Failed` result.
    let session = child_session(vec![Vec::new()]);
    let cancel = CancelToken::new();
    let events = session.prompt(Vec::new(), cancel.clone());
    let report = collect_report(
        "scout",
        events,
        cancel,
        rho_core::CollectOptions::with_timeout(Duration::from_secs(60)),
    )
    .await;
    match report.outcome {
        AgentOutcome::Failed { reason } => assert!(!reason.is_empty()),
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn a_child_past_its_timeout_is_cancelled_and_reported() {
    // A registered blocking tool never returns, so the child hangs. The timeout
    // then fires and cancels the child. The paused runtime auto-advances time to
    // the timer, with no wall sleep.
    let blocker = common::BlockingTool::new();
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(blocker));
    let turn = tool_call_turn("call-1", "blocker", serde_json::json!({}));
    let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::cycling(turn));
    let session = Session::with_config(
        common::test_config(),
        provider,
        Arc::new(tools),
        Arc::new(HookChain::new()),
        Context::new(None, Vec::new()),
    );
    let cancel = CancelToken::new();
    let events = session.prompt(Vec::new(), cancel.clone());
    let report = collect_report(
        "scout",
        events,
        cancel.clone(),
        rho_core::CollectOptions::with_timeout(Duration::from_millis(50)),
    )
    .await;
    assert_eq!(report.outcome, AgentOutcome::Canceled);
    assert!(
        cancel.is_cancelled(),
        "the timeout cancels the shared token"
    );
}

#[tokio::test]
async fn cancelling_the_parent_cancels_every_descendant() {
    // The shared `CancelToken` is the mechanism. A cancel on the token the child
    // runs under stops the child, which reports `Canceled`.
    let session = child_session(vec![text_turn("answer")]);
    let cancel = CancelToken::new();
    cancel.cancel();
    let events = session.prompt(Vec::new(), cancel.clone());
    let report = collect_report(
        "scout",
        events,
        cancel,
        rho_core::CollectOptions::with_timeout(Duration::from_secs(60)),
    )
    .await;
    assert_eq!(report.outcome, AgentOutcome::Canceled);
}

/// A turn that ends with a stop reason, used to reach `OutOfTurns` behaviour.
#[tokio::test]
async fn a_child_out_of_turns_is_reported() {
    // A cycling tool-call turn never stops on its own, so the loop hits its turn
    // cap and ends with `MaxTurnRequests`, which maps to `OutOfTurns`.
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(common::RecordingTool::new("probe")));
    let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::cycling(tool_call_turn(
        "c",
        "probe",
        serde_json::json!({}),
    )));
    let config = common::test_config().with_max_turns(2);
    let session = Session::with_config(
        config,
        provider,
        Arc::new(tools),
        Arc::new(HookChain::new()),
        Context::new(None, Vec::new()),
    );
    let cancel = CancelToken::new();
    let events = session.prompt(Vec::new(), cancel.clone());
    let report = collect_report(
        "scout",
        events,
        cancel,
        rho_core::CollectOptions::with_timeout(Duration::from_secs(60)),
    )
    .await;
    assert_eq!(report.outcome, AgentOutcome::OutOfTurns);
}

#[tokio::test]
async fn the_tool_call_budget_stops_a_turn_that_asks_for_too_many_tools() {
    // The case a turn cap cannot see. One turn asks for five tool calls, and the
    // budget is three. A turn cap of 32 would let all five run.
    let dir = tempfile::tempdir().unwrap();
    // A counting tool, because the outcome alone cannot show an overrun. A review
    // mutated the check from `>=` to `>` and this test still passed, so it proved
    // only that the run ended, never that the cap held.
    let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(common::CountingTool::new(
        "probe",
        Arc::clone(&calls),
    )));

    // One turn, five calls.
    let mut turn = vec![StreamEvent::MessageStart {
        role: rho_core::Role::Assistant,
    }];
    for index in 0..5u32 {
        turn.push(StreamEvent::ToolCallStart {
            index,
            id: format!("c{index}"),
            name: "probe".to_string(),
        });
        turn.push(StreamEvent::ToolCallEnd {
            index,
            arguments: serde_json::json!({}),
        });
    }
    turn.push(StreamEvent::Done {
        stop_reason: rho_core::StopReason::ToolUse,
    });

    let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::new(vec![turn]));
    let config = rho_core::SessionConfig::new(
        "child-model",
        dir.path().to_path_buf(),
        Arc::new(rho_core::AllowAllPolicy),
    )
    .with_max_tool_calls(3);
    let session = Session::with_config(
        config,
        provider,
        Arc::new(tools),
        Arc::new(rho_core::HookChain::new()),
        rho_core::Context::new(None, Vec::new()),
    );

    let cancel = CancelToken::new();
    let events = session.prompt(Vec::new(), cancel.clone());
    let report = collect_report(
        "scout",
        events,
        cancel,
        rho_core::CollectOptions::with_timeout(Duration::from_secs(60)),
    )
    .await;

    // The budget is spent, so the child is out of room. The parent gets what it
    // had rather than nothing.
    assert_eq!(
        report.outcome,
        rho_core::AgentOutcome::OutOfTurns,
        "spending the tool-call budget must end the run, got {:?}",
        report.outcome
    );
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "the budget is 3, so exactly 3 calls may run. A cap that is noticed after the \
         fact is not a cap."
    );
}

#[test]
fn the_retry_ledger_does_not_grow_without_a_bound() {
    // The ledger's key holds the whole prompt, which a model writes. A parent that
    // fails many distinct tasks would grow the map for ever. A security review
    // flagged it, and this project has already shipped one unbounded buffer.
    let ledger = RetryLedger::new();
    for index in 0..5_000 {
        // Each key is distinct, so nothing is ever a repeat.
        let _ = ledger.record_death(&format!("scout\u{1f}task number {index}"));
    }
    assert!(
        ledger.tracked() <= 256,
        "the ledger must stay bounded, tracked {}",
        ledger.tracked()
    );

    // And the cap must not break the guarantee: the same work still hits the cap.
    let fresh = RetryLedger::new();
    let mut refused = false;
    for _ in 0..MAX_CHILD_RETRIES {
        if fresh.record_death("scout\u{1f}the poisoned task").is_err() {
            refused = true;
        }
    }
    assert!(refused, "repeated deaths of one task must still be refused");
}

// --- The child transcript: streamed, JSONL, and readable (round eight) ---

#[tokio::test]
async fn a_transcript_is_jsonl_and_one_line_per_event() {
    // The transcript was `format!("{event:?}")`, which no reader can parse. pi writes
    // JSONL, one entry per message, and a parent that is handed the path needs a
    // format it can actually read.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("child.jsonl");
    let session = child_session(vec![text_turn("the answer")]);
    let cancel = CancelToken::new();
    let events = session.prompt(Vec::new(), cancel.clone());
    let report = collect_report(
        "scout",
        events,
        cancel,
        rho_core::CollectOptions::with_timeout(Duration::from_secs(60)).transcript(path.clone()),
    )
    .await;

    assert_eq!(report.transcript.as_deref(), Some(path.as_path()));
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(!body.trim().is_empty(), "the transcript must hold entries");
    for line in body.lines() {
        let entry: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("every line must be JSON: {e} in {line}"));
        assert!(
            entry["ts"].is_string(),
            "each entry carries a timestamp: {line}"
        );
        assert_eq!(
            entry["agent"], "scout",
            "each entry names the agent: {line}"
        );
        assert!(
            entry["type"].is_string(),
            "each entry names its kind: {line}"
        );
    }
    assert!(
        body.contains("TurnStart"),
        "a turn must appear, got: {body}"
    );
    assert!(
        body.contains("the answer"),
        "the child's text must appear, got: {body}"
    );
}

#[tokio::test]
async fn a_timed_out_child_still_leaves_the_lines_it_wrote() {
    // The case a transcript is most wanted for, and the one that had none. The old
    // writer buffered every line and wrote once at the end, so a child that was
    // cancelled or crashed left an empty file or no file at all.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("timeout.jsonl");

    // A tool that never returns, so the child is still working when the timeout fires.
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(common::HangingTool::new("probe")));
    let session = child_session_with_tools(
        vec![tool_call_turn("c1", "probe", serde_json::json!({}))],
        tools,
    );
    let cancel = CancelToken::new();
    let events = session.prompt(Vec::new(), cancel.clone());
    let report = collect_report(
        "scout",
        events,
        cancel,
        rho_core::CollectOptions::with_timeout(Duration::from_millis(50)).transcript(path.clone()),
    )
    .await;

    assert_eq!(report.outcome, AgentOutcome::Canceled);
    let body = std::fs::read_to_string(&path).expect("a cancelled child still leaves its file");
    assert!(
        body.contains("TurnStart"),
        "the lines written before the timeout must survive, got: {body:?}"
    );
}
