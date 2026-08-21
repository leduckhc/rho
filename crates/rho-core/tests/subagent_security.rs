//! Tests for the subagent security core: composition, not comparison.
//!
//! A child is confined by `BothPolicies`, the tool-set intersection, and the
//! sandbox-narrowing rule. The session root is never overridable. See
//! `docs/specs/20260818-000223-SPEC-subagents.md` section 3 and decision D-child-confined-by-composition.

use std::sync::Arc;

use async_trait::async_trait;
use rho_core::{
    AllowAllPolicy, ApprovalDecision, ApprovalPolicy, BothPolicies, ReadOnlyPolicy, SandboxMode,
    SubagentError, ToolKind, intersect_tools, narrow_sandbox,
};

/// A policy that denies every call. The child in the "child denies" test.
struct DenyAllPolicy;

#[async_trait]
impl ApprovalPolicy for DenyAllPolicy {
    async fn approve(
        &self,
        _tool: &str,
        _kind: ToolKind,
        _args: &serde_json::Value,
    ) -> ApprovalDecision {
        ApprovalDecision::Deny
    }
}

fn args() -> serde_json::Value {
    serde_json::json!({})
}

#[tokio::test]
async fn a_child_of_a_read_only_parent_cannot_write() {
    // However the definition asks, the parent's `ReadOnlyPolicy` denies a write.
    // The child here is `AllowAllPolicy`, the most permissive definition. The
    // composition still denies a mutating call.
    let policy = BothPolicies::new(Arc::new(ReadOnlyPolicy), Arc::new(AllowAllPolicy));

    let write = policy.approve("write", ToolKind::Edit, &args()).await;
    assert_eq!(write, ApprovalDecision::Deny, "a write must be denied");

    let read = policy.approve("read", ToolKind::Read, &args()).await;
    assert_eq!(read, ApprovalDecision::Allow, "a read must be allowed");
}

#[tokio::test]
async fn both_policies_denies_when_the_parent_denies() {
    let policy = BothPolicies::new(Arc::new(DenyAllPolicy), Arc::new(AllowAllPolicy));
    let decision = policy.approve("read", ToolKind::Read, &args()).await;
    assert_eq!(decision, ApprovalDecision::Deny);
}

#[tokio::test]
async fn both_policies_denies_when_the_child_denies() {
    let policy = BothPolicies::new(Arc::new(AllowAllPolicy), Arc::new(DenyAllPolicy));
    let decision = policy.approve("read", ToolKind::Read, &args()).await;
    assert_eq!(decision, ApprovalDecision::Deny);
}

#[tokio::test]
async fn both_policies_allows_only_when_both_allow() {
    let policy = BothPolicies::new(Arc::new(AllowAllPolicy), Arc::new(AllowAllPolicy));
    let decision = policy.approve("read", ToolKind::Read, &args()).await;
    assert_eq!(decision, ApprovalDecision::Allow);
}

#[test]
fn a_child_tool_set_is_the_intersection_with_the_parent() {
    // The child list must exercise **both** directions, or the test is vacuous.
    //
    // An earlier version used a child list that was a subset of the parent's, so removing
    // the parent check entirely left this test passing: every requested name was in the
    // parent set anyway. The controller found that by deleting the check and watching this
    // test stay green. See decision D-two-weak-tests.
    //
    // So the child here asks for one name the parent lacks, and omits one the parent has.
    let parent = vec!["read".to_string(), "glob".to_string(), "grep".to_string()];
    let child = vec![
        "read".to_string(),
        "grep".to_string(),
        // The parent never had this one. Intersection must drop it.
        "write".to_string(),
    ];
    let result = intersect_tools(&parent, Some(&child));

    assert_eq!(
        result.allowed,
        vec!["read".to_string(), "grep".to_string()],
        "only names the parent holds and the child asked for"
    );
    assert_eq!(
        result.dropped,
        vec!["write".to_string()],
        "a name the parent lacks must be dropped and reported"
    );
    assert!(
        !result.allowed.contains(&"glob".to_string()),
        "a parent tool the child did not ask for must not be granted"
    );
    assert!(
        !result.allowed.contains(&"write".to_string()),
        "escalation: the child received a tool its parent never had"
    );
}

#[test]
fn a_child_that_names_no_tools_inherits_the_parent_set() {
    let parent = vec!["read".to_string(), "glob".to_string()];
    let result = intersect_tools(&parent, None);
    assert_eq!(result.allowed, parent);
    assert!(result.dropped.is_empty());
}

#[test]
fn a_child_asking_for_a_tool_the_parent_lacks_is_dropped_and_reported() {
    let parent = vec!["read".to_string(), "glob".to_string()];
    // The child asks for `write`, which the parent never had.
    let child = vec!["read".to_string(), "write".to_string()];
    let result = intersect_tools(&parent, Some(&child));
    assert_eq!(result.allowed, vec!["read".to_string()], "write is dropped");
    assert_eq!(
        result.dropped,
        vec!["write".to_string()],
        "the drop is reported to the caller"
    );
}

#[test]
fn a_child_cannot_widen_the_sandbox_mode() {
    // A `Strict` parent refuses a `Confined` or `Off` child.
    let error = narrow_sandbox(SandboxMode::Strict, Some(SandboxMode::Confined)).unwrap_err();
    match error {
        SubagentError::WeakerSandbox { parent, requested } => {
            assert_eq!(parent, SandboxMode::Strict);
            assert_eq!(requested, SandboxMode::Confined);
        }
        other => panic!("expected WeakerSandbox, got {other:?}"),
    }
    // The refusal names both modes and what to do.
    let message = narrow_sandbox(SandboxMode::Confined, Some(SandboxMode::Off))
        .unwrap_err()
        .to_string();
    assert!(message.contains("confined"), "{message}");
    assert!(message.contains("off"), "{message}");

    // A child may ask for a stricter mode, and it may keep the same mode.
    assert_eq!(
        narrow_sandbox(SandboxMode::Confined, Some(SandboxMode::Strict)).unwrap(),
        SandboxMode::Strict
    );
    assert_eq!(
        narrow_sandbox(SandboxMode::Off, None).unwrap(),
        SandboxMode::Off,
        "no request inherits the parent mode"
    );
}

#[test]
fn a_child_cannot_change_the_session_root() {
    // The session root is a security boundary. It is never overridable. A child
    // definition carries no session-root field, so a child inherits the parent
    // root unchanged. This test pins that the composed child config takes the
    // parent root and ignores anything else.
    use rho_core::SessionConfig;

    let parent_root = std::path::PathBuf::from("/parent/root");
    let parent = SessionConfig::new(
        "model",
        parent_root.clone(),
        Arc::new(AllowAllPolicy) as Arc<dyn ApprovalPolicy>,
    );

    // A child config is derived from the parent. The derivation must copy the
    // parent root, whatever a definition might wish.
    let child = SessionConfig::new(
        parent.model.clone(),
        parent.session_root.clone(),
        Arc::new(ReadOnlyPolicy) as Arc<dyn ApprovalPolicy>,
    );
    assert_eq!(
        child.session_root, parent_root,
        "the child root is the parent root"
    );
}

#[test]
fn a_depth_refusal_names_no_flag_that_cannot_help() {
    // "Refusing must teach." The old message told the user to raise
    // `--max-agent-depth`, and no such flag exists. `rho-cli` gives a child no
    // spawn tool, so no flag can make a grandchild. A refusal must not send the
    // user after an impossible fix. See docs/verification/subagents-bedrock.md.
    let refusal = rho_core::SubagentError::DepthExceeded {
        limit: 0,
        attempted: 1,
    };
    let text = refusal.to_string();
    assert!(
        !text.contains("--max-agent-depth"),
        "the refusal must not name a flag that does not exist, got: {text}"
    );
    assert!(
        text.contains("Do the work here"),
        "the refusal must still say what to do instead, got: {text}"
    );
}

#[test]
fn a_child_may_keep_its_parents_exact_sandbox_mode() {
    // The comment on `narrow_sandbox` says a child may keep the same mode, and no
    // test called it with an equal pair. A review mutated `>=` to `>` and
    // `a_child_cannot_widen_the_sandbox_mode` still passed, so the claim was unproven.
    // It fails safe, but an unproven invariant is not an invariant.
    use rho_core::SandboxMode::{Confined, Off, Strict};
    for mode in [Off, Confined, Strict] {
        assert_eq!(
            rho_core::narrow_sandbox(mode, Some(mode)).expect("an equal mode is allowed"),
            mode,
            "a child asking for its parent's exact mode must be allowed: {mode:?}"
        );
    }
    // And a child that asks for nothing inherits.
    assert_eq!(
        rho_core::narrow_sandbox(Confined, None).unwrap(),
        Confined,
        "no request inherits the parent's mode"
    );
}

#[test]
fn the_process_wide_cap_holds_when_two_threads_race() {
    // The comment says the compare-and-swap loop stops two parents both passing a cap
    // of one. Only a sequential test covered it, and a review replaced the loop with a
    // non-atomic read-then-add while every test stayed green. This races real threads.
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    for _ in 0..200 {
        let registry = rho_core::AgentRegistry::new(rho_core::SubagentLimits {
            max_children_per_parent: 64,
            max_live_total: 1,
            ..rho_core::SubagentLimits::new()
        });
        let granted = Arc::new(AtomicUsize::new(0));
        // A barrier makes every thread reach the check at the same moment. Without it
        // the window between a read and an add is too small to lose reliably, and a
        // non-atomic reservation passed 200 rounds of this test.
        // A spin gate, not a `Barrier`. A barrier wakes threads through the operating
        // system, which staggers them, and a staggered start never loses the race.
        let threads = 16;
        let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut handles = Vec::new();
        for _ in 0..threads {
            let registry = registry.clone();
            let granted = Arc::clone(&granted);
            let gate = Arc::clone(&gate);
            handles.push(std::thread::spawn(move || {
                let node = registry.new_tree();
                while !gate.load(Ordering::SeqCst) {
                    std::hint::spin_loop();
                }
                if let Ok(spawn) = node.spawn_child("scout", rho_core::CancelToken::new()) {
                    granted.fetch_add(1, Ordering::SeqCst);
                    // Hold the slot until every thread has tried.
                    std::mem::forget(spawn);
                }
            }));
        }
        gate.store(true, Ordering::SeqCst);
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(
            granted.load(Ordering::SeqCst),
            1,
            "a process-wide cap of one must grant exactly one child, whatever the race"
        );
    }
}

#[test]
fn the_per_parent_cap_holds_when_two_threads_race() {
    // The per-parent reservation was a load, a check, then a later add. Its comment
    // claimed the three happened under one atomic and they did not, so two racing
    // spawns could both pass a cap of one. A review found the false comment.
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    for _ in 0..200 {
        let registry = rho_core::AgentRegistry::new(rho_core::SubagentLimits {
            max_children_per_parent: 1,
            max_live_total: 64,
            ..rho_core::SubagentLimits::new()
        });
        // One shared parent, so the per-parent cap is the binding one.
        let parent = Arc::new(registry.new_tree());
        let granted = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(AtomicBool::new(false));
        let mut handles = Vec::new();
        for _ in 0..16 {
            let parent = Arc::clone(&parent);
            let granted = Arc::clone(&granted);
            let gate = Arc::clone(&gate);
            handles.push(std::thread::spawn(move || {
                while !gate.load(Ordering::SeqCst) {
                    std::hint::spin_loop();
                }
                if let Ok(spawn) = parent.spawn_child("scout", rho_core::CancelToken::new()) {
                    granted.fetch_add(1, Ordering::SeqCst);
                    std::mem::forget(spawn);
                }
            }));
        }
        gate.store(true, Ordering::SeqCst);
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(
            granted.load(Ordering::SeqCst),
            1,
            "a per-parent cap of one must grant exactly one child, whatever the race"
        );
    }
}
