//! End-to-end proof that a tool result feeds back into a provider turn.
//!
//! This is the S7 definition-of-done test. A scripted fake provider asks for a
//! real `rho-tools` `read` call on turn one. The agent loop runs the tool. On
//! turn two the same provider inspects the request it receives and proves the
//! tool result reached it. So the harness does real tool calling end to end.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream;
use rho_core::{
    AgentEvent, CancelToken, CompletionRequest, ContentBlock, Context, HookChain, Provider,
    ProviderError, ProviderStream, Role, Session, SessionConfig, StopReason, StreamEvent,
};
use rho_tools::builtin_registry;

/// A provider that scripts two turns and records the second request it receives.
struct ToolCallingProvider {
    calls: AtomicUsize,
    /// The messages the provider saw on its second call. The test asserts on it.
    second_request_messages: Arc<Mutex<Vec<rho_core::Message>>>,
}

#[async_trait]
impl Provider for ToolCallingProvider {
    fn id(&self) -> &str {
        "tool-calling-fake"
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        _cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let events = if call == 0 {
            // Turn one: ask to read the file the test wrote.
            vec![
                StreamEvent::MessageStart {
                    role: Role::Assistant,
                },
                StreamEvent::ToolCallStart {
                    index: 0,
                    id: "call_1".to_string(),
                    name: "read".to_string(),
                },
                StreamEvent::ToolCallEnd {
                    index: 0,
                    arguments: serde_json::json!({ "path": "note.txt" }),
                },
                StreamEvent::Done {
                    stop_reason: StopReason::ToolUse,
                },
            ]
        } else {
            // Turn two: record the request, then finish.
            *self.second_request_messages.lock().unwrap() = request.messages.clone();
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

#[tokio::test]
async fn tool_result_feeds_back_into_a_provider_turn_end_to_end() {
    // The session root holds one file the model will read.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("note.txt"), "secret payload").unwrap();

    let second = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(ToolCallingProvider {
        calls: AtomicUsize::new(0),
        second_request_messages: Arc::clone(&second),
    });

    let config = SessionConfig::new("test-model", dir.path(), Arc::new(rho_core::AllowAllPolicy));
    let session = Session::with_config(
        config,
        provider,
        Arc::new(builtin_registry()),
        Arc::new(HookChain::new()),
        Context::new(None, Vec::new()),
    );

    // Drive the run to the end.
    let mut events = session.prompt(
        vec![ContentBlock::Text {
            text: "read note.txt".to_string(),
        }],
        CancelToken::new(),
    );

    let mut saw_tool_end = false;
    let mut stop_reason = None;
    while let Some(event) = events.next().await {
        match event.unwrap() {
            AgentEvent::ToolEnd { output, .. } => {
                saw_tool_end = true;
                let text: String = output
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(text, "secret payload", "the tool read the real file");
            }
            AgentEvent::AgentEnd { stop_reason: sr } => stop_reason = Some(sr),
            _ => {}
        }
    }

    assert!(saw_tool_end, "the read tool ran");
    assert_eq!(stop_reason, Some(rho_core::AgentStopReason::EndTurn));

    // The core proof: the tool result reached the provider on the second turn.
    let messages = second.lock().unwrap().clone();
    let found_result = messages.iter().any(|m| {
        m.role == Role::Tool
            && m.content.iter().any(|b| {
                matches!(
                    b,
                    ContentBlock::ToolResult { content, .. }
                        if content.iter().any(|c| matches!(
                            c,
                            ContentBlock::Text { text } if text == "secret payload"
                        ))
                )
            })
    });
    assert!(
        found_result,
        "the tool result must feed back into the next provider request, got: {messages:?}"
    );
}
