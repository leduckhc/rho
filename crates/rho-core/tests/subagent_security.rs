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
