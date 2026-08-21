//! The agent task contract: a goal, its deliverables, and a verified verdict.
//!
//! A prompt is not a task. A child that says "done" proves nothing, so rho checks
//! the work itself. See `docs/specs/20260819-102750-SPEC-agent-tasks.md`.
//!
//! The rule that shapes this module: **a child never grades its own work.** No
//! type here lets a child build a [`CheckResult`]. Only a [`Gate`] does. The bug
//! is unrepresentable rather than tested, which is the same move as
//! `BothPolicies`. See decision D-a-child-does-not-grade-itself.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::cancel::CancelToken;
use crate::subagent::SubagentError;
use crate::tool::confine;

/// The work delegated to a child. It replaces the bare prompt.
///
/// The `goal` becomes the child's first message. The `artifacts` are the things
/// the child must deliver. The `acceptance` checks prove the work is correct. rho
/// verifies both after the child stops.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTask {
    /// The agent definition to run.
    pub agent: String,
    /// The goal, in the parent's words. It becomes the child's prompt.
    pub goal: String,
    /// The deliverables the child must produce. Empty is allowed.
    #[serde(default)]
    pub artifacts: Vec<ArtifactSpec>,
    /// The checks rho runs to prove the work. Empty is allowed.
    #[serde(default)]
    pub acceptance: Vec<Acceptance>,
}

impl AgentTask {
    /// A task with a goal and no checks. The defaults are stated here, not
    /// hidden. See decision D-no-four-argument-session-new.
    pub fn new(agent: impl Into<String>, goal: impl Into<String>) -> Self {
        Self {
            agent: agent.into(),
            goal: goal.into(),
            artifacts: Vec::new(),
            acceptance: Vec::new(),
        }
    }

    /// Add the deliverables the child must produce.
    pub fn with_artifacts(mut self, artifacts: Vec<ArtifactSpec>) -> Self {
        self.artifacts = artifacts;
        self
    }

    /// Add the checks rho runs to prove the work.
    pub fn with_acceptance(mut self, acceptance: Vec<Acceptance>) -> Self {
        self.acceptance = acceptance;
        self
    }

    /// Refuse a task rho cannot act on. A goal is required.
    pub fn validate(&self) -> Result<(), SubagentError> {
        if self.goal.trim().is_empty() {
            return Err(SubagentError::EmptyGoal);
        }
        Ok(())
    }
}

/// One thing the child must deliver, in a form rho can check without asking it.
///
/// `File` and `Command` are the built-in kinds. `Named` routes to a registered
/// [`ArtifactChecker`], so a third party adds a kind without editing this enum.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtifactSpec {
    /// A file that must exist and hold bytes, under the session root.
    File { path: PathBuf },
    /// A command that must exit zero. It runs under the parent's sandbox.
    ///
    /// The command must come from a trusted author, never from child output. See
    /// decision D-an-acceptance-check-has-a-trusted-author.
    Command { run: String },
    /// A named kind, resolved by a registered [`ArtifactChecker`].
    ///
    /// The field is `name` and not `kind`, because `kind` is the serde tag for
    /// this enum and serde refuses the collision. The spec said `kind`, and it was
    /// never compile-checked. AGENTS.md step 3 says to paste a signature into a
    /// scratch crate for exactly this reason.
    Named { name: String, value: String },
}

impl ArtifactSpec {
    /// A short label for a report line.
    pub fn label(&self) -> String {
        match self {
            Self::File { path } => format!("file {}", path.display()),
            Self::Command { run } => format!("command {run}"),
            Self::Named { name, value } => format!("{name} {value}"),
        }
    }
}

/// One named acceptance check. rho runs it, never the child.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acceptance {
    /// A short label, for the report and for a refusal.
    pub label: String,
    /// The condition the gate verifies.
    pub check: ArtifactSpec,
}

impl Acceptance {
    /// A named check.
    pub fn new(label: impl Into<String>, check: ArtifactSpec) -> Self {
        Self {
            label: label.into(),
            check,
        }
    }
}

/// The verdict of one check. Only a gate builds one, so a child cannot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckOutcome {
    Pass,
    Fail { detail: String },
}

/// One verified verdict.
///
/// There is no public constructor that a child can reach with a chosen `passed`
/// value, because a child never builds one of these. The gate does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub label: String,
    pub passed: bool,
    /// Why it passed or failed, for a human and for the model.
    pub detail: String,
}

impl CheckResult {
    /// Build a result from a gate's outcome. Crate-internal on purpose.
    pub(crate) fn from_outcome(label: impl Into<String>, outcome: CheckOutcome) -> Self {
        match outcome {
            CheckOutcome::Pass => Self {
                label: label.into(),
                passed: true,
                detail: "passed".to_string(),
            },
            CheckOutcome::Fail { detail } => Self {
                label: label.into(),
                passed: false,
                detail,
            },
        }
    }
}

/// The gate's verified verdict. One result per declared item, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateReport {
    #[serde(default)]
    pub artifacts: Vec<CheckResult>,
    #[serde(default)]
    pub acceptance: Vec<CheckResult>,
}

impl GateReport {
    /// True only when every check passed. An empty report passes.
    pub fn passed(&self) -> bool {
        self.artifacts.iter().all(|r| r.passed) && self.acceptance.iter().all(|r| r.passed)
    }

    /// The labels of every failed check, in order.
    pub fn failed_labels(&self) -> Vec<String> {
        self.artifacts
            .iter()
            .chain(self.acceptance.iter())
            .filter(|r| !r.passed)
            .map(|r| r.label.clone())
            .collect()
    }
}

/// The child's own words. **Unverified.**
///
/// These are jcode's honesty fields. They catch a lazy child cheaply, and they
/// are not proof. A gate never reads them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildClaims {
    /// Questions the child could not resolve. Unverified.
    #[serde(default)]
    pub open_questions: Vec<String>,
    /// Things the child says it did not check. Unverified.
    #[serde(default)]
    pub what_i_did_not_check: Vec<String>,
}

/// Runs a command check under the parent's sandbox. `rho-tools` supplies it, so
/// `rho-core` keeps no sandbox and no process dependency.
#[async_trait]
pub trait CommandRunner: Send + Sync {
    /// Run `command` under `root` and return its exit code.
    async fn run(&self, command: &str, root: &Path, cancel: &CancelToken) -> std::io::Result<i32>;
}

/// What a gate needs to run.
pub struct GateContext {
    /// The session root. A file artifact must resolve under it.
    pub session_root: PathBuf,
    /// Cancels a long command check with the parent.
    pub cancel: CancelToken,
    /// Runs a `Command` artifact under the parent's sandbox.
    pub runner: Arc<dyn CommandRunner>,
}

/// Resolves one `Named` artifact kind. This is the extension point.
///
/// A third party registers an impl to add a check kind, and edits nothing in rho.
#[async_trait]
pub trait ArtifactChecker: Send + Sync {
    /// The `Named` kind this checker handles.
    fn kind(&self) -> &str;
    /// Inspect one named artifact and return a verified verdict.
    async fn check(&self, value: &str, ctx: &GateContext) -> CheckOutcome;
}

/// Verifies a task after the child stops.
///
/// rho owns the gate. The child never runs it, so a child cannot certify its own
/// work. See decision D-a-child-does-not-grade-itself.
#[async_trait]
pub trait Gate: Send + Sync {
    /// Check every artifact and every acceptance check, in declaration order.
    async fn verify(
        &self,
        task: &AgentTask,
        ctx: &GateContext,
    ) -> Result<GateReport, SubagentError>;
}

/// rho's gate. It checks a file, runs a command, and dispatches a named kind.
///
/// An unknown named kind is **refused**, never skipped. A skipped check that
/// counts as a pass is the fail-open shape this project has shipped twice. See
/// decisions D-plugin-does-not-classify-itself and D-todo-in-a-green-stage.
#[derive(Default)]
pub struct DefaultGate {
    checkers: Vec<Arc<dyn ArtifactChecker>>,
}

impl DefaultGate {
    /// A gate with no named checkers.
    pub fn new() -> Self {
        Self {
            checkers: Vec::new(),
        }
    }

    /// Register a checker for one `Named` kind.
    pub fn with_checker(mut self, checker: Arc<dyn ArtifactChecker>) -> Self {
        self.checkers.push(checker);
        self
    }

    /// Check one artifact.
    async fn check_one(&self, spec: &ArtifactSpec, ctx: &GateContext) -> CheckOutcome {
        match spec {
            ArtifactSpec::File { path } => Self::check_file(path, ctx),
            ArtifactSpec::Command { run } => self.check_command(run, ctx).await,
            ArtifactSpec::Named { name, value } => {
                match self.checkers.iter().find(|c| c.kind() == name) {
                    Some(checker) => checker.check(value, ctx).await,
                    // Fail closed. An unmatched kind is a mistake in the task, and
                    // treating it as a pass would approve unverified work.
                    None => CheckOutcome::Fail {
                        detail: format!(
                            "no checker is registered for the artifact kind \"{name}\", so this \
                             check cannot be verified. Register an ArtifactChecker, or use a \
                             file or command check."
                        ),
                    },
                }
            }
        }
    }

    /// A file must resolve under the session root, exist, and hold bytes.
    fn check_file(path: &Path, ctx: &GateContext) -> CheckOutcome {
        // A declared artifact must not escape the root. `confine` is the same
        // boundary every tool uses, so an artifact cannot reach outside it.
        let resolved = match confine(&ctx.session_root, path) {
            Ok(resolved) => resolved,
            Err(error) => {
                return CheckOutcome::Fail {
                    detail: format!("the artifact path is outside the session root: {error}"),
                };
            }
        };
        match std::fs::metadata(&resolved) {
            Err(error) => CheckOutcome::Fail {
                detail: format!("{} was not delivered: {error}", path.display()),
            },
            Ok(meta) if !meta.is_file() => CheckOutcome::Fail {
                detail: format!("{} exists but is not a file", path.display()),
            },
            Ok(meta) if meta.len() == 0 => CheckOutcome::Fail {
                detail: format!("{} was delivered empty", path.display()),
            },
            Ok(_) => CheckOutcome::Pass,
        }
    }

    /// A command must start and exit zero. A command that cannot start fails.
    async fn check_command(&self, run: &str, ctx: &GateContext) -> CheckOutcome {
        match ctx.runner.run(run, &ctx.session_root, &ctx.cancel).await {
            Ok(0) => CheckOutcome::Pass,
            Ok(code) => CheckOutcome::Fail {
                detail: format!("`{run}` exited {code}"),
            },
            // Fail closed. A check that cannot run has proved nothing.
            Err(error) => CheckOutcome::Fail {
                detail: format!("`{run}` could not run: {error}"),
            },
        }
    }
}

#[async_trait]
impl Gate for DefaultGate {
    async fn verify(
        &self,
        task: &AgentTask,
        ctx: &GateContext,
    ) -> Result<GateReport, SubagentError> {
        task.validate()?;
        let mut report = GateReport::default();
        for spec in &task.artifacts {
            let outcome = self.check_one(spec, ctx).await;
            report
                .artifacts
                .push(CheckResult::from_outcome(spec.label(), outcome));
        }
        for check in &task.acceptance {
            let outcome = self.check_one(&check.check, ctx).await;
            report
                .acceptance
                .push(CheckResult::from_outcome(check.label.clone(), outcome));
        }
        Ok(report)
    }
}
