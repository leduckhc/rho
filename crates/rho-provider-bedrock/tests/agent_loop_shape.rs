//! The shape rho's own agent loop builds, taken from the loop and not written by hand.
//!
//! **This is the test that would have caught the defect on the day it landed.** Every unit
//! test of the replay scope built its message list by hand, and all of them ended it with an
//! assistant turn. rho never sends that: it appends the tool results and then builds the
//! request, so the last message is always a tool result. The scope rule ended the pending run
//! at a tool result, which emptied it, and no reasoning reached Bedrock for seven days behind
//! nine green tests and a green live run.
//!
//! So this test does not describe the shape. It **asks the agent loop for it**. A fake
//! provider mints a reasoning payload and a tool call on turn one, and records the request it
//! receives on turn two. Those recorded messages, exactly as `rho-core` assembled them, go
//! into `build_messages_for_model`.
//!
//! The guard holds for the whole class: if `rho-core` ever changes how it orders or merges the
//! messages of a loop, this test sees the new shape without an edit. See
//! `D-the-pending-run-includes-the-turn-a-tool-result-answers`.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream;
use rho_core::{
    AllowAllPolicy, CancelToken, CompletionRequest, ContentBlock, Context, HookChain, Provider,
    ProviderError, ProviderState, ProviderStream, ReasoningOwner, Role, Session, SessionConfig,
    StopReason, StreamEvent, Tool, ToolContext, ToolError, ToolKind, ToolOutput, ToolRegistry,
};

const MODEL: &str = "us.anthropic.claude-haiku-4-5-20251001-v1:0";

/// A tool that exists only so the agent loop has something to run, and so a tool result lands
/// in the transcript the way a real one does.
struct Echo;

#[async_trait]
impl Tool for Echo {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "returns a fixed string"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Read
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text("a tool result"))
    }
}

/// Turn one mints reasoning and calls the tool. Turn two records what it was sent.
struct RecordingProvider {
    calls: AtomicUsize,
    second_request: Arc<Mutex<Vec<rho_core::Message>>>,
}

#[async_trait]
impl Provider for RecordingProvider {
    fn id(&self) -> &str {
        // The owner rule compares this against the payload's owner, so it must be the real id.
        "bedrock"
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        _cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let events = if call == 0 {
            vec![
                StreamEvent::MessageStart {
                    role: Role::Assistant,
                },
                StreamEvent::ThinkingStart { index: 0 },
                StreamEvent::ThinkingDelta {
                    index: 0,
                    delta: "the pending thought".to_string(),
                },
                StreamEvent::ThinkingEnd {
                    index: 0,
                    state: Some(ProviderState {
                        owner: ReasoningOwner {
                            provider: "bedrock".to_string(),
                            model: MODEL.to_string(),
                        },
                        value: serde_json::json!({ "signature": "a-real-signature" }),
                    }),
                },
                StreamEvent::ToolCallStart {
                    index: 1,
                    id: "call_1".to_string(),
                    name: "echo".to_string(),
                },
                StreamEvent::ToolCallEnd {
                    index: 1,
                    arguments: serde_json::json!({}),
                    state: None,
                },
                StreamEvent::Done {
                    stop_reason: StopReason::ToolUse,
                },
            ]
        } else {
            // The messages the loop assembled after it ran the tool. This is the request rho
            // really sends, and it is what the assertion below reads.
            *self.second_request.lock().unwrap() = request.messages.clone();
            vec![
                StreamEvent::MessageStart {
                    role: Role::Assistant,
                },
                StreamEvent::TextStart { index: 0 },
                StreamEvent::TextDelta {
                    index: 0,
                    delta: "done".to_string(),
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

/// The request rho actually builds after a tool result must carry the pending turn's thinking.
#[tokio::test]
async fn the_request_the_agent_loop_builds_carries_its_reasoning() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let second_request = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(RecordingProvider {
        calls: AtomicUsize::new(0),
        second_request: Arc::clone(&second_request),
    });

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(Echo));
    let session = Session::with_config(
        SessionConfig::new(MODEL, dir.path(), Arc::new(AllowAllPolicy)),
        provider,
        Arc::new(registry),
        Arc::new(HookChain::new()),
        Context::new(None, Vec::new()),
    );

    let mut events = session.prompt(
        vec![ContentBlock::Text {
            text: "use the tool".to_string(),
        }],
        CancelToken::new(),
    );
    while let Some(event) = events.next().await {
        event.expect("the loop runs to the end");
    }

    let messages = second_request.lock().unwrap().clone();

    // First, prove the harness produced the shape this test is about. Without this, the
    // assertion below could pass against a loop that never ran at all.
    assert!(
        matches!(messages.last().map(|m| &m.role), Some(Role::Tool)),
        "the loop must end its request with a tool result, or this test proves nothing: {:?}",
        messages.iter().map(|m| &m.role).collect::<Vec<_>>()
    );
    assert!(
        messages.iter().any(|message| message
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::ReasoningReplay { .. }))),
        "the transcript must hold a replayable payload: {messages:?}"
    );

    // Now the invariant: that payload reaches the wire.
    use aws_sdk_bedrockruntime::types::{
        ContentBlock as SdkBlock, ReasoningContentBlock as SdkReasoning,
    };
    let sent: Vec<String> = rho_provider_bedrock::build_messages_for_model(&messages, MODEL)
        .messages
        .iter()
        .flat_map(|message| message.content().iter())
        .filter_map(|block| match block {
            SdkBlock::ReasoningContent(SdkReasoning::ReasoningText(text)) => {
                Some(text.text().to_string())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        sent,
        vec!["the pending thought".to_string()],
        "Anthropic needs the thinking of the turn that made the call it is answering"
    );
}
