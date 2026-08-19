use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;

use crate::sandbox::SandboxMode;
use crate::subagent::error::SubagentError;
use crate::tool::{ApprovalDecision, ApprovalPolicy, ToolKind};

// --- The security core: composition, not comparison (D-child-confined-by-composition) ---

/// Allow a call only when **both** policies allow it.
///
/// This is how a child is confined. The parent's policy is always one of the two
/// conjuncts, so a child can only ever be more restrictive. Escalation is not
/// checked, it is unrepresentable. See decision D-child-confined-by-composition.
pub struct BothPolicies {
    parent: Arc<dyn ApprovalPolicy>,
    child: Arc<dyn ApprovalPolicy>,
}

impl BothPolicies {
    /// Compose a parent policy and a child policy. The parent is checked first.
    pub fn new(parent: Arc<dyn ApprovalPolicy>, child: Arc<dyn ApprovalPolicy>) -> Self {
        Self { parent, child }
    }
}

#[async_trait]
impl ApprovalPolicy for BothPolicies {
    async fn approve(
        &self,
        tool: &str,
        kind: ToolKind,
        args: &serde_json::Value,
    ) -> ApprovalDecision {
        match self.parent.approve(tool, kind, args).await {
            ApprovalDecision::Deny => ApprovalDecision::Deny,
            ApprovalDecision::Allow => self.child.approve(tool, kind, args).await,
        }
    }
}

/// The result of intersecting a child's requested tools with the parent's set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolIntersection {
    /// The tools the child keeps: the parent's set filtered by the child's list.
    pub allowed: Vec<String>,
    /// The names the child asked for that the parent does not hold. Reported to
    /// the caller so a bad definition is visible rather than silent.
    pub dropped: Vec<String>,
}

/// Intersect a child's requested tool names with the parent's set.
///
/// A child that names no tools inherits the parent's set unchanged. A name the
/// parent does not hold is dropped and reported. This removes a class of
/// privilege escalation by delegation: a child can never receive a tool its
/// parent never had. See decision D-child-confined-by-composition.
pub fn intersect_tools(parent: &[String], child_request: Option<&[String]>) -> ToolIntersection {
    let Some(requested) = child_request else {
        return ToolIntersection {
            allowed: parent.to_vec(),
            dropped: Vec::new(),
        };
    };
    let parent_set: HashSet<&str> = parent.iter().map(String::as_str).collect();
    let mut allowed = Vec::new();
    let mut dropped = Vec::new();
    for name in requested {
        if parent_set.contains(name.as_str()) {
            allowed.push(name.clone());
        } else {
            dropped.push(name.clone());
        }
    }
    ToolIntersection { allowed, dropped }
}

/// Narrow a sandbox mode. A child may ask for a stricter mode, never a weaker
/// one.
///
/// A child that names no mode inherits the parent's mode. A child that asks for
/// a mode at least as strict as the parent's gets that mode. A child that asks
/// for a weaker mode is refused, and the refusal names both modes. See `SPEC-subagents`
/// section 4.
pub fn narrow_sandbox(
    parent: SandboxMode,
    child_request: Option<SandboxMode>,
) -> Result<SandboxMode, SubagentError> {
    let Some(requested) = child_request else {
        return Ok(parent);
    };
    if sandbox_rank(requested) >= sandbox_rank(parent) {
        Ok(requested)
    } else {
        Err(SubagentError::WeakerSandbox { parent, requested })
    }
}

/// Rank the confinement strength of a sandbox mode. A higher rank is stricter.
fn sandbox_rank(mode: SandboxMode) -> u8 {
    match mode {
        SandboxMode::Off => 0,
        SandboxMode::Confined => 1,
        SandboxMode::Strict => 2,
    }
}
