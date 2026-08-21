//! Steering: a message queued mid-run reaches the model at a turn boundary.
//!
//! Tests for `docs/specs/20260818-014343-SPEC-steering.md`. The delivery point is
//! the one thing that matters here: never inside a provider request, and never by
//! rewriting an already-sent turn.
//!
//! Another writer in this tree began the same integration tests at the same time,
//! against a guessed API. Their intent is kept here, against the real API.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{ScriptedProvider, text_turn, tool_call_turn};
use futures::StreamExt;
use rho_core::{
    AgentEvent, CancelToken, ContentBlock, Context, HookChain, MessageQueue, Provider, Role,
    Session, StreamEvent, ToolRegistry,
};

fn text(body: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text {
        text: body.to_string(),
    }]
}

/// A tool that queues a steering message while it runs.
///
/// Reacting to a `ToolStart` event from the stream is a race: the driver runs
/// ahead, so it can pass the drain point before the test observes the event. A
/// tool that pushes from inside its own `execute` is deterministic, and it uses no
/// `sleep`. This is the "synchronise with a channel" rule in SPEC-steering
/// section 9, applied with the tool itself as the synchroniser.
struct SteeringTool {
    queue: MessageQueue,
    messages: Vec<String>,
}

#[async_trait::async_trait]
impl rho_core::Tool for SteeringTool {
    fn name(&self) -> &str {
        "probe"
    }
    fn description(&self) -> &str {
        "a probe that steers its own session"
    }
    fn kind(&self) -> rho_core::ToolKind {
        rho_core::ToolKind::Read
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: rho_core::ToolContext,
    ) -> Result<rho_core::ToolOutput, rho_core::ToolError> {
        for body in &self.messages {
            self.queue.push(text(body)).expect("the queue has room");
        }
        Ok(rho_core::ToolOutput::text("probed"))
    }
}

fn session_with(turns: Vec<Vec<StreamEvent>>, tools: ToolRegistry) -> Session {
    let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::new(turns));
    Session::with_config(
        common::test_config(),
        provider,
        Arc::new(tools),
        Arc::new(HookChain::new()),
        Context::new(None, Vec::new()),
    )
}

#[tokio::test]
async fn a_message_queued_mid_turn_is_delivered_before_the_next_request() {
    // The load-bearing test. The first turn calls a tool. While that runs, a
    // message is queued. The driver must deliver it before it builds the second
    // request, so the model sees it as the next user message.
    let queue = MessageQueue::new();
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(SteeringTool {
        queue: queue.clone(),
        messages: vec!["steer me".to_string()],
    }));
    let session = session_with(
        vec![
            tool_call_turn("c1", "probe", serde_json::json!({})),
            text_turn("second turn"),
        ],
        tools,
    )
    .with_queue(queue.clone());

    let cancel = CancelToken::new();
    let mut events = session.prompt(text("start"), cancel);

    let mut delivered = None;
    while let Some(Ok(event)) = events.next().await {
        if let AgentEvent::MessageDelivered { count } = event {
            delivered = Some(count);
        }
    }
    assert_eq!(
        delivered,
        Some(1),
        "the driver must deliver the queued message at a turn boundary"
    );
    assert!(
        queue.is_empty(),
        "a delivered message leaves the queue empty"
    );
}

#[tokio::test]
async fn a_delivered_message_appends_a_new_turn_and_edits_nothing() {
    // The append-only rule. A steering message adds a user turn. It must never
    // rewrite an earlier one, or the stable prompt prefix breaks and the provider
    // cache goes cold. See SPEC-steering section 7.
    let queue = MessageQueue::new();
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(SteeringTool {
        queue: queue.clone(),
        messages: vec!["also check the parser".to_string()],
    }));
    let session = session_with(
        vec![
            tool_call_turn("c1", "probe", serde_json::json!({})),
            text_turn("done"),
        ],
        tools,
    )
    .with_queue(queue.clone());

    let before = session.messages().await.len();

    let cancel = CancelToken::new();
    let mut events = session.prompt(text("start"), cancel);
    while events.next().await.is_some() {}

    let after = session.messages().await;
    assert!(after.len() > before, "the run must have appended messages");
    // The steering message is present, as a user turn.
    let steer = after
        .iter()
        .find(|message| {
            message.role == Role::User
                && message.content.iter().any(|block| {
                    matches!(block, ContentBlock::Text { text } if text == "also check the parser")
                })
        })
        .expect("the steering message must appear as a user turn");
    assert_eq!(steer.role, Role::User);
    // The first user message is still the original prompt, unedited.
    let first_user = after
        .iter()
        .find(|message| message.role == Role::User)
        .unwrap();
    assert!(
        first_user
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::Text { text } if text == "start")),
        "the first user turn must still be the original prompt, unedited"
    );
}

#[tokio::test]
async fn a_message_after_the_run_ends_stays_queued_for_the_next_run() {
    // No user message is dropped in silence. The ended run has no next turn, so
    // the message waits, and the next run delivers it.
    let session = session_with(
        vec![text_turn("first run"), text_turn("second run")],
        ToolRegistry::new(),
    );
    let queue = session.queue();

    let cancel = CancelToken::new();
    let mut events = session.prompt(text("one"), cancel.clone());
    while events.next().await.is_some() {}

    queue.push(text("late message")).unwrap();
    assert_eq!(queue.len(), 1, "the ended run must not deliver it");

    let mut delivered = None;
    let mut events = session.prompt(text("two"), cancel);
    while let Some(Ok(event)) = events.next().await {
        if let AgentEvent::MessageDelivered { count } = event {
            delivered = Some(count);
        }
    }
    assert_eq!(
        delivered,
        Some(1),
        "the next run must deliver the waiting message"
    );
}

#[tokio::test]
async fn a_cancel_keeps_the_queue() {
    // A queued message is user input. Dropping it as a side effect of a cancel is
    // the worse failure, so a cancel leaves it in place. See D-bounded-steering-queue.
    let session = session_with(vec![text_turn("answer")], ToolRegistry::new());
    let queue = session.queue();
    queue.push(text("keep me")).unwrap();

    let cancel = CancelToken::new();
    cancel.cancel();
    let mut events = session.prompt(text("start"), cancel);
    while events.next().await.is_some() {}

    assert_eq!(
        queue.len(),
        1,
        "a cancel must never drop a queued user message"
    );
}

#[tokio::test]
async fn the_delivered_count_equals_the_drained_count() {
    // The invariant, not one example. Three messages queued mid-run must arrive as
    // one delivery of three, in order.
    let queue = MessageQueue::new();
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(SteeringTool {
        queue: queue.clone(),
        messages: vec!["m0".to_string(), "m1".to_string(), "m2".to_string()],
    }));
    let session = session_with(
        vec![
            tool_call_turn("c1", "probe", serde_json::json!({})),
            text_turn("done"),
        ],
        tools,
    )
    .with_queue(queue.clone());

    let cancel = CancelToken::new();
    let mut events = session.prompt(text("start"), cancel);
    let mut delivered = None;
    while let Some(Ok(event)) = events.next().await {
        if let AgentEvent::MessageDelivered { count } = event {
            delivered = Some(count);
        }
    }
    assert_eq!(
        delivered,
        Some(3),
        "the count must equal what the drain removed"
    );

    let after = session.messages().await;
    let bodies: Vec<&str> = after
        .iter()
        .filter(|m| m.role == Role::User)
        .flat_map(|m| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let positions: Vec<usize> = ["m0", "m1", "m2"]
        .iter()
        .map(|needle| bodies.iter().position(|body| body == needle).unwrap())
        .collect();
    assert!(
        positions[0] < positions[1] && positions[1] < positions[2],
        "arrival order must survive delivery, got {bodies:?}"
    );
    let _ = Duration::from_secs(0);
    let _ = MessageQueue::new();
}

#[tokio::test]
async fn a_queued_message_announces_itself_to_the_frontend() {
    // `MessageQueued` was defined in rho-core and rendered by rho-tui while nothing
    // emitted it, so the "N queued" indicator could never light. That is the same
    // family as the three agent events, and a review caught it here. The spec also
    // claimed the event "fires on a successful push", which was false.
    let queue = MessageQueue::new();
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(SteeringTool {
        queue: queue.clone(),
        messages: vec!["first".to_string(), "second".to_string()],
    }));
    let session = session_with(
        vec![
            tool_call_turn("c1", "probe", serde_json::json!({})),
            text_turn("done"),
        ],
        tools,
    )
    .with_queue(queue.clone());

    let cancel = CancelToken::new();
    let mut events = session.prompt(text("start"), cancel);
    let mut positions = Vec::new();
    let mut delivered = None;
    while let Some(Ok(event)) = events.next().await {
        match event {
            AgentEvent::MessageQueued { position } => positions.push(position),
            AgentEvent::MessageDelivered { count } => delivered = Some(count),
            _ => {}
        }
    }

    assert_eq!(
        positions,
        vec![1, 2],
        "each push must announce its own position, counted from one"
    );
    assert_eq!(delivered, Some(2), "and both must then be delivered");
}

#[tokio::test]
async fn a_queue_set_on_the_config_is_the_one_the_session_uses() {
    // `SessionConfig::with_queue` existed with no caller, and `Session::with_config`
    // built a fresh queue and ignored it. So a caller who set the queue there had
    // every steering message silently dropped. A review found it. Two ways to set one
    // thing is a trap unless both work.
    let queue = MessageQueue::new();
    let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::new(vec![text_turn("done")]));
    let config = common::test_config().with_queue(queue.clone());
    let session = Session::with_config(
        config,
        provider,
        Arc::new(ToolRegistry::new()),
        Arc::new(HookChain::new()),
        Context::new(None, Vec::new()),
    );

    assert_eq!(
        session.queue().len(),
        0,
        "the session starts with the config's queue, which is empty"
    );
    queue.push(text("through the config queue")).unwrap();
    assert_eq!(
        session.queue().len(),
        1,
        "a push on the config's queue must be visible to the session, or it is dropped"
    );
}
