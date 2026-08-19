//! A live subagent is addressable: you can watch it and cancel it alone.
//!
//! Before this, `AgentRegistry` counted children and did not know them, so a
//! caller could not cancel or watch one child. Three missing features were one
//! missing contract. See `SPEC-subagents` section 7a.

use std::time::Duration;

use rho_core::{AgentProgress, AgentRegistry, CancelToken, SubagentLimits, Usage};

fn registry() -> AgentRegistry {
    AgentRegistry::new(SubagentLimits {
        max_children_per_parent: 8,
        ..SubagentLimits::new()
    })
}

#[test]
fn a_spawned_child_is_listed_as_live() {
    let registry = registry();
    let root = registry.new_tree();
    let parent_cancel = CancelToken::new();

    let spawn = root.spawn_child("scout", parent_cancel.child()).unwrap();

    let live = registry.live_under(&root);
    assert_eq!(live.len(), 1, "a spawned child must be addressable");
    assert_eq!(live[0].agent, "scout");
    assert_eq!(live[0].id, spawn.node.id());
    assert_eq!(live[0].depth, 1);
}

#[test]
fn a_finished_child_is_no_longer_listed() {
    let registry = registry();
    let root = registry.new_tree();
    let cancel = CancelToken::new();

    {
        let _spawn = root.spawn_child("scout", cancel.child()).unwrap();
        assert_eq!(registry.live_under(&root).len(), 1);
    }
    // The slot dropped, so the handle goes with it. A registry that kept a dead
    // child would leak, and it would hand out a handle that cancels nothing.
    assert!(
        registry.live_under(&root).is_empty(),
        "a finished child must leave the live list"
    );
}

#[test]
fn cancelling_one_child_leaves_its_sibling_running() {
    // This is the capability the whole change exists for. A fan-out runs several
    // siblings, and one of them may need to stop.
    let registry = registry();
    let root = registry.new_tree();
    let parent = CancelToken::new();

    let first = root.spawn_child("scout", parent.child()).unwrap();
    let second = root.spawn_child("greedy", parent.child()).unwrap();

    assert!(
        registry.cancel_descendant(&root, first.node.id()),
        "cancelling a live child must report success"
    );

    let live = registry.live_under(&root);
    let first_handle = live.iter().find(|h| h.id == first.node.id()).unwrap();
    let second_handle = live.iter().find(|h| h.id == second.node.id()).unwrap();
    assert!(first_handle.is_cancelled(), "the named child must stop");
    assert!(
        !second_handle.is_cancelled(),
        "a sibling must keep running when one child is cancelled"
    );
    assert!(
        !parent.is_cancelled(),
        "cancelling one child must never cancel the parent"
    );
}

#[test]
fn cancelling_an_unknown_id_reports_false() {
    // A caller may name a child that already finished. That is a result, not a
    // fault, so the caller can say so and continue.
    let registry = registry();
    let root = registry.new_tree();
    let spawn = root
        .spawn_child("scout", CancelToken::new().child())
        .unwrap();
    let id = spawn.node.id();
    drop(spawn);

    assert!(
        !registry.cancel_descendant(&root, id),
        "cancelling a child that already finished must report false"
    );
}

#[test]
fn cancelling_the_parent_still_cancels_every_child() {
    // The old guarantee must survive the new one.
    let registry = registry();
    let root = registry.new_tree();
    let parent = CancelToken::new();

    let first = root.spawn_child("scout", parent.child()).unwrap();
    let second = root.spawn_child("greedy", parent.child()).unwrap();

    parent.cancel();

    assert!(
        registry.live_under(&root).iter().all(|h| h.is_cancelled()),
        "a parent cancel must reach every live child"
    );
    let _ = (first, second);
}

#[test]
fn a_handle_reports_the_progress_its_child_publishes() {
    // A frontend needs to read a running child without touching its transcript.
    let registry = registry();
    let root = registry.new_tree();
    let spawn = root
        .spawn_child("scout", CancelToken::new().child())
        .unwrap();

    assert_eq!(
        registry.live_under(&root)[0].progress().turns,
        0,
        "a fresh child has run no turn"
    );

    let usage = Usage {
        input_tokens: 120,
        ..Usage::default()
    };
    spawn.publish(AgentProgress { turns: 3, usage });

    let seen = registry.live_under(&root)[0].progress();
    assert_eq!(
        seen.turns, 3,
        "the handle must read the published turn count"
    );
    assert_eq!(seen.usage.input_tokens, 120, "and the published usage");
}

#[tokio::test]
async fn a_cancelled_child_token_resolves_for_a_waiter() {
    // A per-child cancel must wake whatever awaits that child, or the child keeps
    // burning tokens after a caller asked it to stop.
    let registry = registry();
    let root = registry.new_tree();
    let spawn = root
        .spawn_child("scout", CancelToken::new().child())
        .unwrap();
    let token = registry.live_under(&root)[0].cancel_token();

    let waiter = tokio::spawn(async move { token.cancelled().await });
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    assert!(!waiter.is_finished(), "the waiter must park first");

    assert!(registry.cancel_descendant(&root, spawn.node.id()));
    let woke = tokio::time::timeout(Duration::from_secs(5), waiter).await;
    assert!(
        woke.is_ok(),
        "a per-child cancel must wake the child's waiter"
    );
    woke.unwrap().unwrap();
}

// --- The tool-call budget (item 3) ---
//
// A turn cap does not bound a child that makes forty tool calls in one turn. The
// turn cap counts provider round trips; a budget must count the work.

#[test]
fn the_tool_call_budget_has_a_stated_default() {
    // The default is stated in the constructor, not hidden. See decision
    // D-no-four-argument-session-new.
    let stated = SubagentLimits::new();
    assert!(
        stated.max_tool_calls > 0,
        "a budget of zero would forbid every tool call"
    );
    assert_eq!(
        rho_core::SessionConfig::new(
            "model",
            std::env::temp_dir(),
            std::sync::Arc::new(rho_core::AllowAllPolicy),
        )
        .max_tool_calls,
        rho_core::AgentConfig::default().max_tool_calls,
        "a session and the agent loop must agree on the default"
    );
}

#[test]
fn a_child_tool_call_budget_is_capped_by_the_parent() {
    // A definition may ask for less. It may never ask for more, exactly like the
    // turn cap. Otherwise a definition file raises its own budget.
    let parent = 10u32;
    assert_eq!(
        rho_core::cap_tool_calls(parent, Some(4)),
        4,
        "less is allowed"
    );
    assert_eq!(
        rho_core::cap_tool_calls(parent, Some(999)),
        parent,
        "more is capped at the parent's budget"
    );
    assert_eq!(
        rho_core::cap_tool_calls(parent, None),
        parent,
        "no request inherits the parent's budget"
    );
}

// --- Steering a live child (SPEC-steering, applied to a subagent) ---

#[test]
fn a_handle_and_its_child_share_one_queue() {
    // The whole feature turns on this. If the handle and the child hold different
    // queues, a steer pushes into a queue nobody drains, and the message vanishes.
    let registry = registry();
    let root = registry.new_tree();
    let spawn = root
        .spawn_child("scout", CancelToken::new().child())
        .unwrap();

    let handle = registry.live_under(&root).into_iter().next().unwrap();
    handle
        .steer(vec![rho_core::ContentBlock::Text {
            text: "look at parser.rs instead".to_string(),
        }])
        .unwrap();

    assert_eq!(handle.queued(), 1, "the handle sees its own push");
    let child_queue = spawn.queue();
    assert_eq!(
        child_queue.len(),
        1,
        "the child must see the same message, or a steer is lost"
    );
    let drained = child_queue.drain();
    assert_eq!(drained.len(), 1);
    assert_eq!(
        handle.queued(),
        0,
        "a drain by the child empties the handle"
    );
}

#[test]
fn steering_one_child_does_not_reach_its_sibling() {
    let registry = registry();
    let root = registry.new_tree();
    let parent = CancelToken::new();
    let first = root.spawn_child("scout", parent.child()).unwrap();
    let second = root.spawn_child("greedy", parent.child()).unwrap();

    let live = registry.live_under(&root);
    let first_handle = live.iter().find(|h| h.id == first.node.id()).unwrap();
    first_handle
        .steer(vec![rho_core::ContentBlock::Text {
            text: "for the first child only".to_string(),
        }])
        .unwrap();

    assert_eq!(first_handle.queued(), 1);
    assert_eq!(
        second.queue().len(),
        0,
        "a steer must reach one child, never its sibling"
    );
}

#[test]
fn a_full_child_queue_is_a_typed_error_and_not_a_silent_drop() {
    let registry = registry();
    let root = registry.new_tree();
    let spawn = root
        .spawn_child("scout", CancelToken::new().child())
        .unwrap();
    let handle = registry.live_under(&root).into_iter().next().unwrap();

    let text = || {
        vec![rho_core::ContentBlock::Text {
            text: "m".to_string(),
        }]
    };
    for _ in 0..rho_core::STEER_QUEUE_CAPACITY {
        handle.steer(text()).unwrap();
    }
    let error = handle
        .steer(text())
        .expect_err("a full queue must refuse, not grow");
    assert!(
        error.to_string().contains("full"),
        "the refusal must say the queue is full, got: {error}"
    );
    assert_eq!(
        spawn.queue().len(),
        rho_core::STEER_QUEUE_CAPACITY,
        "a full queue drops no earlier message"
    );
}

// --- Ownership: a caller may only address its own descendants ---

#[test]
fn one_tree_cannot_cancel_another_trees_child() {
    // A security review proved this. `cancel` took a bare id and resolved it against
    // the whole registry, and the registry is process-wide by design, so one session
    // could stop another session's child. The shipped CLI happened to be safe because
    // each run built its own registry and no child held a control tool. That is a
    // boundary enforced by wiring, not by the contract. See D-a-caller-addresses-only-its-own.
    let registry = registry();
    let victim_root = registry.new_tree();
    let attacker_root = registry.new_tree();

    let victim = victim_root
        .spawn_child("scout", CancelToken::new().child())
        .unwrap();

    assert!(
        registry
            .descendant(&attacker_root, victim.node.id())
            .is_none(),
        "another tree's child must not be addressable"
    );
    assert!(
        !registry.cancel_descendant(&attacker_root, victim.node.id()),
        "another tree must not cancel this child"
    );
    assert!(
        !registry.live_under(&victim_root)[0].is_cancelled(),
        "the victim must still be running"
    );

    // The owner can still do both.
    assert!(
        registry
            .descendant(&victim_root, victim.node.id())
            .is_some(),
        "the owner must see its own child"
    );
    assert!(registry.cancel_descendant(&victim_root, victim.node.id()));
}

#[test]
fn a_grandchild_is_addressable_by_its_ancestor() {
    // Descendancy, not parenthood. A root must reach any depth below it, or a fan-out
    // of a fan-out becomes unmanageable.
    let registry = registry();
    let root = registry.new_tree();
    let child = root
        .spawn_child("scout", CancelToken::new().child())
        .unwrap();
    let grandchild = child
        .node
        .spawn_child("greedy", CancelToken::new().child())
        .unwrap();

    assert!(
        registry.descendant(&root, grandchild.node.id()).is_some(),
        "an ancestor must reach a grandchild"
    );
    assert!(
        registry
            .descendant(&grandchild.node, child.node.id())
            .is_none(),
        "a child must not reach upward to its own parent"
    );
}

#[test]
fn live_under_lists_only_the_callers_own_descendants() {
    let registry = registry();
    let mine = registry.new_tree();
    let theirs = registry.new_tree();
    let _a = mine
        .spawn_child("scout", CancelToken::new().child())
        .unwrap();
    let _b = theirs
        .spawn_child("greedy", CancelToken::new().child())
        .unwrap();

    let listed = registry.live_under(&mine);
    assert_eq!(listed.len(), 1, "only my own child is listed");
    assert_eq!(listed[0].agent, "scout");
    let theirs_listed = registry.live_under(&theirs);
    assert_eq!(
        theirs_listed.len(),
        1,
        "and the other tree sees only its own child, never mine"
    );
    assert_eq!(theirs_listed[0].agent, "greedy");
}

#[tokio::test]
async fn a_handle_reports_progress_while_the_child_still_runs() {
    // The point of a handle is to watch a child **during** its work. The tool
    // published progress once, after the child finished, so `progress()` read zero
    // for the whole run and then jumped to the final number. That is a post-mortem,
    // not progress, and the event name `AgentProgressed` said otherwise.
    use rho_core::{AgentProgress, Usage};

    let registry = registry();
    let root = registry.new_tree();
    let spawn = root
        .spawn_child("scout", CancelToken::new().child())
        .unwrap();
    let handle = registry.live_under(&root).into_iter().next().unwrap();

    // Stand in for the child: publish each turn as it starts.
    for turn in 1..=3u32 {
        spawn.publish(AgentProgress {
            turns: turn,
            usage: Usage::default(),
        });
        assert_eq!(
            handle.progress().turns,
            turn,
            "a handle must read turn {turn} while the child is still running"
        );
    }
}

#[test]
fn a_finished_report_is_found_by_its_own_id() {
    // The first version of `status` matched only the ancestor chain, so it answered
    // with the most recent sibling's report whatever id was asked for. Answering with
    // another child's work is worse than answering nothing.
    let registry = registry();
    let root = registry.new_tree();
    let first = root
        .spawn_child("scout", CancelToken::new().child())
        .unwrap();
    let second = root
        .spawn_child("greedy", CancelToken::new().child())
        .unwrap();
    let first_id = first.node.id();
    let second_id = second.node.id();

    let report_for = |agent: &str| rho_core::AgentReport {
        agent: agent.to_string(),
        outcome: rho_core::AgentOutcome::Done,
        summary: format!("{agent} finished"),
        usage: Usage::default(),
        turns: 1,
        gate: Default::default(),
        claims: Default::default(),
        transcript: None,
    };
    registry.record_report(&root, first_id, report_for("scout"));
    registry.record_report(&root, second_id, report_for("greedy"));
    drop(first);
    drop(second);

    match registry.status(&root, first_id) {
        Some(rho_core::AgentStatus::Finished { report }) => {
            assert_eq!(report.agent, "scout", "each id must return its own report")
        }
        other => panic!("expected the first child's report, got {other:?}"),
    }
    match registry.status(&root, second_id) {
        Some(rho_core::AgentStatus::Finished { report }) => assert_eq!(report.agent, "greedy"),
        other => panic!("expected the second child's report, got {other:?}"),
    }
    assert!(
        registry.status(&root, rho_core::AgentId(9999)).is_none(),
        "an unknown id must answer nothing, not somebody else's report"
    );
}
