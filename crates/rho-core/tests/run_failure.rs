//! A failed run still ends its event stream.
//!
//! This is the invariant three frontends already depend on, and nothing pinned it.
//!
//! `Driver::run` returns early on `TurnOutcome::Failed`, and that early return skipped
//! `queue.unobserve()`. `Session::prompt` spawns a task that forwards steering-queue
//! announcements into the run's event channel, and that task holds a clone of the event
//! sender. It ends only when the queue drops its observer. So on a provider failure the
//! channel stayed open for ever: one leaked task per failed run, and a consumer that reads
//! to the end of the stream waits for ever.
//!
//! `rho-cli`, `rho-jsonl`, `rho-tui`, and `collect_report` each work around it by leaving the
//! loop on the error item. Four workarounds for one defect, and a fifth frontend would meet
//! the hang. `D-an-error-on-the-event-stream-ends-the-run` recorded the defect as known and
//! deferred, because another worktree owned the steering queue at the time. That work merged,
//! so the fix belongs here now.
//!
//! Each test drains the stream **to the end** under a timeout, so a regression fails rather
//! than hanging the suite. See `D-a-failed-run-releases-the-queue-observer`.

mod common;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use common::{ScriptedProvider, text_turn};
use futures::StreamExt;
use rho_core::{
    AgentEvent, CancelToken, CompletionRequest, ContentBlock, Context, HookChain, Provider,
    ProviderError, ProviderStream, Role, Session, StreamEvent, ToolRegistry,
};

/// A provider whose `stream` call fails, like a 400 or a transport reset.
///
/// This drives the first of the three sites that send an error and return `Failed`.
struct FailingProvider;

#[async_trait]
impl Provider for FailingProvider {
    fn id(&self) -> &str {
        "failing"
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
        _cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError> {
        Err(ProviderError::Client {
            status: 400,
            advice: "bad request",
        })
    }
}

fn session_with(provider: Arc<dyn Provider>) -> Session {
    Session::with_config(
        common::test_config(),
        provider,
        Arc::new(ToolRegistry::new()),
        Arc::new(HookChain::new()),
        Context::new(Some("system".to_string()), Vec::new()),
    )
}

fn user_input(text: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text {
        text: text.to_string(),
    }]
}

/// Drain a run to the end, and say what arrived.
///
/// The timeout is the assertion. An unbounded wait here would hang the suite instead of
/// failing it, which `session_integrity.rs` already learned the hard way.
async fn drain(session: &Session, prompt: &str) -> (bool, bool, bool) {
    let mut events = session.prompt(user_input(prompt), CancelToken::new());
    let mut saw_error = false;
    let mut saw_agent_end = false;
    let drained = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(item) = events.next().await {
            match item {
                Err(_) => saw_error = true,
                Ok(AgentEvent::AgentEnd { .. }) => saw_agent_end = true,
                Ok(_) => {}
            }
        }
    })
    .await;
    (drained.is_ok(), saw_error, saw_agent_end)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_provider_call_ends_the_event_stream() {
    let session = session_with(Arc::new(FailingProvider));
    let (ended, saw_error, _) = drain(&session, "hi").await;
    assert!(
        saw_error,
        "a provider failure reaches the caller as an error item"
    );
    assert!(
        ended,
        "and the stream ends after it. A stream that never ends leaks the forwarder task \
         and hangs any consumer that reads to the end"
    );
}

/// The same run, twice. The leak was one task and one stale observer **per** failed run, so
/// the second failure is where a per-run leak shows.
#[tokio::test(flavor = "multi_thread")]
async fn two_failed_runs_each_end_their_stream() {
    let session = session_with(Arc::new(FailingProvider));
    for attempt in 1..=2 {
        let (ended, saw_error, _) = drain(&session, "hi").await;
        assert!(saw_error, "attempt {attempt} reports the failure");
        assert!(ended, "attempt {attempt} ends its stream");
    }
}

/// A stream that stops with no `Done` event is the third site that sends an error and
/// returns `Failed`. It must end its stream too.
///
/// The decision that recorded this defect said "both sites that send an error". There are
/// three. A guard that names one site cannot see a fourth, so this test drives a second site
/// by a different route: a scripted turn with its `Done` event removed.
#[tokio::test(flavor = "multi_thread")]
async fn a_stream_that_ends_without_done_ends_the_event_stream() {
    let mut turn = text_turn("half an answer");
    let done = turn.pop();
    assert!(
        matches!(done, Some(StreamEvent::Done { .. })),
        "the fixture must end with Done, or this test drives the wrong path"
    );
    let session = session_with(Arc::new(ScriptedProvider::new(vec![turn])));
    let (ended, saw_error, _) = drain(&session, "hi").await;
    assert!(
        saw_error,
        "a stream with no Done event is a decode fault the caller hears about"
    );
    assert!(ended, "and the stream ends after it");
}

/// The good path must keep working. A run that ends well emits `AgentEnd` and then ends its
/// stream, and it did that before this fix, so this is the guard against fixing the leak by
/// closing the channel too early.
#[tokio::test(flavor = "multi_thread")]
async fn a_successful_run_still_ends_with_agent_end() {
    let session = session_with(Arc::new(ScriptedProvider::new(vec![text_turn("hello")])));
    let (ended, saw_error, saw_agent_end) = drain(&session, "hi").await;
    assert!(!saw_error, "a scripted answer carries no error");
    assert!(saw_agent_end, "a run that ends well says so");
    assert!(ended, "and its stream ends");
}

/// A message pushed after a failed run is not lost, and it is not announced on the dead
/// run's channel either. The next run delivers it.
///
/// The normal path calls `unobserve` for exactly this reason, and the failed path skipped it.
#[tokio::test(flavor = "multi_thread")]
async fn a_message_pushed_after_a_failed_run_reaches_the_next_run() {
    let session = session_with(Arc::new(FailingProvider));
    let (ended, _, _) = drain(&session, "hi").await;
    assert!(ended, "the failed run ends its stream");

    session
        .queue()
        .push(vec![ContentBlock::Text {
            text: "steered".to_string(),
        }])
        .expect("the queue accepts a small message");
    assert_eq!(
        session.queue().len(),
        1,
        "the message waits for the next run rather than vanishing into the dead channel"
    );

    let messages = session.messages().await;
    assert!(
        messages.iter().all(|message| message.role != Role::User
            || !message.content.iter().any(|block| matches!(
                block,
                ContentBlock::Text { text } if text == "steered"
            ))),
        "and it is not in the context until a run delivers it"
    );
}
