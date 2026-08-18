//! Security tests for background tasks. See `SPEC-background-tasks` section 9.
//!
//! The approval policy is the real boundary. A read-only policy denies a
//! background `bash` and denies `task_cancel`, but allows a `task` probe. And the
//! approval runs before the task starts, so a denied command starts no process.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream;
use rho_core::{
    ApprovalDecision, ApprovalPolicy, CancelToken, CompletionRequest, ContentBlock, Context,
    HookChain, Provider, ProviderError, ProviderStream, ReadOnlyPolicy, Role, Session,
    SessionConfig, StopReason, StreamEvent, TaskLimits, TaskRegistry, Tool, ToolKind,
};
use rho_tools::{BashTool, TaskCancelTool, TaskTool, builtin_registry_with_tasks};

#[tokio::test]
async fn a_read_only_policy_denies_a_background_bash() {
    // A background `bash` is still Execute, so a read-only policy denies it.
    let registry = Arc::new(TaskRegistry::new(TaskLimits::default()));
    let bash = BashTool::with_tasks(registry);
    let decision = ReadOnlyPolicy
        .approve(
            "bash",
            bash.kind(),
            &serde_json::json!({ "run_in_background": true }),
        )
        .await;
    assert_eq!(decision, ApprovalDecision::Deny);
    assert_eq!(bash.kind(), ToolKind::Execute);
}

#[tokio::test]
async fn a_read_only_policy_allows_a_task_probe() {
    let registry = Arc::new(TaskRegistry::new(TaskLimits::default()));
    let task = TaskTool::new(registry);
    let decision = ReadOnlyPolicy
        .approve(
            "task",
            task.kind(),
            &serde_json::json!({ "action": "list" }),
        )
        .await;
    assert_eq!(decision, ApprovalDecision::Allow);
    assert_eq!(task.kind(), ToolKind::Read);
}

#[tokio::test]
async fn a_read_only_policy_denies_task_cancel() {
    let registry = Arc::new(TaskRegistry::new(TaskLimits::default()));
    let cancel = TaskCancelTool::new(registry);
    let decision = ReadOnlyPolicy
        .approve(
            "task_cancel",
            cancel.kind(),
            &serde_json::json!({ "id": "task-1" }),
        )
        .await;
    assert_eq!(decision, ApprovalDecision::Deny);
    assert_eq!(cancel.kind(), ToolKind::Execute);
}

/// A provider that asks for a background `bash` on turn one, then ends.
struct BackgroundBashProvider {
    calls: AtomicUsize,
}

#[async_trait]
impl Provider for BackgroundBashProvider {
    fn id(&self) -> &str {
        "background-bash-fake"
    }
    async fn stream(
        &self,
        _request: CompletionRequest,
        _cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let events = if call == 0 {
            vec![
                StreamEvent::MessageStart {
                    role: Role::Assistant,
                },
                StreamEvent::ToolCallStart {
                    index: 0,
                    id: "call_1".to_string(),
                    name: "bash".to_string(),
                },
                StreamEvent::ToolCallEnd {
                    index: 0,
                    arguments: serde_json::json!({ "command": "echo hi", "run_in_background": true }),
                },
                StreamEvent::Done {
                    stop_reason: StopReason::ToolUse,
                },
            ]
        } else {
            vec![
                StreamEvent::MessageStart {
                    role: Role::Assistant,
                },
                StreamEvent::TextStart { index: 0 },
                StreamEvent::TextDelta {
                    index: 0,
                    delta: "stopped".to_string(),
                },
                StreamEvent::TextEnd { index: 0 },
                StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                },
            ]
        };
        Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
    }
}

#[tokio::test]
async fn approval_runs_before_the_task_starts() {
    // A read-only policy denies the background `bash` before it runs. So no task
    // is ever created. The approval runs before the task starts, never after.
    let dir = tempfile::tempdir().unwrap();
    let registry = Arc::new(TaskRegistry::new(TaskLimits::default()));
    let tools = Arc::new(builtin_registry_with_tasks(Arc::clone(&registry)));
    let provider = Arc::new(BackgroundBashProvider {
        calls: AtomicUsize::new(0),
    });

    let config = SessionConfig::new("test-model", dir.path(), Arc::new(ReadOnlyPolicy));
    let session = Session::with_config(
        config,
        provider,
        tools,
        Arc::new(HookChain::new()),
        Context::new(None, Vec::new()),
    );

    let mut events = session.prompt(
        vec![ContentBlock::Text {
            text: "run something".to_string(),
        }],
        CancelToken::new(),
    );
    while let Some(event) = events.next().await {
        let _ = event.unwrap();
    }

    assert!(
        registry.list().await.is_empty(),
        "a denied background bash must start no task"
    );
}
