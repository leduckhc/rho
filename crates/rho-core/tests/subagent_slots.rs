//! The slot queue: a per-parent cap queues a child instead of refusing it.
//!
//! Tests for `SPEC-subagent-slots-handles-grace` sections 2.1 to 2.9. Every test
//! uses real tasks and no `sleep`. Nothing here talks to a provider, because the
//! subject is admission and not a run.

use std::sync::Arc;
use std::time::Duration;

use rho_core::{
    Admission, AgentId, AgentRegistry, AgentStatus, CancelToken, Dequeued, QueueScope,
    SubagentError, SubagentLimits,
};

/// Limits with one child per parent, so the second admission must queue.
fn one_slot() -> SubagentLimits {
    SubagentLimits {
        max_children_per_parent: 1,
        ..SubagentLimits::new()
    }
}

fn registry(limits: SubagentLimits) -> AgentRegistry {
    AgentRegistry::new(limits)
}

#[tokio::test]
async fn admit_child_with_a_free_slot_starts_at_once() {
    // The `Started` arm. An earlier draft of this spec never asserted it.
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();
    let admission = root
        .admit_child("scout", CancelToken::new())
        .expect("a free slot admits");
    match admission {
        Admission::Started(spawn) => {
            assert_eq!(spawn.node.depth(), 1, "a child of the root sits at depth 1");
        }
        Admission::Queued(queued) => panic!("a free slot must not queue: id {}", queued.id()),
    }
}

#[tokio::test]
async fn admit_child_over_the_per_parent_cap_queues_and_returns_an_id() {
    // The heart of the change. Over the per-parent cap, rho waits instead of
    // refusing, and the caller gets an id it can poll, steer, and cancel.
    let registry = registry(one_slot());
    let root = registry.new_tree();
    let _first = root
        .admit_child("scout", CancelToken::new())
        .expect("the first child starts");

    let admission = root
        .admit_child("scout", CancelToken::new())
        .expect("the second child is admitted, not refused");
    match admission {
        Admission::Queued(queued) => {
            assert_eq!(queued.position(), 1, "it is first in its parent's line");
            assert!(queued.id() != AgentId(0) || queued.id() == queued.id());
        }
        Admission::Started(_) => panic!("a full per-parent cap must queue"),
    }
}

#[tokio::test]
async fn admit_child_over_the_process_wide_cap_refuses_and_names_the_limit() {
    // A wait on this cap is a wait on another tree, so rho refuses instead. The
    // message must name the limit and the flag that raises it.
    let limits = SubagentLimits {
        max_live_total: 1,
        ..SubagentLimits::new()
    };
    let registry = registry(limits);
    let first_tree = registry.new_tree();
    let _held = first_tree
        .admit_child("scout", CancelToken::new())
        .expect("the first child starts");

    let other_tree = registry.new_tree();
    let refusal = other_tree
        .admit_child("scout", CancelToken::new())
        .expect_err("the process-wide cap must refuse");
    let text = refusal.to_string();
    assert!(
        matches!(refusal, SubagentError::TooManyLiveAgents { limit: 1, .. }),
        "the refusal must name the process-wide cap: {refusal:?}"
    );
    assert!(
        text.contains("--max-live-agents"),
        "the refusal must name the flag that helps: {text}"
    );
}

#[tokio::test]
async fn a_queued_child_starts_when_a_slot_frees() {
    // Dropping the live slot releases the permit, and the waiter takes it.
    let registry = registry(one_slot());
    let root = registry.new_tree();
    let first = root
        .admit_child("scout", CancelToken::new())
        .expect("the first child starts");
    let queued = match root
        .admit_child("scout", CancelToken::new())
        .expect("the second is admitted")
    {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };

    let waiter = tokio::spawn(async move { queued.started().await });
    drop(first);

    let spawn = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("the waiter must not hang")
        .expect("the task must not panic")
        .expect("a freed slot must start the waiter");
    assert_eq!(spawn.node.depth(), 1);
}

#[tokio::test]
async fn every_waiter_eventually_starts_when_slots_free_one_at_a_time() {
    // The liveness test. A counter plus a notify loses a wake that fires between a
    // failed retry and the next await, and both other queue tests still pass while
    // that happens. Three waiters and one slot is the smallest case that shows it.
    let registry = registry(one_slot());
    let root = registry.new_tree();
    // The live child holds the only slot. Each waiter takes it in turn, and the
    // binding below is rebound rather than read, which is the point: dropping it is
    // what frees the slot.
    let mut live: Option<rho_core::ChildSpawn> = match root
        .admit_child("scout", CancelToken::new())
        .expect("the first child starts")
    {
        Admission::Started(spawn) => Some(spawn),
        Admission::Queued(_) => panic!("the first child has a slot"),
    };

    let mut waiters = Vec::new();
    for _ in 0..3 {
        let queued = match root
            .admit_child("scout", CancelToken::new())
            .expect("each is admitted")
        {
            Admission::Queued(queued) => queued,
            Admission::Started(_) => panic!("the cap is one"),
        };
        waiters.push(tokio::spawn(async move { queued.started().await }));
    }

    for waiter in waiters {
        // Free the one slot, then wait for exactly one waiter to take it.
        drop(live.take());
        let spawn = tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .expect("every waiter must eventually start")
            .expect("the task must not panic")
            .expect("a freed slot must start a waiter");
        live = Some(spawn);
    }
    drop(live);
}

#[tokio::test]
async fn a_queued_child_refuses_when_the_process_wide_cap_filled_while_it_waited() {
    // A queued child holds no process-wide permit, so the cap can fill between the
    // admission and the start. rho refuses rather than wait on another tree.
    //
    // The interleaving is deterministic, not lucky. `tokio::test` runs one thread, so
    // the waiter task cannot run until this test awaits. Dropping the live child frees
    // one process-wide permit, and the other tree takes it synchronously before the
    // waiter is ever polled.
    let limits = SubagentLimits {
        max_children_per_parent: 1,
        max_live_total: 2,
        ..SubagentLimits::new()
    };
    let registry = registry(limits);
    let root = registry.new_tree();
    let first = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Started(spawn) => spawn,
        Admission::Queued(_) => panic!("the first child has a slot"),
    };
    let other = registry.new_tree();
    let _other_child = other
        .admit_child("scout", CancelToken::new())
        .expect("the other tree takes the second process-wide slot");

    let queued = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the per-parent cap is one"),
    };
    let waiter = tokio::spawn(async move { queued.started().await });

    // Free the parent's slot, then take the process-wide permit it released before
    // the waiter runs.
    drop(first);
    let third_tree = registry.new_tree();
    let _thief = third_tree
        .spawn_child("scout", CancelToken::new())
        .expect("a third tree takes the freed process-wide slot");

    let outcome = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("the waiter must not hang")
        .expect("the task must not panic");
    match outcome {
        Err(Dequeued::ProcessWideFull { limit }) => assert_eq!(limit, 2),
        other => panic!("a full process cap must refuse at the start: {other:?}"),
    }
}

#[tokio::test]
async fn a_refused_queued_child_releases_its_per_parent_permit() {
    // A waiter that gives up must not keep the slot it took, or its sibling waits for
    // ever. The refusal happens after the per-parent permit is held, so the release
    // order is the property under test.
    let limits = SubagentLimits {
        max_children_per_parent: 1,
        max_live_total: 2,
        ..SubagentLimits::new()
    };
    let registry = registry(limits);
    let root = registry.new_tree();
    let first = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Started(spawn) => spawn,
        Admission::Queued(_) => panic!("the first child has a slot"),
    };
    let other = registry.new_tree();
    let _other_child = other.admit_child("scout", CancelToken::new()).unwrap();
    let refused = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the per-parent cap is one"),
    };
    let waiter = tokio::spawn(async move { refused.started().await });

    drop(first);
    let third_tree = registry.new_tree();
    let thief = third_tree
        .spawn_child("scout", CancelToken::new())
        .expect("a third tree takes the freed process-wide slot");
    let outcome = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("no hang")
        .expect("no panic");
    assert!(
        matches!(outcome, Err(Dequeued::ProcessWideFull { .. })),
        "the waiter must refuse: {outcome:?}"
    );

    // The refusal released the per-parent permit. Once a process-wide slot frees, the
    // parent must be able to start a child at once.
    drop(thief);
    match root
        .admit_child("scout", CancelToken::new())
        .expect("the parent has a free slot again")
    {
        Admission::Started(_) => {}
        Admission::Queued(queued) => {
            panic!(
                "the refused waiter kept its per-parent permit: {}",
                queued.id()
            )
        }
    }
}

#[tokio::test]
async fn a_queued_child_starts_before_a_later_one_under_one_parent() {
    // First in, first out, per parent. The semaphore grants in wait order, so no
    // second structure decides this.
    let registry = registry(one_slot());
    let root = registry.new_tree();
    let first = root.admit_child("a", CancelToken::new()).unwrap();
    let early = match root.admit_child("early", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };
    let early_id = early.id();
    let early_waiter = tokio::spawn(async move { early.started().await });

    // The later waiter only begins to wait after the earlier one, which is what
    // "first in" means here.
    let late = match root.admit_child("late", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };
    assert_eq!(
        late.position(),
        2,
        "the later child waits behind the earlier"
    );

    drop(first);
    let started = tokio::time::timeout(Duration::from_secs(5), early_waiter)
        .await
        .expect("no hang")
        .expect("no panic")
        .expect("the earlier waiter starts first");
    assert_eq!(started.node.id(), early_id);
}

#[tokio::test]
async fn spawn_child_still_refuses_over_a_cap() {
    // The immediate form is unchanged, and it is the bypass a Rust caller uses.
    let registry = registry(one_slot());
    let root = registry.new_tree();
    let _first = root.spawn_child("scout", CancelToken::new()).unwrap();
    let refusal = root
        .spawn_child("scout", CancelToken::new())
        .expect_err("spawn_child never queues");
    assert!(matches!(
        refusal,
        SubagentError::TooManyChildren { limit: 1, .. }
    ));
}

#[tokio::test]
async fn depth_beyond_the_cap_refuses_and_never_queues() {
    // Waiting adds no depth, so this cap cannot be fixed by waiting.
    let limits = SubagentLimits {
        max_depth: 1,
        ..SubagentLimits::new()
    };
    let registry = registry(limits);
    let root = registry.new_tree();
    let child = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Started(spawn) => spawn,
        Admission::Queued(_) => panic!("the first child has a slot"),
    };
    let refusal = child
        .node
        .admit_child("grandchild", CancelToken::new())
        .expect_err("depth must refuse");
    assert!(matches!(
        refusal,
        SubagentError::DepthExceeded {
            limit: 1,
            attempted: 2
        }
    ));
}

#[tokio::test]
async fn a_full_wait_line_refuses_and_names_the_limit() {
    // The wait line is bounded, because a queued child holds a cancel token and a
    // message queue. An unbounded line is a memory defect this project shipped once.
    let limits = SubagentLimits {
        max_children_per_parent: 1,
        max_queued_per_parent: 2,
        ..SubagentLimits::new()
    };
    let registry = registry(limits);
    let root = registry.new_tree();
    let _live = root.admit_child("scout", CancelToken::new()).unwrap();
    let _one = root.admit_child("scout", CancelToken::new()).unwrap();
    let _two = root.admit_child("scout", CancelToken::new()).unwrap();

    let refusal = root
        .admit_child("scout", CancelToken::new())
        .expect_err("a full line must refuse");
    let text = refusal.to_string();
    assert!(
        matches!(
            refusal,
            SubagentError::QueueFull {
                scope: QueueScope::Parent,
                limit: 2
            }
        ),
        "the refusal must name this parent's line: {refusal:?}"
    );
    assert!(
        text.contains("--max-queued-per-parent"),
        "the refusal must name the flag: {text}"
    );
}

#[tokio::test]
async fn a_full_process_wait_line_refuses_and_names_the_other_limit() {
    // A session root holds no live-child slot, so `max_live_total` bounds neither
    // the number of roots nor the number of lines. Without a process-wide line cap
    // the waiting total is the per-parent cap times an unbounded number of trees.
    let limits = SubagentLimits {
        max_children_per_parent: 1,
        max_queued_per_parent: 4,
        max_queued_total: 2,
        max_live_total: 32,
        ..SubagentLimits::new()
    };
    let registry = registry(limits);

    let mut trees = Vec::new();
    for _ in 0..3 {
        let tree = registry.new_tree();
        let _live = tree.admit_child("scout", CancelToken::new()).unwrap();
        trees.push((tree, _live));
    }

    // Two waiters fit in the process line. The third must be refused, even though
    // each parent's own line has room.
    let _first = trees[0].0.admit_child("scout", CancelToken::new()).unwrap();
    let _second = trees[1].0.admit_child("scout", CancelToken::new()).unwrap();
    let refusal = trees[2]
        .0
        .admit_child("scout", CancelToken::new())
        .expect_err("the process line must refuse");
    let text = refusal.to_string();
    assert!(
        matches!(
            refusal,
            SubagentError::QueueFull {
                scope: QueueScope::Process,
                limit: 2
            }
        ),
        "the refusal must name the process line: {refusal:?}"
    );
    assert!(
        text.contains("--max-queued-total"),
        "the refusal must name the flag: {text}"
    );
}

#[tokio::test]
async fn cancelling_a_queued_child_resolves_started_with_cancelled() {
    // A cancel reaches a child that never ran, and it frees the place.
    let registry = registry(one_slot());
    let root = registry.new_tree();
    let _live = root.admit_child("scout", CancelToken::new()).unwrap();
    let queued = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };
    queued.cancel();
    assert!(queued.is_cancelled());

    let outcome = tokio::time::timeout(Duration::from_secs(5), queued.started())
        .await
        .expect("a cancelled waiter must not hang");
    assert!(matches!(outcome, Err(Dequeued::Cancelled)));
}

#[tokio::test]
async fn cancelling_a_parent_dequeues_every_queued_child() {
    // An ancestor cancel stops every descendant, including the ones that never ran.
    let registry = registry(one_slot());
    let root = registry.new_tree();
    let parent_token = CancelToken::new();
    let _live = root.admit_child("scout", parent_token.child()).unwrap();

    let mut waiters = Vec::new();
    for _ in 0..3 {
        let queued = match root.admit_child("scout", parent_token.child()).unwrap() {
            Admission::Queued(queued) => queued,
            Admission::Started(_) => panic!("the cap is one"),
        };
        waiters.push(tokio::spawn(async move { queued.started().await }));
    }

    parent_token.cancel();
    for waiter in waiters {
        let outcome = tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .expect("no waiter may hang after a parent cancel")
            .expect("no panic");
        assert!(matches!(outcome, Err(Dequeued::Cancelled)));
    }
}

#[tokio::test]
async fn status_reports_a_queued_child_with_its_position() {
    // A queued child is addressable through the registry, or "pollable by id" is
    // false. The tools reach a child only through these scoped accessors.
    let registry = registry(one_slot());
    let root = registry.new_tree();
    let _live = root.admit_child("scout", CancelToken::new()).unwrap();
    let queued = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };

    let status = registry
        .status(&root, queued.id())
        .expect("a queued child must answer");
    match status {
        AgentStatus::Queued {
            agent,
            depth,
            position,
            cancelled,
        } => {
            assert_eq!(agent, "scout");
            assert_eq!(depth, 1);
            assert_eq!(position, 1);
            assert!(!cancelled, "a waiting child is not a cancelled one");
        }
        other => panic!("a queued child must report Queued: {other:?}"),
    }
}

#[tokio::test]
async fn a_queued_position_is_computed_and_never_stale() {
    // A stored place goes stale the moment a child ahead leaves, and a wrong number
    // is worse than none, because a model acts on it.
    let registry = registry(one_slot());
    let root = registry.new_tree();
    let live = root.admit_child("scout", CancelToken::new()).unwrap();
    let first = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };
    let second = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };
    assert_eq!(second.position(), 2, "it starts second in the line");

    // The child ahead gives up its place. The one behind moves up.
    first.cancel();
    let _ = first.started().await;
    drop(live);
    assert_eq!(
        second.position(),
        1,
        "the place must be computed, not remembered"
    );
}

#[tokio::test]
async fn status_reports_running_after_a_queued_child_starts() {
    let registry = registry(one_slot());
    let root = registry.new_tree();
    let live = root.admit_child("scout", CancelToken::new()).unwrap();
    let queued = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };
    let id = queued.id();
    let waiter = tokio::spawn(async move { queued.started().await });
    drop(live);
    let _started = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("no hang")
        .expect("no panic")
        .expect("the waiter starts");

    let status = registry.status(&root, id).expect("the child must answer");
    assert!(
        matches!(status, AgentStatus::Running { .. }),
        "a started child reports Running: {status:?}"
    );
}

#[tokio::test]
async fn a_queued_entry_leaves_the_map_when_the_child_starts() {
    // No id may sit in two indexes. Asserting through `status` alone would pass with
    // a stale entry, because `status` prefers the live answer. So this asserts
    // through the wait line instead: a stale entry would still occupy a place.
    let registry = registry(one_slot());
    let root = registry.new_tree();
    let live = root.admit_child("scout", CancelToken::new()).unwrap();
    let queued = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };
    let waiter = tokio::spawn(async move { queued.started().await });
    drop(live);
    let started = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("no hang")
        .expect("no panic")
        .expect("the waiter starts");

    // A new waiter must now be first in an otherwise empty line.
    let next = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };
    assert_eq!(
        next.position(),
        1,
        "the started child must have left the line"
    );
    drop(started);
}

#[tokio::test]
async fn a_queued_entry_leaves_the_map_when_the_child_is_cancelled() {
    let registry = registry(one_slot());
    let root = registry.new_tree();
    let _live = root.admit_child("scout", CancelToken::new()).unwrap();
    let queued = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };
    let id = queued.id();
    queued.cancel();
    let _ = queued.started().await;

    assert!(
        registry.status(&root, id).is_none(),
        "a cancelled queued child must leave no entry behind"
    );
}

#[tokio::test]
async fn a_dropped_queued_child_leaves_no_entry_behind() {
    // A caller may drop the value and never await it. Without a drop guard the entry
    // stays, the map grows with every spawn, and a dead id answers for ever.
    let registry = registry(one_slot());
    let root = registry.new_tree();
    let _live = root.admit_child("scout", CancelToken::new()).unwrap();
    let id = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => {
            let id = queued.id();
            drop(queued);
            id
        }
        Admission::Started(_) => panic!("the cap is one"),
    };

    assert!(
        registry.status(&root, id).is_none(),
        "a dropped queued child must leave no entry behind"
    );
}

#[tokio::test]
async fn one_tree_cannot_reach_another_queued_child_by_id() {
    // The escape a security review already found once for the live map. A queued
    // entry carries its ancestor chain, so a lookup stays scoped.
    let registry = registry(one_slot());
    let mine = registry.new_tree();
    let theirs = registry.new_tree();
    let _live = theirs.admit_child("scout", CancelToken::new()).unwrap();
    let queued = match theirs.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };

    assert!(
        registry.status(&mine, queued.id()).is_none(),
        "one tree must not see another tree's queued child"
    );
    assert!(
        !registry.cancel_descendant(&mine, queued.id()),
        "one tree must not cancel another tree's queued child"
    );
    assert!(
        !queued.is_cancelled(),
        "the child must still be waiting for its own tree"
    );
}

#[tokio::test]
async fn two_racing_starts_cannot_both_pass_a_cap_of_one() {
    // The property the old compare-and-swap loops protected. A semaphore keeps it,
    // and this test does not care which mechanism holds.
    let registry = registry(one_slot());
    let root = registry.new_tree();
    let started = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let mut tasks = Vec::new();
    for _ in 0..16 {
        let root = root.clone();
        let started = Arc::clone(&started);
        tasks.push(tokio::task::spawn_blocking(move || {
            if let Ok(Admission::Started(spawn)) = root.admit_child("scout", CancelToken::new()) {
                started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                std::mem::forget(spawn);
            }
        }));
    }
    for task in tasks {
        task.await.expect("no panic");
    }

    assert_eq!(
        started.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "a cap of one must admit exactly one running child"
    );
}

#[tokio::test]
async fn steering_a_queued_child_buffers_until_it_starts() {
    // A steer reaches a child that has not started, and the message survives the
    // handout. One entry point serves a live child and a queued one, so the two
    // cannot drift apart.
    let registry = registry(one_slot());
    let root = registry.new_tree();
    let live = root.admit_child("scout", CancelToken::new()).unwrap();
    let queued = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };
    let id = queued.id();

    let delivered = registry
        .steer_descendant(
            &root,
            id,
            vec![rho_core::ContentBlock::Text {
                text: "change course".to_string(),
            }],
        )
        .expect("a queued child must accept a steer")
        .expect("the queue has room");
    assert_eq!(delivered, 1, "the message waits in the child's own queue");

    let waiter = tokio::spawn(async move { queued.started().await });
    drop(live);
    let spawn = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("no hang")
        .expect("no panic")
        .expect("the waiter starts");
    assert_eq!(
        spawn.queue().len(),
        1,
        "the buffered message must survive the handout"
    );
}

#[tokio::test]
async fn one_tree_cannot_steer_another_queued_child() {
    let registry = registry(one_slot());
    let mine = registry.new_tree();
    let theirs = registry.new_tree();
    let _live = theirs.admit_child("scout", CancelToken::new()).unwrap();
    let queued = match theirs.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };

    assert!(
        registry
            .steer_descendant(
                &mine,
                queued.id(),
                vec![rho_core::ContentBlock::Text {
                    text: "not yours".to_string()
                }],
            )
            .is_none(),
        "one tree must not steer another tree's queued child"
    );
    assert_eq!(queued.queue().len(), 0, "nothing reached the child");
}

#[tokio::test]
async fn the_started_child_keeps_the_id_the_caller_was_given() {
    // The id in the admission is the id that runs. A handout that allocated a fresh
    // one would strand every steer and cancel the model had already been told about.
    let registry = registry(one_slot());
    let root = registry.new_tree();
    let live = root.admit_child("scout", CancelToken::new()).unwrap();
    let queued = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };
    let promised = queued.id();
    let waiter = tokio::spawn(async move { queued.started().await });
    drop(live);
    let spawn = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("no hang")
        .expect("no panic")
        .expect("the waiter starts");

    assert_eq!(
        spawn.node.id(),
        promised,
        "the child must run under the id the caller holds"
    );
    assert!(
        registry.descendant(&root, promised).is_some(),
        "and the live handle must answer to it"
    );
}

#[tokio::test]
async fn a_process_wide_refusal_leaves_no_queued_entry_behind() {
    // The ordering trap. `handed_out` must flip only after the process-wide permit is
    // held and the live handle exists. Set earlier, the refusal path skips the drop
    // guard and the entry answers "queued" for ever, which is the leak the flag was
    // added to prevent.
    let limits = SubagentLimits {
        max_children_per_parent: 1,
        max_live_total: 2,
        ..SubagentLimits::new()
    };
    let registry = registry(limits);
    let root = registry.new_tree();
    let first = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Started(spawn) => spawn,
        Admission::Queued(_) => panic!("the first child has a slot"),
    };
    let other = registry.new_tree();
    let _other_child = other.admit_child("scout", CancelToken::new()).unwrap();
    let queued = match root.admit_child("scout", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the per-parent cap is one"),
    };
    let id = queued.id();
    let waiter = tokio::spawn(async move { queued.started().await });

    drop(first);
    let third = registry.new_tree();
    let _thief = third
        .spawn_child("scout", CancelToken::new())
        .expect("a third tree takes the freed process-wide slot");
    let outcome = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("no hang")
        .expect("no panic");
    assert!(matches!(outcome, Err(Dequeued::ProcessWideFull { .. })));

    assert!(
        registry.status(&root, id).is_none(),
        "a refused waiter must leave no queued entry behind"
    );
}

#[tokio::test]
async fn a_position_counts_only_its_own_siblings() {
    // A place is per parent. Counting every waiter in the process would tell a child
    // it is eighth when it is first, and a model would then cancel useful work.
    let registry = registry(one_slot());
    let first_parent = registry.new_tree();
    let second_parent = registry.new_tree();
    let _live_one = first_parent
        .admit_child("scout", CancelToken::new())
        .unwrap();
    let _live_two = second_parent
        .admit_child("scout", CancelToken::new())
        .unwrap();

    // Three children wait under the first parent, and they queued earlier.
    let mut under_first = Vec::new();
    for _ in 0..3 {
        match first_parent
            .admit_child("scout", CancelToken::new())
            .unwrap()
        {
            Admission::Queued(queued) => under_first.push(queued),
            Admission::Started(_) => panic!("the cap is one"),
        }
    }
    // One child waits under the second parent, and it queued last.
    let lonely = match second_parent
        .admit_child("scout", CancelToken::new())
        .unwrap()
    {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };

    assert_eq!(
        lonely.position(),
        1,
        "it is first in its own parent's line, whatever another parent is doing"
    );
    assert_eq!(
        under_first[2].position(),
        3,
        "and the third under the other parent is third"
    );
}

#[tokio::test]
async fn admission_reports_started_when_a_slot_was_free_and_queued_when_it_was_not() {
    // Both arms, and every `QueuedChild` accessor. A public accessor no test reads is
    // where three defects hid in this project already. See AGENTS.md step 8.
    let registry = registry(one_slot());
    let root = registry.new_tree();

    let first = root
        .admit_child("scout", CancelToken::new())
        .expect("a free slot admits");
    let live_id = match first {
        Admission::Started(ref spawn) => spawn.node.id(),
        Admission::Queued(_) => panic!("the first child holds the only slot"),
    };

    let cancel = CancelToken::new();
    let second = root
        .admit_child("explore", cancel.clone())
        .expect("a full parent queues");
    let Admission::Queued(queued) = second else {
        panic!("a full per-parent cap must queue");
    };

    assert_ne!(queued.id(), live_id, "a waiter holds an id of its own");
    assert_eq!(queued.agent(), "explore", "it remembers what it will run");
    assert_eq!(queued.depth(), 1, "a child of the root waits at depth 1");
    assert_eq!(queued.position(), 1, "it is first in its parent's line");
    assert!(!queued.is_cancelled(), "a fresh waiter is not cancelled");
    assert_eq!(
        queued.queue().len(),
        0,
        "its queue exists and starts empty, so a steer buffers rather than fails"
    );

    cancel.cancel();
    assert!(
        queued.is_cancelled(),
        "the token the caller passed is the token that stops it"
    );
}

#[tokio::test]
async fn the_wait_line_does_not_grow_without_a_bound() {
    // A thousand admissions must not queue a thousand children. Each waiter holds a
    // cancel token and a message queue, so an unbounded line is a memory defect. This
    // asserts the bound, not one example of it. See decision D-bounded-slot-queue.
    let limits = SubagentLimits {
        max_children_per_parent: 1,
        max_queued_per_parent: 16,
        ..SubagentLimits::new()
    };
    let registry = registry(limits);
    let root = registry.new_tree();

    let mut held = Vec::new();
    let mut refused = 0;
    for _ in 0..1000 {
        match root.admit_child("scout", CancelToken::new()) {
            Ok(admission) => held.push(admission),
            Err(SubagentError::QueueFull { limit, .. }) => {
                assert_eq!(limit, 16, "the refusal names the bound it enforced");
                refused += 1;
            }
            Err(other) => panic!("only a full line may refuse here: {other:?}"),
        }
    }

    assert_eq!(
        held.len(),
        17,
        "one child runs and sixteen wait, whatever the caller asks for"
    );
    assert_eq!(refused, 983, "every admission over the bound is refused");
}

#[tokio::test]
async fn status_says_a_cancelled_queued_child_will_not_start() {
    // Found on live Bedrock, not by a test. A cancel wakes the waiter, and the entry
    // leaves the map when that task drops it. In between, `status` reported a place in
    // the line and promised the child would start. Both were false. See decision
    // D-a-cancelled-waiter-says-so.
    let registry = registry(one_slot());
    let root = registry.new_tree();
    let _live = root
        .admit_child("scout", CancelToken::new())
        .expect("the first child holds the only slot");

    let cancel = CancelToken::new();
    let waiter = root
        .admit_child("scout", cancel.clone())
        .expect("a full parent queues");
    let Admission::Queued(queued) = waiter else {
        panic!("a full per-parent cap must queue");
    };

    match registry.status(&root, queued.id()) {
        Some(AgentStatus::Queued { cancelled, .. }) => {
            assert!(!cancelled, "a fresh waiter has not been cancelled")
        }
        other => panic!("a queued child reports queued, got {other:?}"),
    }

    cancel.cancel();

    match registry.status(&root, queued.id()) {
        Some(AgentStatus::Queued { cancelled, .. }) => {
            assert!(
                cancelled,
                "the state must carry the cancel, or the tool cannot report it"
            )
        }
        other => panic!("the entry is still in the map until its waiter drops, got {other:?}"),
    }
}
