//! Named handles: a second address for a child, derived from its agent name.
//!
//! Tests for `SPEC-subagent-slots-handles-grace` section 3. The id stays the identity.
//! A handle is a name a model can hold in its head, and it never crosses a tree.
//! Nothing here talks to a provider, because the subject is naming and not a run.

use std::sync::Arc;

use rho_core::{
    Admission, AgentId, AgentRef, AgentRegistry, AliasError, CancelToken, MAX_ALIAS_LENGTH,
    SubagentLimits,
};

fn registry(limits: SubagentLimits) -> AgentRegistry {
    AgentRegistry::new(limits)
}

/// A report for a child that finished, so the remembered index can be filled.
fn report(agent: &str) -> rho_core::AgentReport {
    rho_core::AgentReport {
        agent: agent.to_string(),
        outcome: rho_core::AgentOutcome::Done,
        summary: "done".to_string(),
        usage: Default::default(),
        turns: 1,
        gate: Default::default(),
        claims: Default::default(),
        transcript: None,
    }
}

#[tokio::test]
async fn a_handle_is_derived_from_the_agent_name() {
    // The first child of an agent takes the agent's own name. A model that read the
    // name in the spawn result can then use it without keeping a number.
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();
    let spawn = root.spawn_child("explore", CancelToken::new()).unwrap();

    assert_eq!(
        registry.handle_of(spawn.node.id()).as_deref(),
        Some("explore"),
        "the first child of one agent name is that name"
    );
}

#[tokio::test]
async fn a_second_child_of_one_name_is_numbered() {
    // Two children of one agent cannot share a name, or one name would address two
    // children and `resolve` would pick one of them in silence.
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();
    let first = root.spawn_child("explore", CancelToken::new()).unwrap();
    let second = root.spawn_child("explore", CancelToken::new()).unwrap();
    let third = root.spawn_child("explore", CancelToken::new()).unwrap();

    assert_eq!(
        registry.handle_of(first.node.id()).as_deref(),
        Some("explore")
    );
    assert_eq!(
        registry.handle_of(second.node.id()).as_deref(),
        Some("explore-2")
    );
    assert_eq!(
        registry.handle_of(third.node.id()).as_deref(),
        Some("explore-3")
    );
}

#[tokio::test]
async fn a_handle_is_unique_per_tree_not_per_process() {
    // One registry may hold several unrelated sessions. Each tree numbers from one, or
    // a busy neighbour would push this tree's names up for no reason the caller can see.
    let registry = registry(SubagentLimits::new());
    let tree_a = registry.new_tree();
    let tree_b = registry.new_tree();

    let a_child = tree_a.spawn_child("explore", CancelToken::new()).unwrap();
    let b_child = tree_b.spawn_child("explore", CancelToken::new()).unwrap();

    assert_eq!(
        registry.handle_of(a_child.node.id()).as_deref(),
        Some("explore")
    );
    assert_eq!(
        registry.handle_of(b_child.node.id()).as_deref(),
        Some("explore"),
        "a second tree starts its own numbering"
    );
    assert_ne!(a_child.node.id(), b_child.node.id(), "the ids still differ");
}

#[tokio::test]
async fn a_queued_child_holds_a_handle_too() {
    // A queued child is addressable by id, so it must be addressable by name. A handle
    // derived only at start would leave the model with a name that reached nothing.
    let limits = SubagentLimits {
        max_children_per_parent: 1,
        ..SubagentLimits::new()
    };
    let registry = registry(limits);
    let root = registry.new_tree();
    let _live = root.admit_child("explore", CancelToken::new()).unwrap();
    let waiter = match root.admit_child("explore", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };

    assert_eq!(
        registry.handle_of(waiter.id()).as_deref(),
        Some("explore-2"),
        "a waiter takes the next name, so a live child and a waiter never share one"
    );
    assert_eq!(
        registry.resolve(&root, &AgentRef::Name("explore-2".to_string())),
        Some(waiter.id()),
        "and the name reaches it while it waits"
    );
}

#[tokio::test]
async fn a_started_child_keeps_the_handle_it_was_given_while_queued() {
    // The handout must not re-derive. A fresh name at start would strand the name the
    // model was told, exactly as a fresh id once stranded every steer and cancel.
    let limits = SubagentLimits {
        max_children_per_parent: 1,
        ..SubagentLimits::new()
    };
    let registry = registry(limits);
    let root = registry.new_tree();
    let live = root.admit_child("explore", CancelToken::new()).unwrap();
    let waiter = match root.admit_child("explore", CancelToken::new()).unwrap() {
        Admission::Queued(queued) => queued,
        Admission::Started(_) => panic!("the cap is one"),
    };
    let promised = registry
        .handle_of(waiter.id())
        .expect("a waiter has a handle");
    drop(live);

    let started = waiter
        .started()
        .await
        .expect("a freed slot starts the waiter");
    assert_eq!(
        registry.handle_of(started.node.id()).as_deref(),
        Some(promised.as_str()),
        "the name survives the start"
    );
}

#[test]
fn sixteen_threads_admitting_one_agent_name_get_sixteen_distinct_handles() {
    // The set size is the assertion, so this proves the invariant and not one lucky
    // interleave. It fails if a handle is derived outside the registration lock.
    let limits = SubagentLimits {
        max_children_per_parent: 16,
        max_live_total: 64,
        ..SubagentLimits::new()
    };
    let registry = AgentRegistry::new(limits);
    let root = registry.new_tree();

    let mut threads = Vec::new();
    for _ in 0..16 {
        let root = root.clone();
        threads.push(std::thread::spawn(move || {
            let spawn = root
                .spawn_child("explore", CancelToken::new())
                .expect("sixteen slots for sixteen children");
            let id = spawn.node.id();
            // Hold the reservation, so no handle is pruned before the check.
            (id, spawn)
        }));
    }

    let mut held = Vec::new();
    let mut handles = std::collections::HashSet::new();
    for thread in threads {
        let (id, spawn) = thread.join().expect("no thread may panic");
        handles.insert(
            registry
                .handle_of(id)
                .expect("every registered child has a handle"),
        );
        held.push(spawn);
    }

    assert_eq!(
        handles.len(),
        16,
        "sixteen children of one agent name need sixteen names: {handles:?}"
    );
}

#[tokio::test]
async fn a_handle_is_not_reused_while_a_finished_child_is_remembered() {
    // A finished child still answers `agent_status` while its report is in the ring, so
    // its name must not be handed to a newcomer. The numbering reads all three indexes.
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();

    let first = root.spawn_child("explore", CancelToken::new()).unwrap();
    let first_id = first.node.id();
    registry.record_report(&root, first_id, report("explore"));
    // The slot drops, so the live handle goes and only the report remains.
    drop(first);

    let second = root.spawn_child("explore", CancelToken::new()).unwrap();
    assert_eq!(
        registry.handle_of(first_id).as_deref(),
        Some("explore"),
        "the remembered child keeps its name"
    );
    assert_eq!(
        registry.handle_of(second.node.id()).as_deref(),
        Some("explore-2"),
        "so the newcomer takes the next one"
    );
}

#[tokio::test]
async fn resolve_reads_an_integer_id() {
    // The old shape must keep working, because a model already writes integers.
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();
    let spawn = root.spawn_child("explore", CancelToken::new()).unwrap();

    assert_eq!(
        registry.resolve(&root, &AgentRef::Id(spawn.node.id().0)),
        Some(spawn.node.id())
    );
}

#[tokio::test]
async fn resolve_reads_a_digits_only_string_as_an_id() {
    // A model that quotes the number must reach the same child as one that does not.
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();
    let spawn = root.spawn_child("explore", CancelToken::new()).unwrap();
    let quoted = spawn.node.id().0.to_string();

    assert_eq!(
        registry.resolve(&root, &AgentRef::Name(quoted)),
        Some(spawn.node.id())
    );
}

#[tokio::test]
async fn a_digits_only_name_resolves_as_an_id_and_never_as_a_handle() {
    // An agent may be called `42`. Its derived handle is still `42` for display, and a
    // model that sends "42" reaches the child whose id is 42. No child is unreachable,
    // because the id always reaches it.
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();
    let numeric = root.spawn_child("42", CancelToken::new()).unwrap();

    assert_eq!(
        registry.handle_of(numeric.node.id()).as_deref(),
        Some("42"),
        "the derived handle is kept for display"
    );

    // The child's own id is not 42, so a digits-only name must not reach it.
    assert_ne!(numeric.node.id(), AgentId(42));
    assert_eq!(
        registry.resolve(&root, &AgentRef::Name("42".to_string())),
        None,
        "a digits-only name is an id, and no child here holds id 42"
    );

    // And an alias may not be digits only, or it could never be reached either.
    let refusal = registry
        .set_alias(&root, numeric.node.id(), "7")
        .expect_err("a digits-only alias must be refused");
    assert!(matches!(refusal, AliasError::DigitsOnly { .. }));
}

#[tokio::test]
async fn resolve_reads_a_handle_name() {
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();
    let first = root.spawn_child("explore", CancelToken::new()).unwrap();
    let second = root.spawn_child("explore", CancelToken::new()).unwrap();

    assert_eq!(
        registry.resolve(&root, &AgentRef::Name("explore".to_string())),
        Some(first.node.id())
    );
    assert_eq!(
        registry.resolve(&root, &AgentRef::Name("explore-2".to_string())),
        Some(second.node.id())
    );
}

#[tokio::test]
async fn an_alias_resolves_to_its_child() {
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();
    let spawn = root.spawn_child("explore", CancelToken::new()).unwrap();

    registry
        .set_alias(&root, spawn.node.id(), "auth-audit")
        .expect("a fresh name is free");
    assert_eq!(
        registry.resolve(&root, &AgentRef::Name("auth-audit".to_string())),
        Some(spawn.node.id())
    );
}

#[tokio::test]
async fn an_alias_that_shadows_a_handle_is_refused() {
    // A derived handle is rho's own naming. An alias that could hide one would let a
    // model make another child unreachable by name.
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();
    let first = root.spawn_child("explore", CancelToken::new()).unwrap();
    let second = root.spawn_child("scout", CancelToken::new()).unwrap();

    let refusal = registry
        .set_alias(&root, second.node.id(), "explore")
        .expect_err("a derived handle wins");
    assert!(
        matches!(refusal, AliasError::ShadowsHandle { ref name } if name == "explore"),
        "the refusal must name the clash: {refusal:?}"
    );
    assert_eq!(
        registry.resolve(&root, &AgentRef::Name("explore".to_string())),
        Some(first.node.id()),
        "and the derived handle still reaches its own child"
    );
}

#[tokio::test]
async fn an_alias_that_is_already_taken_is_refused() {
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();
    let first = root.spawn_child("explore", CancelToken::new()).unwrap();
    let second = root.spawn_child("scout", CancelToken::new()).unwrap();

    registry
        .set_alias(&root, first.node.id(), "auditor")
        .unwrap();
    let refusal = registry
        .set_alias(&root, second.node.id(), "auditor")
        .expect_err("one name, one owner");
    assert!(matches!(refusal, AliasError::Taken { ref name } if name == "auditor"));
    assert_eq!(
        registry.resolve(&root, &AgentRef::Name("auditor".to_string())),
        Some(first.node.id()),
        "the first owner keeps the name"
    );
}

#[tokio::test]
async fn resolve_checks_a_handle_before_an_alias() {
    // The shadow rule holds at read time too, not only at write time. A tree that set
    // an alias before a later child derived the same handle must not hide that child.
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();
    let aliased = root.spawn_child("scout", CancelToken::new()).unwrap();
    registry
        .set_alias(&root, aliased.node.id(), "explore")
        .expect("no handle holds this name yet");

    // Now a child derives the handle `explore`.
    let derived = root.spawn_child("explore", CancelToken::new()).unwrap();

    assert_eq!(
        registry.resolve(&root, &AgentRef::Name("explore".to_string())),
        Some(derived.node.id()),
        "the derived handle wins the read, so no child becomes unreachable"
    );
}

#[tokio::test]
async fn an_alias_longer_than_the_cap_is_refused_and_the_cap_is_counted_in_characters() {
    // The model writes an alias, and rho stores one per child. A bound in characters,
    // not bytes, so an emoji costs one and not four.
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();
    let spawn = root.spawn_child("explore", CancelToken::new()).unwrap();

    let exact: String = "\u{1f600}".repeat(MAX_ALIAS_LENGTH);
    registry
        .set_alias(&root, spawn.node.id(), exact.clone())
        .expect("64 characters is the cap, and the cap is allowed");

    let over: String = "a".repeat(MAX_ALIAS_LENGTH + 1);
    let refusal = registry
        .set_alias(&root, spawn.node.id(), over)
        .expect_err("one character more must be refused");
    assert!(
        matches!(
            refusal,
            AliasError::TooLong {
                limit: MAX_ALIAS_LENGTH,
                length: 65
            }
        ),
        "the refusal must state the bound and what arrived: {refusal:?}"
    );
}

#[tokio::test]
async fn an_alias_with_a_control_character_is_refused() {
    // An alias is echoed into the parent's context beside rho's own lines. A newline
    // would let a prompt-injected model forge one of those lines.
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();
    let spawn = root.spawn_child("explore", CancelToken::new()).unwrap();

    for bad in ["two\nlines", "bell\u{7}", "escape\u{1b}[31m"] {
        let refusal = registry
            .set_alias(&root, spawn.node.id(), bad)
            .expect_err("a control character must be refused");
        assert!(
            matches!(refusal, AliasError::NotPrintable { .. }),
            "expected NotPrintable for {bad:?}, got {refusal:?}"
        );
    }
}

#[tokio::test]
async fn an_alias_for_a_child_of_another_tree_is_unknown() {
    // The scope guard covers the write path too, or one session could name another
    // session's child and then address it.
    let registry = registry(SubagentLimits::new());
    let mine = registry.new_tree();
    let theirs = registry.new_tree();
    let stranger = theirs.spawn_child("explore", CancelToken::new()).unwrap();

    let refusal = registry
        .set_alias(&mine, stranger.node.id(), "mine-now")
        .expect_err("a stranger's child is not addressable");
    assert!(matches!(refusal, AliasError::Unknown { .. }));
    assert_eq!(
        registry.resolve(&mine, &AgentRef::Name("mine-now".to_string())),
        None,
        "and no binding was written"
    );
}

#[tokio::test]
async fn one_tree_cannot_reach_another_by_handle() {
    // Two guards stack: the handle table is keyed per tree, and `resolve` re-checks the
    // resolved id against the caller's own descendants.
    let registry = registry(SubagentLimits::new());
    let mine = registry.new_tree();
    let theirs = registry.new_tree();
    let stranger = theirs.spawn_child("explore", CancelToken::new()).unwrap();

    assert_eq!(
        registry.resolve(&mine, &AgentRef::Name("explore".to_string())),
        None,
        "a handle must not cross a tree"
    );
    assert_eq!(
        registry.resolve(&mine, &AgentRef::Id(stranger.node.id().0)),
        None,
        "and neither must a bare id"
    );
}

#[tokio::test]
async fn a_handle_resolves_while_the_report_is_remembered() {
    // A background child finishes while its parent is busy. Its name must outlive its
    // slot, or the parent could poll by id and not by the name it was given.
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();
    let spawn = root.spawn_child("explore", CancelToken::new()).unwrap();
    let id = spawn.node.id();
    registry.record_report(&root, id, report("explore"));
    drop(spawn);

    assert_eq!(
        registry.resolve(&root, &AgentRef::Name("explore".to_string())),
        Some(id),
        "the name still reaches the finished child"
    );
}

#[tokio::test]
async fn a_handle_stops_resolving_when_the_report_is_evicted() {
    // The remembered ring is bounded at 64, oldest dropped first. A handle table that
    // kept every name would be an unbounded map keyed by model-chosen agent names.
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();
    let spawn = root.spawn_child("explore", CancelToken::new()).unwrap();
    let id = spawn.node.id();
    registry.record_report(&root, id, report("explore"));
    drop(spawn);

    // Push the first report out of the ring.
    for _ in 0..70 {
        let filler = root.spawn_child("filler", CancelToken::new()).unwrap();
        let filler_id = filler.node.id();
        registry.record_report(&root, filler_id, report("filler"));
        drop(filler);
    }

    assert_eq!(
        registry.handle_of(id),
        None,
        "the binding goes with the report"
    );
    assert_eq!(
        registry.resolve(&root, &AgentRef::Name("explore".to_string())),
        None,
        "so the name resolves for exactly as long as the report answers"
    );
}

#[tokio::test]
async fn an_alias_goes_when_its_child_is_forgotten() {
    // An alias is stored per tree. Without eviction the map would grow with every
    // finished child, keyed by a name the model chose. See decision D-bash-line-cap.
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();
    let spawn = root.spawn_child("explore", CancelToken::new()).unwrap();
    let id = spawn.node.id();
    registry.set_alias(&root, id, "auditor").unwrap();
    drop(spawn);

    assert_eq!(
        registry.resolve(&root, &AgentRef::Name("auditor".to_string())),
        None,
        "a child with no live handle and no report is gone, and so is its alias"
    );
    let fresh = root.spawn_child("scout", CancelToken::new()).unwrap();
    registry
        .set_alias(&root, fresh.node.id(), "auditor")
        .expect("the freed name may be used again");
}

#[test]
fn an_agent_ref_reads_a_number_and_a_string() {
    // The wire shape. A JSON number is an id, and a JSON string is a name.
    let by_number: AgentRef =
        serde_json::from_value(serde_json::json!(7)).expect("a number is an id");
    assert_eq!(by_number, AgentRef::Id(7));

    let by_name: AgentRef =
        serde_json::from_value(serde_json::json!("explore-2")).expect("a string is a name");
    assert_eq!(by_name, AgentRef::Name("explore-2".to_string()));
}

#[test]
fn a_malformed_agent_ref_names_both_accepted_shapes() {
    // serde's own untagged message is "data did not match any variant", which teaches
    // nothing. Every refusal here must name both shapes and show what arrived.
    for bad in [
        serde_json::json!(true),
        serde_json::json!(1.5),
        serde_json::json!(-3),
        serde_json::json!(null),
        serde_json::json!({ "id": 7 }),
        serde_json::json!([7]),
    ] {
        let refusal = serde_json::from_value::<AgentRef>(bad.clone())
            .expect_err(&format!("{bad} is neither an id nor a name"));
        let text = refusal.to_string();
        assert!(
            text.contains("subagent id") && text.contains("handle"),
            "the refusal must name both accepted shapes, got: {text}"
        );
        assert!(
            text.contains("agent_status"),
            "and it must say how to list what is running, got: {text}"
        );
    }
}

#[test]
fn a_number_beyond_u64_is_refused_as_no_id() {
    // serde reads it as a float, so it is not an id. A silent truncation to u64::MAX
    // would address a child that does not exist.
    let refusal = serde_json::from_str::<AgentRef>("18446744073709551616")
        .expect_err("a number past u64 is not an id");
    assert!(refusal.to_string().contains("subagent id"));
}

#[test]
fn an_empty_agent_ref_name_is_an_ordinary_not_found() {
    // An empty string parses. It matches no child, which is a result and not a fault.
    let parsed: AgentRef = serde_json::from_value(serde_json::json!("")).expect("a string parses");
    assert_eq!(parsed, AgentRef::Name(String::new()));

    let registry = AgentRegistry::new(SubagentLimits::new());
    let root = registry.new_tree();
    let _spawn = root.spawn_child("explore", CancelToken::new()).unwrap();
    assert_eq!(registry.resolve(&root, &parsed), None);
}

#[tokio::test]
async fn every_alias_refusal_teaches_what_to_do() {
    // A refusal a model cannot act on wastes a turn. Each variant must name the reason.
    let registry = registry(SubagentLimits::new());
    let root = registry.new_tree();
    let spawn = root.spawn_child("explore", CancelToken::new()).unwrap();
    let id = spawn.node.id();
    registry.set_alias(&root, id, "taken-name").unwrap();

    let cases: Vec<AliasError> = vec![
        registry.set_alias(&root, id, "explore").unwrap_err(),
        registry
            .set_alias(&root, spawn.node.id(), "taken-name")
            .unwrap_err(),
        registry
            .set_alias(&root, AgentId(9999), "ghost")
            .unwrap_err(),
        registry
            .set_alias(&root, id, "x".repeat(MAX_ALIAS_LENGTH + 1))
            .unwrap_err(),
        registry.set_alias(&root, id, "bad\nname").unwrap_err(),
        registry.set_alias(&root, id, "12345").unwrap_err(),
    ];

    for refusal in cases {
        let text = refusal.to_string();
        assert!(
            text.len() > 20 && text.ends_with('.'),
            "every refusal is a sentence that teaches: {text:?}"
        );
    }
}

/// A live child is needed to prove the handle table does not leak, and `Arc` keeps the
/// reservations alive for the length of the check.
#[tokio::test]
async fn the_handle_table_holds_no_more_than_the_three_indexes() {
    // The bound is the point: the table is the union of live, queued, and remembered,
    // so it cannot outgrow them. A thousand short-lived children must leave nothing.
    let registry = registry(SubagentLimits::new());
    let root = Arc::new(registry.new_tree());

    for _ in 0..1000 {
        let spawn = root.spawn_child("explore", CancelToken::new()).unwrap();
        // No report, so nothing remembers it once the slot drops.
        drop(spawn);
    }

    assert_eq!(
        registry.resolve(&root, &AgentRef::Name("explore".to_string())),
        None,
        "no name survives a child that nothing remembers"
    );
    let fresh = root.spawn_child("explore", CancelToken::new()).unwrap();
    assert_eq!(
        registry.handle_of(fresh.node.id()).as_deref(),
        Some("explore"),
        "and the numbering starts again from the plain name"
    );
}

#[tokio::test]
async fn a_child_cannot_reach_its_uncles_child_by_handle() {
    // Found by a mutation, not by reading. The handle table is keyed by the **tree**, and
    // every node of one tree shares that key. So the per-tree key alone does not scope a
    // name: a name lookup inside one session finds a cousin's binding. The second guard,
    // the ancestor re-check inside `resolve`, is the only thing that stops it. Deleting
    // that re-check passed every other test here. See decision
    // D-a-caller-addresses-only-its-own and section 3.4.
    let registry = registry(SubagentLimits {
        max_depth: 4,
        ..SubagentLimits::new()
    });
    let root = registry.new_tree();
    let uncle = root.spawn_child("uncle", CancelToken::new()).unwrap();
    let parent = root.spawn_child("parent", CancelToken::new()).unwrap();

    // The uncle's own child, in the same tree as `parent`.
    let cousin = uncle
        .node
        .spawn_child("explore", CancelToken::new())
        .unwrap();
    let cousin_handle = registry
        .handle_of(cousin.node.id())
        .expect("a registered child has a handle");

    // The parent may not address it, by name or by id, because it is not below it.
    assert_eq!(
        registry.resolve(&parent.node, &AgentRef::Name(cousin_handle.clone())),
        None,
        "a name must not reach a child that is not a descendant of the caller"
    );
    assert_eq!(
        registry.resolve(&parent.node, &AgentRef::Id(cousin.node.id().0)),
        None,
        "and neither must the bare id"
    );

    // The uncle, which is above it, still can.
    assert_eq!(
        registry.resolve(&uncle.node, &AgentRef::Name(cousin_handle)),
        Some(cousin.node.id()),
        "the owner keeps its own reach"
    );
}

#[tokio::test]
async fn an_alias_cannot_be_set_on_a_cousin() {
    // The write path needs the same guard as the read path, or one branch of a session
    // could name another branch's child and then address it by that name.
    let registry = registry(SubagentLimits {
        max_depth: 4,
        ..SubagentLimits::new()
    });
    let root = registry.new_tree();
    let uncle = root.spawn_child("uncle", CancelToken::new()).unwrap();
    let parent = root.spawn_child("parent", CancelToken::new()).unwrap();
    let cousin = uncle
        .node
        .spawn_child("explore", CancelToken::new())
        .unwrap();

    let refusal = registry
        .set_alias(&parent.node, cousin.node.id(), "not-mine")
        .expect_err("a cousin is not addressable");
    assert!(matches!(refusal, AliasError::Unknown { .. }));
}
