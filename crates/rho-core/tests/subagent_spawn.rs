//! Tests for the subagent spawner: limits, the cycle guard, the retry cap, and
//! the result contract.
//!
//! See `docs/specs/SPEC-11-subagents.md` sections 6, 7, and 8.

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
    let error = root.spawn_child().unwrap_err();
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
    let (child, _s1) = root.spawn_child().unwrap();
    let (grandchild, _s2) = child.spawn_child().unwrap();
    // The great-grandchild would be depth 3, past the cap of 2.
    let error = grandchild.spawn_child().unwrap_err();
    let message = error.to_string();
    assert!(message.contains("depth limit is 2"), "{message}");
    assert!(message.contains("depth 3"), "{message}");
    assert!(message.contains("--max-agent-depth"), "{message}");
}

#[test]
fn more_children_than_the_per_parent_cap_is_refused() {
    let limits = SubagentLimits {
        max_children_per_parent: 2,
        ..SubagentLimits::new()
    };
    let registry = AgentRegistry::new(limits);
    let root = registry.root();
    let (_c1, _s1) = root.spawn_child().unwrap();
    let (_c2, _s2) = root.spawn_child().unwrap();
    let error = root.spawn_child().unwrap_err();
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
    let (parent_a, _sa) = root.spawn_child().unwrap(); // live total 1
    let (parent_b, _sb) = root.spawn_child().unwrap(); // live total 2
    let (_child_a, _sca) = parent_a.spawn_child().unwrap(); // live total 3
    // The process-wide cap of 3 is now reached. Parent B cannot spawn.
    let error = parent_b.spawn_child().unwrap_err();
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
        let (_child, _slot) = root.spawn_child().unwrap();
        assert_eq!(registry.live_total(), 1);
    }
    // The slot dropped, so the counts are free again.
    assert_eq!(registry.live_total(), 0);
    assert_eq!(root.live_children(), 0);
    let (_child, _slot) = root.spawn_child().expect("a freed slot allows a new child");
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
    let report = collect_report("scout", events, cancel, Duration::from_secs(60), None).await;

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
    let report = collect_report("scout", events, cancel, Duration::from_secs(60), None).await;
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
        Duration::from_secs(60),
        Some(transcript.clone()),
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
async fn a_child_failure_returns_a_result_and_the_parent_continues() {
    // The provider stream ends with no `Done` event, which the driver reports as
    // a decode fault. A child failure is a result, not the end of the run.
    let broken = vec![vec![StreamEvent::MessageStart {
        role: rho_core::Role::Assistant,
    }]];
    let session = child_session(broken);
    let cancel = CancelToken::new();
    let events = session.prompt(Vec::new(), cancel.clone());
    let report = collect_report("scout", events, cancel, Duration::from_secs(60), None).await;
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
    let report = collect_report("scout", events, cancel, Duration::from_secs(60), None).await;
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
        Duration::from_millis(50),
        None,
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
    let report = collect_report("scout", events, cancel, Duration::from_secs(60), None).await;
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
    let report = collect_report("scout", events, cancel, Duration::from_secs(60), None).await;
    assert_eq!(report.outcome, AgentOutcome::OutOfTurns);
}
