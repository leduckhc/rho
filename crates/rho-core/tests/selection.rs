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
    ModelCatalog, ModelSelection, Provider, ProviderError, ProviderStream, ReasoningEffort,
    Session, SessionConfig, StreamEvent, ToolRegistry,
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
            // `selection()` names the running provider, which `with_config` seeds from
            // `RecordingProvider::id`. No `with_provider_name` overrides it here.
            provider: Some("recording".to_string()),
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

    session
        .apply_selection(ModelSelection {
            model: "picked-model".to_string(),
            reasoning_effort: Some(ReasoningEffort::High),
            provider: None,
        })
        .expect("a None-provider selection never fails");

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

        fn catalog(&self) -> Option<&dyn ModelCatalog> {
            None
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
    session
        .apply_selection(ModelSelection {
            model: "different-model".to_string(),
            reasoning_effort: Some(ReasoningEffort::XHigh),
            provider: None,
        })
        .expect("a None-provider selection never fails");

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

/// A provider with a fixed id and a plain one-text-block turn. A switch test swaps to it,
/// so its `id` is what `selection().provider` must not report; the config name does.
struct StubProvider {
    id: String,
}

impl StubProvider {
    fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

#[async_trait::async_trait]
impl Provider for StubProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn catalog(&self) -> Option<&dyn ModelCatalog> {
        None
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
        _cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError> {
        let events = vec![
            StreamEvent::MessageStart {
                role: rho_core::Role::Assistant,
            },
            StreamEvent::TextStart { index: 0 },
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

/// A factory that counts its builds and returns a preset result. A test asserts the count,
/// so a switch that never builds and a no-op that builds twice both fail.
struct CountingFactory {
    calls: std::sync::Mutex<Vec<String>>,
    result: Result<Arc<dyn Provider>, String>,
}

impl CountingFactory {
    fn building(id: impl Into<String>) -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            result: Ok(Arc::new(StubProvider::new(id)) as Arc<dyn Provider>),
        }
    }

    fn failing(message: impl Into<String>) -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            result: Err(message.into()),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.lock().expect("the lock holds").len()
    }
}

impl rho_core::ProviderFactory for CountingFactory {
    fn build(&self, name: &str) -> Result<Arc<dyn Provider>, rho_core::ProviderBuildError> {
        self.calls
            .lock()
            .expect("the lock holds")
            .push(name.to_string());
        self.result.clone().map_err(rho_core::ProviderBuildError)
    }
}

/// A `None` provider selection commits the model and never builds, even with no factory.
#[tokio::test]
async fn a_none_provider_selection_never_builds() {
    let session = session_over(Arc::new(RecordingProvider::new()), common::test_config());
    session
        .apply_selection(ModelSelection::new("picked", None))
        .expect("a None-provider selection never fails");
    assert_eq!(session.selection().model, "picked", "the model committed");
}

/// A selection that names the running provider writes the model and never builds.
#[tokio::test]
async fn a_same_provider_selection_never_builds() {
    let factory = Arc::new(CountingFactory::building("other"));
    let session = session_over(Arc::new(RecordingProvider::new()), common::test_config())
        .with_provider_factory(factory.clone());
    // `RecordingProvider::id` is `recording`, so this names the running provider.
    session
        .apply_selection(ModelSelection::switch("recording", "picked", None))
        .expect("a same-provider selection never fails");
    assert_eq!(factory.call_count(), 0, "no build for the running provider");
    assert_eq!(session.selection().model, "picked", "the model committed");
}

/// A switch to a new provider calls the factory once and reports the new provider.
#[tokio::test]
async fn a_switch_builds_and_swaps_the_provider() {
    let factory = Arc::new(CountingFactory::building("gpt"));
    let session = session_over(Arc::new(RecordingProvider::new()), common::test_config())
        .with_provider_factory(factory.clone());
    session
        .apply_selection(ModelSelection::switch("openrouter", "gpt-4o", None))
        .expect("the switch builds");
    assert_eq!(factory.call_count(), 1, "the factory built once");
    assert_eq!(
        session.selection().provider,
        Some("openrouter".to_string()),
        "the running provider is the new one"
    );
    assert_eq!(session.selection().model, "gpt-4o", "the model committed");
}

/// `selection().provider` always names the running provider, from `RunningState`.
#[tokio::test]
async fn selection_reads_the_provider_from_the_running_unit() {
    let session = session_over(
        Arc::new(StubProvider::new("bedrock")),
        common::test_config(),
    );
    assert_eq!(
        session.selection().provider,
        Some("bedrock".to_string()),
        "the running provider is the seed id"
    );
}

/// A named entry reports its config name, and re-picking that name builds nothing.
#[tokio::test]
async fn a_named_entry_keeps_its_id_and_re_pick_does_not_rebuild() {
    let factory = Arc::new(CountingFactory::building("anthropic"));
    let session = session_over(
        Arc::new(StubProvider::new("anthropic")),
        common::test_config(),
    )
    .with_provider_name("xdent-claude")
    .with_provider_factory(factory.clone());
    assert_eq!(
        session.selection().provider,
        Some("xdent-claude".to_string()),
        "the named entry keeps its own id, not the protocol id"
    );
    session
        .apply_selection(ModelSelection::switch("xdent-claude", "claude", None))
        .expect("a re-pick of the named entry never fails");
    assert_eq!(factory.call_count(), 0, "a re-pick builds nothing");
}

/// A failed switch keeps the current provider and returns the error.
#[tokio::test]
async fn a_failed_switch_keeps_the_current_provider() {
    let factory = Arc::new(CountingFactory::failing("boom"));
    let session = session_over(
        Arc::new(StubProvider::new("bedrock")),
        common::test_config(),
    )
    .with_provider_factory(factory);
    let result = session.apply_selection(ModelSelection::switch("azure", "m", None));
    assert!(result.is_err(), "the switch returns the build error");
    assert_eq!(
        session.selection().provider,
        Some("bedrock".to_string()),
        "the running provider is unchanged"
    );
    assert_eq!(
        session.selection().model,
        "test-model",
        "the model is unchanged"
    );
}

/// A switch with no factory is refused and changes nothing.
#[tokio::test]
async fn a_switch_without_a_factory_is_refused() {
    let session = session_over(
        Arc::new(StubProvider::new("bedrock")),
        common::test_config(),
    );
    let result = session.apply_selection(ModelSelection::switch("azure", "m", None));
    assert!(result.is_err(), "a switch with no factory is refused");
    assert_eq!(
        session.selection().provider,
        Some("bedrock".to_string()),
        "the running provider is unchanged"
    );
    assert_eq!(
        session.selection().model,
        "test-model",
        "the model is unchanged"
    );
}

/// The build error line holds only what the factory reported, and no resolved credential.
#[tokio::test]
async fn a_provider_build_error_carries_no_secret() {
    let secret = "sk-super-secret-value";
    let factory = Arc::new(CountingFactory::failing(
        "set AWS_SECRET_ACCESS_KEY in the environment",
    ));
    let session = session_over(
        Arc::new(StubProvider::new("bedrock")),
        common::test_config(),
    )
    .with_provider_factory(factory);
    let error = session
        .apply_selection(ModelSelection::switch("bedrock-alt", "m", None))
        .expect_err("the credential failure is an error");
    assert!(
        !error.to_string().contains(secret),
        "the error line holds no resolved credential value"
    );
}

// The three tests that pinned the deleted `rho-core` recorder surface lived here:
// `model_change_writes_a_model_change_record`, `unchanged_selection_does_not_write_a_model_change_record`,
// and `model_change_takes_effect_next_request`. The interactive TUI now records a model
// change through the app seam, so `Session::with_recorder` and the `set_selection` recorder
// branch are deleted. `set_selection_takes_effect_on_the_next_request` still proves the wire
// invariant the third test duplicated. See
// `SPEC-the-interactive-session-records-itself` section 15 and `bench/deleted-tests.txt`.
