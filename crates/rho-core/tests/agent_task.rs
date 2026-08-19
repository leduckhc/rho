//! The agent task gate: rho verifies the work, and a child never grades itself.
//!
//! Tests for `SPEC-agent-tasks`. Every test isolates the filesystem with
//! `tempfile`. No test uses the network. No test uses `sleep`.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use rho_core::{
    Acceptance, AgentTask, ArtifactChecker, ArtifactSpec, CancelToken, CheckOutcome, CommandRunner,
    DefaultGate, Gate, GateContext, GateReport,
};

/// A runner that reports a fixed exit code, and counts its calls.
struct FakeRunner {
    code: i32,
    fail_to_start: bool,
    calls: AtomicUsize,
}

impl FakeRunner {
    fn exiting(code: i32) -> Arc<Self> {
        Arc::new(Self {
            code,
            fail_to_start: false,
            calls: AtomicUsize::new(0),
        })
    }
    fn unstartable() -> Arc<Self> {
        Arc::new(Self {
            code: 0,
            fail_to_start: true,
            calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl CommandRunner for FakeRunner {
    async fn run(
        &self,
        _command: &str,
        _root: &Path,
        _cancel: &CancelToken,
    ) -> std::io::Result<i32> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_to_start {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no such command",
            ));
        }
        Ok(self.code)
    }
}

fn context(root: &Path, runner: Arc<dyn CommandRunner>) -> GateContext {
    GateContext {
        session_root: root.to_path_buf(),
        cancel: CancelToken::new(),
        runner,
    }
}

// --- Artifacts ---

#[tokio::test]
async fn a_missing_file_artifact_fails_the_task() {
    let dir = tempfile::tempdir().unwrap();
    let task =
        AgentTask::new("scout", "write the report").with_artifacts(vec![ArtifactSpec::File {
            path: "report.md".into(),
        }]);
    let report = DefaultGate::new()
        .verify(&task, &context(dir.path(), FakeRunner::exiting(0)))
        .await
        .unwrap();

    assert!(!report.passed(), "a missing deliverable must fail");
    assert!(
        report.artifacts[0].detail.contains("not delivered"),
        "the detail must say the file was not delivered: {}",
        report.artifacts[0].detail
    );
}

#[tokio::test]
async fn an_empty_file_artifact_fails_the_task() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("report.md"), "").unwrap();
    let task =
        AgentTask::new("scout", "write the report").with_artifacts(vec![ArtifactSpec::File {
            path: "report.md".into(),
        }]);
    let report = DefaultGate::new()
        .verify(&task, &context(dir.path(), FakeRunner::exiting(0)))
        .await
        .unwrap();

    assert!(!report.passed(), "an empty deliverable must fail");
    assert!(report.artifacts[0].detail.contains("empty"));
}

#[tokio::test]
async fn a_present_file_artifact_passes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("report.md"), "the findings").unwrap();
    let task =
        AgentTask::new("scout", "write the report").with_artifacts(vec![ArtifactSpec::File {
            path: "report.md".into(),
        }]);
    let report = DefaultGate::new()
        .verify(&task, &context(dir.path(), FakeRunner::exiting(0)))
        .await
        .unwrap();

    assert!(report.passed(), "a delivered file must pass");
    assert!(report.failed_labels().is_empty());
}

#[tokio::test]
async fn a_file_artifact_outside_the_session_root_fails() {
    // An artifact path is attacker-influenced if a definition or a model writes
    // it. It must obey the same boundary as every tool. `confine` is that
    // boundary. Without this, a task could assert a file outside the root and
    // pass on something the child never produced.
    let dir = tempfile::tempdir().unwrap();
    let task =
        AgentTask::new("scout", "write the report").with_artifacts(vec![ArtifactSpec::File {
            path: "../escaped.md".into(),
        }]);
    let report = DefaultGate::new()
        .verify(&task, &context(dir.path(), FakeRunner::exiting(0)))
        .await
        .unwrap();

    assert!(!report.passed(), "a path outside the root must fail");
    assert!(
        report.artifacts[0]
            .detail
            .contains("outside the session root"),
        "the detail must name the boundary: {}",
        report.artifacts[0].detail
    );
}

// --- Commands and acceptance ---

#[tokio::test]
async fn a_command_that_exits_zero_passes() {
    let dir = tempfile::tempdir().unwrap();
    let task = AgentTask::new("dev", "fix the test").with_acceptance(vec![Acceptance::new(
        "the suite passes",
        ArtifactSpec::Command {
            run: "cargo test".into(),
        },
    )]);
    let report = DefaultGate::new()
        .verify(&task, &context(dir.path(), FakeRunner::exiting(0)))
        .await
        .unwrap();

    assert!(report.passed());
    assert_eq!(report.acceptance[0].label, "the suite passes");
}

#[tokio::test]
async fn a_failing_acceptance_check_fails_the_task() {
    let dir = tempfile::tempdir().unwrap();
    let task = AgentTask::new("dev", "fix the test").with_acceptance(vec![Acceptance::new(
        "the suite passes",
        ArtifactSpec::Command {
            run: "cargo test".into(),
        },
    )]);
    let report = DefaultGate::new()
        .verify(&task, &context(dir.path(), FakeRunner::exiting(101)))
        .await
        .unwrap();

    assert!(!report.passed(), "a non-zero exit must fail the task");
    assert_eq!(report.failed_labels(), vec!["the suite passes".to_string()]);
    assert!(report.acceptance[0].detail.contains("101"));
}

#[tokio::test]
async fn a_command_that_cannot_start_fails_closed() {
    // A check that cannot run has proved nothing, so it must never pass.
    let dir = tempfile::tempdir().unwrap();
    let task = AgentTask::new("dev", "fix the test").with_acceptance(vec![Acceptance::new(
        "the suite passes",
        ArtifactSpec::Command {
            run: "definitely-not-a-command".into(),
        },
    )]);
    let report = DefaultGate::new()
        .verify(&task, &context(dir.path(), FakeRunner::unstartable()))
        .await
        .unwrap();

    assert!(!report.passed(), "an unrunnable check must fail closed");
    assert!(report.acceptance[0].detail.contains("could not run"));
}

#[tokio::test]
async fn a_task_with_no_artifacts_still_runs() {
    let dir = tempfile::tempdir().unwrap();
    let task = AgentTask::new("scout", "have a look around");
    let report = DefaultGate::new()
        .verify(&task, &context(dir.path(), FakeRunner::exiting(0)))
        .await
        .unwrap();

    assert!(report.passed(), "an empty report passes");
    assert!(report.artifacts.is_empty());
    assert!(report.acceptance.is_empty());
}

#[tokio::test]
async fn a_task_with_no_goal_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let task = AgentTask::new("scout", "   ");
    let error = DefaultGate::new()
        .verify(&task, &context(dir.path(), FakeRunner::exiting(0)))
        .await
        .expect_err("a task with no goal must be refused");
    assert!(
        error.to_string().contains("needs a goal"),
        "the refusal must teach: {error}"
    );
}

// --- The extension point ---

struct CoverageChecker;

#[async_trait]
impl ArtifactChecker for CoverageChecker {
    fn kind(&self) -> &str {
        "coverage"
    }
    async fn check(&self, value: &str, _ctx: &GateContext) -> CheckOutcome {
        if value == "80" {
            CheckOutcome::Pass
        } else {
            CheckOutcome::Fail {
                detail: format!("coverage {value} is under the bar"),
            }
        }
    }
}

#[tokio::test]
async fn a_registered_checker_handles_its_named_kind() {
    // A third party adds a check kind with no edit to rho. This is the extension
    // point the spec promises.
    let dir = tempfile::tempdir().unwrap();
    let task =
        AgentTask::new("dev", "raise the coverage").with_artifacts(vec![ArtifactSpec::Named {
            name: "coverage".into(),
            value: "80".into(),
        }]);
    let report = DefaultGate::new()
        .with_checker(Arc::new(CoverageChecker))
        .verify(&task, &context(dir.path(), FakeRunner::exiting(0)))
        .await
        .unwrap();

    assert!(report.passed(), "a registered checker must be consulted");
}

#[tokio::test]
async fn an_unknown_artifact_kind_fails_closed_rather_than_skipping() {
    // The fail-open shape this project has shipped twice: an unhandled case that
    // counts as approval. See D-plugin-does-not-classify-itself.
    let dir = tempfile::tempdir().unwrap();
    let task =
        AgentTask::new("dev", "raise the coverage").with_artifacts(vec![ArtifactSpec::Named {
            name: "nobody-registered-this".into(),
            value: "80".into(),
        }]);
    let report = DefaultGate::new()
        .verify(&task, &context(dir.path(), FakeRunner::exiting(0)))
        .await
        .unwrap();

    assert!(
        !report.passed(),
        "an unverifiable check must never count as a pass"
    );
    assert!(
        report.artifacts[0]
            .detail
            .contains("no checker is registered"),
        "the detail must say why: {}",
        report.artifacts[0].detail
    );
}

// --- The no-self-grade rule ---

#[tokio::test]
async fn the_gate_result_and_the_child_claim_are_separate_fields() {
    // The separation is the whole point. A reader must never mistake a claim for
    // a verified result. See D-a-child-does-not-grade-itself.
    let report = rho_core::AgentReport {
        agent: "scout".into(),
        outcome: rho_core::AgentOutcome::Done,
        summary: "I did everything perfectly".into(),
        usage: rho_core::Usage::default(),
        turns: 1,
        gate: GateReport::default(),
        claims: rho_core::ChildClaims {
            open_questions: vec!["did I miss a case?".into()],
            what_i_did_not_check: vec!["the windows path".into()],
        },
        transcript: None,
    };

    // The claims say something. The gate says nothing, and the gate is the truth.
    assert!(!report.claims.open_questions.is_empty());
    assert!(
        report.gate.passed(),
        "an empty gate report passes, because nothing was declared"
    );
    assert!(
        report.gate.artifacts.is_empty(),
        "a child claim must never appear as a gate result"
    );
}

#[tokio::test]
async fn a_child_cannot_mark_its_own_acceptance_as_passed() {
    // This is a compile-time property, asserted here so the intent is recorded.
    // `CheckResult::from_outcome` is crate-internal, and no public constructor
    // takes a `passed` flag from outside `rho-core`. A child produces text, and
    // text cannot become a `CheckResult`.
    //
    // The observable consequence: a child that claims success while a declared
    // artifact is missing still fails the gate.
    let dir = tempfile::tempdir().unwrap();
    let task =
        AgentTask::new("scout", "write the report").with_artifacts(vec![ArtifactSpec::File {
            path: "report.md".into(),
        }]);
    let gate_report = DefaultGate::new()
        .verify(&task, &context(dir.path(), FakeRunner::exiting(0)))
        .await
        .unwrap();

    let failed = gate_report.failed_labels();
    assert!(
        !gate_report.passed(),
        "the child's opinion is not an input to the verdict"
    );

    // A caller turns a failed gate into `Rejected`, never `Done`.
    let outcome = if gate_report.passed() {
        rho_core::AgentOutcome::Done
    } else {
        rho_core::AgentOutcome::Rejected { failed }
    };
    match outcome {
        rho_core::AgentOutcome::Rejected { failed } => {
            assert_eq!(failed.len(), 1, "the failed label must be carried");
        }
        other => panic!("a failed gate must not report {other:?}"),
    }
}

#[tokio::test]
async fn an_old_record_without_a_gate_field_reads_as_an_empty_report() {
    // The persisted format binds the next version of rho. An old transcript has
    // no `gate` and no `claims`. It must load, and it must not claim a pass it
    // never earned. An empty report passes because nothing was declared, which is
    // the honest reading.
    let json = serde_json::json!({
        "agent": "scout",
        "outcome": "done",
        "summary": "the old answer",
        "usage": rho_core::Usage::default(),
        "turns": 2,
        "transcript": null
    });
    let report: rho_core::AgentReport = serde_json::from_value(json).expect("an old record loads");
    assert_eq!(report.summary, "the old answer");
    assert!(report.gate.artifacts.is_empty());
    assert!(report.claims.open_questions.is_empty());
}
