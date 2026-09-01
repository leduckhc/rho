//! Tests for the mutable model selection.
//!
//! See `SPEC-model-selection-in-tui` section 1 and
//! `D-model-selection-is-mutable-behind-a-mutex`. The mutex is the one source of truth
//! for the running model and effort; `Driver::build_request` reads it at the start of
//! every turn.
//!
//! These tests share the recording provider with `agent_loop.rs`, so they can see what
//! rho actually sent.

mod common;

use std::sync::Arc;

use common::RecordingProvider;
use futures::StreamExt;
use rho_core::{
    AgentEvent, AgentEvents, CancelToken, CompletionRequest, ContentBlock, Context, HookChain,
    ModelSelection, Provider, ProviderError, ProviderStream, ReasoningEffort, Session,
    SessionConfig, StreamEvent, ToolRegistry,
};
use tokio::sync::Notify;

fn user_input(text: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text {
        text: text.to_string(),
    }]
}

async fn collect(mut events: AgentEvents) -> Vec<AgentEvent> {
    let mut out = Vec::new();
    while let Some(item) = events.next().await {
        out.push(item.expect("no error event in these scripts"));
    }
    out
}

fn session_over(provider: Arc<dyn Provider>, config: SessionConfig) -> Session {
    Session::with_config(
        config,
        provider,
        Arc::new(ToolRegistry::new()),
        Arc::new(HookChain::new()),
        Context::new(Some("system".to_string()), Vec::new()),
    )
}

/// A selection reads back what `SessionConfig` seeded, so a caller with no `set_selection`
/// call sees no surprise.
#[tokio::test]
async fn a_selection_reads_back_the_seed_from_the_config() {
    let provider = Arc::new(RecordingProvider::new());
    let config = common::test_config().with_reasoning_effort(Some(ReasoningEffort::Medium));
    // `test_config` uses "test-model", which is what the seed reads back below.
    let session = session_over(provider, config);

    let seen = session.selection();
    assert_eq!(
        seen,
        ModelSelection {
            model: "test-model".to_string(),
            reasoning_effort: Some(ReasoningEffort::Medium),
        },
        "the seed is the initial value"
    );
}

/// A change reaches the wire on the next turn, so the whole feature has a running effect
/// and is not a state field nobody reads.
#[tokio::test]
async fn set_selection_takes_effect_on_the_next_request() {
    let provider = Arc::new(RecordingProvider::new());
    let session = session_over(provider.clone(), common::test_config());

    session.set_selection(ModelSelection {
        model: "picked-model".to_string(),
        reasoning_effort: Some(ReasoningEffort::High),
    });

    let events = session.prompt(user_input("hello"), CancelToken::new());
    let _ = collect(events).await;

    let seen = provider.seen.lock().expect("the lock holds");
    assert_eq!(seen.len(), 1, "one request went out");
    assert_eq!(
        seen[0].model, "picked-model",
        "the model on the wire is new"
    );
    assert_eq!(
        seen[0].reasoning,
        Some(ReasoningEffort::High),
        "the effort on the wire is new"
    );
}

/// A provider that blocks at the start of `stream` proves the running turn keeps the
/// value the mutex held at the moment the turn began. A `set_selection` mid-turn is a
/// no-op for that turn.
///
/// This test is the guard for the invariant D-model-selection-is-mutable-behind-a-mutex
/// names: the running turn is not mutated by a write to the mutex.
#[tokio::test]
async fn set_selection_never_changes_the_running_turn() {
    /// A provider that records the model of the request it saw, then blocks until the
    /// test releases the notify, and finally answers one plain turn.
    struct BlockingProvider {
        released: Arc<Notify>,
        seen_model: std::sync::Mutex<Option<String>>,
    }

    #[async_trait::async_trait]
    impl Provider for BlockingProvider {
        fn id(&self) -> &str {
            "blocking"
        }

        async fn stream(
            &self,
            request: CompletionRequest,
            _cancel: CancelToken,
        ) -> Result<ProviderStream, ProviderError> {
            *self.seen_model.lock().expect("the lock holds") = Some(request.model);
            // Block here until the test releases us. `notified()` awaits the next call to
            // `notify_one`.
            self.released.notified().await;
            let events = vec![
                StreamEvent::MessageStart {
                    role: rho_core::Role::Assistant,
                },
                StreamEvent::TextStart { index: 0 },
                StreamEvent::TextDelta {
                    index: 0,
                    delta: "ok".to_string(),
                },
                StreamEvent::TextEnd { index: 0 },
                StreamEvent::Done {
                    stop_reason: rho_core::StopReason::EndTurn,
                },
            ];
            Ok(Box::pin(futures::stream::iter(
                events.into_iter().map(Ok).collect::<Vec<_>>(),
            )))
        }
    }

    let released = Arc::new(Notify::new());
    let provider = Arc::new(BlockingProvider {
        released: Arc::clone(&released),
        seen_model: std::sync::Mutex::new(None),
    });
    let session = Arc::new(session_over(
        provider.clone() as Arc<dyn Provider>,
        common::test_config(),
    ));

    // Start the run. The provider records the model, then blocks.
    let events = session.prompt(user_input("hi"), CancelToken::new());

    // Spin until the provider has captured the request. Bounded loop, so a broken
    // implementation cannot hang the test forever.
    let mut waited_ms = 0;
    while provider
        .seen_model
        .lock()
        .expect("the lock holds")
        .is_none()
    {
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        waited_ms += 1;
        assert!(
            waited_ms < 2000,
            "the blocking provider never captured the request"
        );
    }

    // The running turn already saw the seed model. A write now must not change it.
    session.set_selection(ModelSelection {
        model: "different-model".to_string(),
        reasoning_effort: Some(ReasoningEffort::XHigh),
    });

    let running = provider
        .seen_model
        .lock()
        .expect("the lock holds")
        .clone()
        .expect("the provider captured the model");
    assert_eq!(
        running, "test-model",
        "the running turn keeps the old model, not the one just written"
    );

    // Release the provider so the run finishes and the test does not hang.
    released.notify_one();
    let _ = collect(events).await;
}
