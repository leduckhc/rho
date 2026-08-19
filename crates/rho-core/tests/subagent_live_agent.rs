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
    let root = registry.root();
    let parent_cancel = CancelToken::new();

    let spawn = root.spawn_child("scout", parent_cancel.child()).unwrap();

    let live = registry.live();
    assert_eq!(live.len(), 1, "a spawned child must be addressable");
    assert_eq!(live[0].agent, "scout");
    assert_eq!(live[0].id, spawn.node.id());
    assert_eq!(live[0].depth, 1);
}

#[test]
fn a_finished_child_is_no_longer_listed() {
    let registry = registry();
    let root = registry.root();
    let cancel = CancelToken::new();

    {
        let _spawn = root.spawn_child("scout", cancel.child()).unwrap();
        assert_eq!(registry.live().len(), 1);
    }
    // The slot dropped, so the handle goes with it. A registry that kept a dead
    // child would leak, and it would hand out a handle that cancels nothing.
    assert!(
        registry.live().is_empty(),
        "a finished child must leave the live list"
    );
}

#[test]
fn cancelling_one_child_leaves_its_sibling_running() {
    // This is the capability the whole change exists for. A fan-out runs several
    // siblings, and one of them may need to stop.
    let registry = registry();
    let root = registry.root();
    let parent = CancelToken::new();

    let first = root.spawn_child("scout", parent.child()).unwrap();
    let second = root.spawn_child("greedy", parent.child()).unwrap();

    assert!(
        registry.cancel(first.node.id()),
        "cancelling a live child must report success"
    );

    let live = registry.live();
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
    let root = registry.root();
    let spawn = root
        .spawn_child("scout", CancelToken::new().child())
        .unwrap();
    let id = spawn.node.id();
    drop(spawn);

    assert!(
        !registry.cancel(id),
        "cancelling a child that already finished must report false"
    );
}

#[test]
fn cancelling_the_parent_still_cancels_every_child() {
    // The old guarantee must survive the new one.
    let registry = registry();
    let root = registry.root();
    let parent = CancelToken::new();

    let first = root.spawn_child("scout", parent.child()).unwrap();
    let second = root.spawn_child("greedy", parent.child()).unwrap();

    parent.cancel();

    assert!(
        registry.live().iter().all(|h| h.is_cancelled()),
        "a parent cancel must reach every live child"
    );
    let _ = (first, second);
}

#[test]
fn a_handle_reports_the_progress_its_child_publishes() {
    // A frontend needs to read a running child without touching its transcript.
    let registry = registry();
    let root = registry.root();
    let spawn = root
        .spawn_child("scout", CancelToken::new().child())
        .unwrap();

    assert_eq!(
        registry.live()[0].progress().turns,
        0,
        "a fresh child has run no turn"
    );

    let usage = Usage {
        input_tokens: 120,
        ..Usage::default()
    };
    spawn.publish(AgentProgress { turns: 3, usage });

    let seen = registry.live()[0].progress();
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
    let root = registry.root();
    let spawn = root
        .spawn_child("scout", CancelToken::new().child())
        .unwrap();
    let token = registry.live()[0].cancel_token();

    let waiter = tokio::spawn(async move { token.cancelled().await });
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    assert!(!waiter.is_finished(), "the waiter must park first");

    assert!(registry.cancel(spawn.node.id()));
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
