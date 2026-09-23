//! Turn-boundary tests for a mid-session provider switch.
//!
//! See `SPEC-switch-the-provider-mid-session` section 13 and
//! `D-the-provider-is-mutable-behind-a-mutex`. The invariant: a turn reads the provider,
//! the model, and the effort under one lock, once, so a switch never pairs a new provider
//! with an old model.
//!
//! Both tests force the interleaving with a `tokio::sync::Notify`. No test sleeps against
//! a wall clock, and no test races one. See AGENTS.md step 5.

mod common;

use std::sync::Arc;

use futures::StreamExt;
use rho_core::{
    AgentEvent, AgentEvents, CancelToken, CompletionRequest, ContentBlock, Context, HookChain,
    ModelCatalog, ModelSelection, Provider, ProviderBuildError, ProviderError, ProviderFactory,
    ProviderStream, Session, SessionConfig, StreamEvent, ToolRegistry,
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

/// The pairs a provider saw, as `(provider id, request model)`. A mixed pair would show a
/// new provider beside an old model, which is the defect these tests guard.
type Pairs = Arc<std::sync::Mutex<Vec<(String, String)>>>;

/// A provider that records its own id and the request model as one pair, then optionally
/// blocks on a `Notify` so the test controls the exact interleaving.
struct PairProvider {
    id: String,
    seen: Pairs,
    block: Option<Arc<Notify>>,
}

#[async_trait::async_trait]
impl Provider for PairProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn catalog(&self) -> Option<&dyn ModelCatalog> {
        None
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        _cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError> {
        self.seen
            .lock()
            .expect("the lock holds")
            .push((self.id.clone(), request.model));
        if let Some(block) = &self.block {
            block.notified().await;
        }
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

/// A factory that builds one preset provider for a switch. It shares the pair log, so a
/// turn on the built provider records into the same list.
struct SwitchFactory {
    id: String,
    seen: Pairs,
}

impl ProviderFactory for SwitchFactory {
    fn build(&self, _name: &str) -> Result<Arc<dyn Provider>, ProviderBuildError> {
        Ok(Arc::new(PairProvider {
            id: self.id.clone(),
            seen: Arc::clone(&self.seen),
            block: None,
        }) as Arc<dyn Provider>)
    }
}

/// Spin until the blocking provider has captured its request. A bounded loop, so a broken
/// implementation cannot hang the test forever.
async fn wait_for_first_pair(seen: &Pairs) {
    let mut waited_ms = 0;
    while seen.lock().expect("the lock holds").is_empty() {
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        waited_ms += 1;
        assert!(
            waited_ms < 2000,
            "the blocking provider never captured the request"
        );
    }
}

/// A running turn keeps its provider across a switch, and the next turn uses the new one.
#[tokio::test]
async fn a_running_turn_keeps_its_provider_across_a_switch() {
    let seen: Pairs = Arc::new(std::sync::Mutex::new(Vec::new()));
    let released = Arc::new(Notify::new());
    let first = Arc::new(PairProvider {
        id: "one".to_string(),
        seen: Arc::clone(&seen),
        block: Some(Arc::clone(&released)),
    });
    let factory = Arc::new(SwitchFactory {
        id: "two".to_string(),
        seen: Arc::clone(&seen),
    });
    let session = session_over(first, common::test_config()).with_provider_factory(factory);

    // Turn one starts. The first provider records its pair, then blocks.
    let events = session.prompt(user_input("one"), CancelToken::new());
    wait_for_first_pair(&seen).await;

    // A switch runs while turn one is blocked. It builds the new provider and commits.
    session
        .apply_selection(ModelSelection::switch("two-name", "picked", None))
        .expect("the switch builds");

    // Release turn one so the run finishes.
    released.notify_one();
    let _ = collect(events).await;

    // The next turn reads the running unit again, so it uses the new provider and model.
    let events = session.prompt(user_input("two"), CancelToken::new());
    let _ = collect(events).await;

    let pairs = seen.lock().expect("the lock holds").clone();
    assert_eq!(
        pairs[0],
        ("one".to_string(), "test-model".to_string()),
        "turn one streamed from the first provider"
    );
    assert_eq!(
        pairs[1],
        ("two".to_string(), "picked".to_string()),
        "the next turn used the new provider and the new model"
    );
}

/// A turn never pairs a new provider with an old model.
#[tokio::test]
async fn a_turn_never_pairs_a_new_provider_with_an_old_model() {
    let seen: Pairs = Arc::new(std::sync::Mutex::new(Vec::new()));
    let released = Arc::new(Notify::new());
    let first = Arc::new(PairProvider {
        id: "one".to_string(),
        seen: Arc::clone(&seen),
        block: Some(Arc::clone(&released)),
    });
    let factory = Arc::new(SwitchFactory {
        id: "two".to_string(),
        seen: Arc::clone(&seen),
    });
    let session = session_over(first, common::test_config()).with_provider_factory(factory);

    let events = session.prompt(user_input("one"), CancelToken::new());
    wait_for_first_pair(&seen).await;

    // While the turn blocks, apply a switch that changes both the provider and the model.
    session
        .apply_selection(ModelSelection::switch("two-name", "picked", None))
        .expect("the switch builds");

    released.notify_one();
    let _ = collect(events).await;

    let pairs = seen.lock().expect("the lock holds").clone();
    assert_eq!(
        pairs[0],
        ("one".to_string(), "test-model".to_string()),
        "the recorded pair is the whole old pair, never a mixed pair"
    );
}
