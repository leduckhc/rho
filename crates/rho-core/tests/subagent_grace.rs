//! Grace turns: a child is warned before its turn cap, so it can write a summary.
//!
//! Tests for `SPEC-subagent-slots-handles-grace` section 4. Every test uses a
//! scripted fake provider. No network, and no `sleep`.
//!
//! The warning is delivered through the steering queue, so these tests assert on
//! the messages that really reached the model, read back with
//! `Session::messages()`. Asserting on an event alone would not prove the child
//! ever saw the text.

mod common;

use std::sync::Arc;

use common::{ScriptedProvider, text_turn, tool_call_turn};
use futures::StreamExt;
use rho_core::{
    AgentEvent, CancelToken, ContentBlock, Context, HookChain, Message, MessageQueue, Provider,
    Role, Session, ToolRegistry,
};

fn text(body: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text {
        text: body.to_string(),
    }]
}

/// Every user message body in the order the model would have seen it.
fn user_bodies(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .filter(|message| message.role == Role::User)
        .flat_map(|message| {
            message.content.iter().filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
        })
        .collect()
}

/// The grace warning bodies among them. The marker is the stable part of the text.
fn warnings(messages: &[Message]) -> Vec<String> {
    user_bodies(messages)
        .into_iter()
        .filter(|body| body.contains("before rho stops you"))
        .collect()
}

/// A session whose model keeps working until the turn cap stops it.
///
/// The provider answers with a tool call every turn, not text. A text turn ends the
/// run at `EndTurn`, so a text-only script never reaches a turn cap and never
/// reaches the grace window. A first draft of these tests used text and failed for
/// that reason instead of the missing warning, which is the trap AGENTS.md step 5
/// exists to catch.
fn session_that_keeps_working(max_turns: u32, grace_turns: u32) -> Session {
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(common::RecordingTool::new("probe")));
    let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::cycling(tool_call_turn(
        "c1",
        "probe",
        serde_json::json!({}),
    )));
    Session::with_config(
        common::test_config()
            .with_max_turns(max_turns)
            .with_max_tool_calls(1_000)
            .with_grace_turns(grace_turns),
        provider,
        Arc::new(tools),
        Arc::new(HookChain::new()),
        Context::new(None, Vec::new()),
    )
}

/// Run to completion and return the events, so a test can also read the stream.
async fn drain(session: &Session, input: &str) -> Vec<AgentEvent> {
    let mut events = session.prompt(text(input), CancelToken::new());
    let mut seen = Vec::new();
    while let Some(Ok(event)) = events.next().await {
        seen.push(event);
    }
    seen
}

#[tokio::test]
async fn a_child_is_warned_five_turns_before_its_cap() {
    // The default window. A cap of eight and a window of five means the warning
    // fires at the boundary where five turns remain, so the child has five turns
    // left to write its summary.
    let session = session_that_keeps_working(8, 5);
    drain(&session, "start").await;

    let seen = warnings(&session.messages().await);
    assert_eq!(
        seen.len(),
        1,
        "exactly one warning must reach the child, and it did not: {seen:?}"
    );
}

#[tokio::test]
async fn the_warning_states_the_true_turns_remaining() {
    // A number the child cannot trust is worse than no number. With a cap of eight
    // and a window of five, five turns remain when the warning is pushed.
    let session = session_that_keeps_working(8, 5);
    drain(&session, "start").await;

    let seen = warnings(&session.messages().await);
    let first = seen.first().expect("a warning must reach the child");
    assert!(
        first.contains("5 turns left"),
        "the warning must state the true count, and it said: {first}"
    );
}

#[tokio::test]
async fn only_one_grace_warning_reaches_the_child() {
    // Every later boundary is also inside the window, so a driver without a flag
    // would warn on each one. A cap of eight with a window of five gives five
    // boundaries inside the window.
    let session = session_that_keeps_working(8, 5);
    drain(&session, "start").await;

    let seen = warnings(&session.messages().await);
    assert_eq!(seen.len(), 1, "the warning must fire once: {seen:?}");
}

#[tokio::test]
async fn the_warning_does_not_count_as_a_turn() {
    // The warning is a user message at a boundary, exactly like a steer. A driver
    // that counted it would end the run one turn early, so the child would lose
    // the turn the warning asked it to use.
    let with_warning = session_that_keeps_working(6, 5);
    let events = drain(&with_warning, "start").await;
    let turns_with = events
        .iter()
        .filter(|event| matches!(event, AgentEvent::TurnStart))
        .count();

    let without = session_that_keeps_working(6, 0);
    let events = drain(&without, "start").await;
    let turns_without = events
        .iter()
        .filter(|event| matches!(event, AgentEvent::TurnStart))
        .count();

    assert_eq!(
        turns_with, turns_without,
        "the warning must not consume a turn: {turns_with} with it, {turns_without} without"
    );
}

#[tokio::test]
async fn a_child_that_ignores_the_warning_still_reports_out_of_turns() {
    // The outcome keeps its meaning. A child that keeps working past the warning
    // hits the hard cap, and the run stops for the reason it always did.
    let session = session_that_keeps_working(6, 5);
    let events = drain(&session, "start").await;

    // The queue announces a push through its own observer, so `MessageQueued` can
    // arrive after `AgentEnd` in the stream. Asserting on the last event would pin
    // an interleaving instead of the outcome.
    let ended = events
        .iter()
        .find(|event| matches!(event, AgentEvent::AgentEnd { .. }))
        .expect("a run must emit AgentEnd");
    assert!(
        matches!(
            ended,
            AgentEvent::AgentEnd {
                stop_reason: rho_core::AgentStopReason::MaxTurnRequests
            }
        ),
        "the run must still stop at the turn cap, and it ended with: {ended:?}"
    );
}

#[tokio::test]
async fn grace_turns_zero_disables_the_warning() {
    // Zero is off. A caller that wants no warning says so, and the driver obeys.
    let session = session_that_keeps_working(8, 0);
    drain(&session, "start").await;

    assert!(
        warnings(&session.messages().await).is_empty(),
        "a zero window must send nothing"
    );
}

#[tokio::test]
async fn a_plain_session_gets_no_warning_by_default() {
    // `SessionConfig::new` states zero, so a top-level session is unchanged by this
    // feature. Only a subagent opts in, through `SubagentLimits`.
    assert_eq!(
        common::test_config().grace_turns,
        0,
        "a plain session must default to no warning"
    );

    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(common::RecordingTool::new("probe")));
    let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::cycling(tool_call_turn(
        "c1",
        "probe",
        serde_json::json!({}),
    )));
    let session = Session::with_config(
        common::test_config()
            .with_max_turns(8)
            .with_max_tool_calls(1_000),
        provider,
        Arc::new(tools),
        Arc::new(HookChain::new()),
        Context::new(None, Vec::new()),
    );
    drain(&session, "start").await;

    assert!(
        warnings(&session.messages().await).is_empty(),
        "the default must send nothing"
    );
}

#[tokio::test]
async fn a_window_wider_than_the_cap_still_warns_once_and_not_before_the_first_turn() {
    // A caller may state a window larger than the cap. The warning must still fire
    // once, and it must not arrive before the child has done anything, because a
    // warning on turn zero tells the child to summarise nothing.
    let session = session_that_keeps_working(3, 9);
    drain(&session, "start").await;

    let messages = session.messages().await;
    let bodies = user_bodies(&messages);
    assert_eq!(
        warnings(&messages).len(),
        1,
        "a wide window must still warn once: {bodies:?}"
    );

    // The load-bearing assertion, and the first draft did not have it. Asserting
    // that "start" comes first proves nothing: the prompt is appended before the
    // loop, so it is first whether the warning fires before the child works or
    // after. The real property is that the child answered at least once before the
    // warning arrived. A mutation that dropped the `turns >= 1` clamp passed the
    // weaker version of this test.
    let warning_at = messages
        .iter()
        .position(|message| {
            message.content.iter().any(|block| match block {
                ContentBlock::Text { text } => text.contains("before rho stops you"),
                _ => false,
            })
        })
        .expect("the warning must be in the context");
    let assistant_before = messages[..warning_at]
        .iter()
        .filter(|message| message.role == Role::Assistant)
        .count();
    assert!(
        assistant_before >= 1,
        "the child must work before it is told to summarise, and {assistant_before} turns \
         preceded the warning: {bodies:?}"
    );
}

#[tokio::test]
async fn the_driver_reads_the_grace_window_from_the_session_config() {
    // The one copy site. `Session::run` builds the driver's `AgentConfig` from the
    // session config, so a value set through the builder must reach the driver. A
    // window of one fires at the last boundary, which no other window does.
    let session = session_that_keeps_working(4, 1);
    drain(&session, "start").await;

    let seen = warnings(&session.messages().await);
    let first = seen.first().expect("a window of one must still warn");
    assert!(
        first.contains("1 turns left"),
        "the driver must use the configured window: {first}"
    );
}

/// A tool that fills the steering queue while it runs.
///
/// This is the deterministic way to make the queue full at grace time. Reacting to
/// a stream event races the driver, and `sleep` is forbidden.
struct FillingTool {
    queue: MessageQueue,
    fill: usize,
    /// Fill once only. A tool that refilled every turn would keep the queue full
    /// for ever, so the retry could never land and the test would prove nothing.
    filled: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl rho_core::Tool for FillingTool {
    fn name(&self) -> &str {
        "probe"
    }
    fn description(&self) -> &str {
        "a probe that fills its own steering queue"
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
        if !self.filled.swap(true, std::sync::atomic::Ordering::SeqCst) {
            for index in 0..self.fill {
                let _ = self.queue.push(text(&format!("user message {index}")));
            }
        }
        Ok(rho_core::ToolOutput::text("probed"))
    }
}

#[tokio::test]
async fn a_full_queue_at_grace_time_keeps_every_user_message() {
    // rho never drops a user message to make room for its own. The tool fills the
    // queue to capacity during the first turn, and the boundary that follows is
    // inside the grace window. Every user message must arrive.
    let queue = MessageQueue::new();
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(FillingTool {
        queue: queue.clone(),
        fill: rho_core::STEER_QUEUE_CAPACITY,
        filled: std::sync::atomic::AtomicBool::new(false),
    }));

    let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::new(vec![
        tool_call_turn("c1", "probe", serde_json::json!({})),
        text_turn("second"),
        text_turn("third"),
    ]));
    let session = Session::with_config(
        common::test_config().with_max_turns(3).with_grace_turns(3),
        provider,
        Arc::new(tools),
        Arc::new(HookChain::new()),
        Context::new(None, Vec::new()),
    )
    .with_queue(queue.clone());

    drain(&session, "start").await;

    let bodies = user_bodies(&session.messages().await);
    let kept = bodies
        .iter()
        .filter(|body| body.starts_with("user message "))
        .count();
    assert_eq!(
        kept,
        rho_core::STEER_QUEUE_CAPACITY,
        "every user message must survive a grace push, and {kept} did"
    );
}

#[tokio::test]
async fn the_warning_is_retried_after_the_queue_drains() {
    // A skipped warning is not a lost warning. The queue is full at the first
    // boundary inside the window, and the drain empties it in the same step, so the
    // next boundary has room and the warning must land there.
    let queue = MessageQueue::new();
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(FillingTool {
        queue: queue.clone(),
        fill: rho_core::STEER_QUEUE_CAPACITY,
        filled: std::sync::atomic::AtomicBool::new(false),
    }));

    // Every turn is a tool call, so the loop reaches a second boundary. A text turn
    // ends the run at `EndTurn`, and the retry would then never happen.
    let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::cycling(tool_call_turn(
        "c1",
        "probe",
        serde_json::json!({}),
    )));
    let session = Session::with_config(
        common::test_config()
            .with_max_turns(4)
            .with_max_tool_calls(1_000)
            .with_grace_turns(3),
        provider,
        Arc::new(tools),
        Arc::new(HookChain::new()),
        Context::new(None, Vec::new()),
    )
    .with_queue(queue.clone());

    drain(&session, "start").await;

    let messages = session.messages().await;
    let seen = warnings(&messages);
    assert_eq!(
        seen.len(),
        1,
        "the warning must land on a later boundary: {:?}",
        user_bodies(&messages)
    );
}

#[tokio::test]
async fn the_tool_call_budget_gets_no_grace_warning() {
    // A tool-call budget is spent inside a turn, so it has no boundary to warn at.
    // The budget still stops the run, and the child gets no warning about it.
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(common::RecordingTool::new("probe")));

    let provider: Arc<dyn Provider> = Arc::new(ScriptedProvider::cycling(tool_call_turn(
        "c1",
        "probe",
        serde_json::json!({}),
    )));
    let session = Session::with_config(
        common::test_config()
            .with_max_turns(50)
            .with_max_tool_calls(2)
            .with_grace_turns(5),
        provider,
        Arc::new(tools),
        Arc::new(HookChain::new()),
        Context::new(None, Vec::new()),
    );

    let events = drain(&session, "start").await;
    let ended = events
        .iter()
        .find(|event| matches!(event, AgentEvent::AgentEnd { .. }))
        .expect("a run must emit AgentEnd");
    assert!(
        matches!(
            ended,
            AgentEvent::AgentEnd {
                stop_reason: rho_core::AgentStopReason::MaxToolCalls
            }
        ),
        "the budget must still stop the run: {ended:?}"
    );
    assert!(
        warnings(&session.messages().await).is_empty(),
        "the tool-call budget gets no warning"
    );
}
