//! Shared test doubles. No network, no real filesystem, no `sleep`.
//!
//! Every test that needs a session builds one here. The provider is scripted, so a
//! test states the exact stream a turn returns.

use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use futures::stream;

use rho_core::{
    AllowAllPolicy, ApprovalPolicy, CancelToken, CompletionRequest, Context, HookChain, Provider,
    ProviderError, ProviderStream, Role, Session, SessionConfig, StopReason, StreamEvent, Tool,
    ToolContext, ToolError, ToolKind, ToolOutput, ToolRegistry,
};
use rho_jsonl::{Asker, FactoryError, SessionFactory, SessionRequest};

/// What one scripted turn does.
#[derive(Clone)]
pub enum Turn {
    /// Stream this text, then end the turn.
    Text(String),
    /// Fail before the stream starts, as a provider error does.
    Fail(String),
    /// End the stream with no `Done` event. rho-core reports a decode fault.
    NoDone,
    /// Ask to call the `touch` tool, which the approval gate must clear first.
    CallTool,
}

/// A provider that replays a script. Turn one uses script entry one, and so on.
pub struct ScriptedProvider {
    turns: Mutex<std::collections::VecDeque<Turn>>,
}

impl ScriptedProvider {
    pub fn new(turns: Vec<Turn>) -> Self {
        Self {
            turns: Mutex::new(turns.into()),
        }
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn id(&self) -> &str {
        "scripted"
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
        _cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError> {
        let turn = self
            .turns
            .lock()
            .expect("the script is poisoned")
            .pop_front()
            // A script that runs out ends the turn, so a test never hangs on a
            // missing entry.
            .unwrap_or(Turn::Text("done".to_string()));

        match turn {
            Turn::Fail(message) => Err(ProviderError::Transport(message)),
            Turn::NoDone => Ok(Box::pin(stream::iter(vec![
                Ok(StreamEvent::MessageStart {
                    role: Role::Assistant,
                }),
                Ok(StreamEvent::TextStart { index: 0 }),
                Ok(StreamEvent::TextDelta {
                    index: 0,
                    delta: "half".to_string(),
                }),
            ]))),
            Turn::CallTool => Ok(Box::pin(stream::iter(vec![
                Ok(StreamEvent::MessageStart {
                    role: Role::Assistant,
                }),
                Ok(StreamEvent::ToolCallStart {
                    index: 0,
                    id: "call-1".to_string(),
                    name: "touch".to_string(),
                }),
                Ok(StreamEvent::ToolCallEnd {
                    index: 0,
                    arguments: serde_json::json!({ "path": "notes.txt" }),
                    state: None,
                }),
                Ok(StreamEvent::Done {
                    stop_reason: StopReason::ToolUse,
                }),
            ]))),
            Turn::Text(text) => Ok(Box::pin(stream::iter(vec![
                Ok(StreamEvent::MessageStart {
                    role: Role::Assistant,
                }),
                Ok(StreamEvent::TextStart { index: 0 }),
                Ok(StreamEvent::TextDelta {
                    index: 0,
                    delta: text,
                }),
                Ok(StreamEvent::TextEnd { index: 0 }),
                Ok(StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                }),
            ]))),
        }
    }
}

/// A tool that only records that it ran. It declares `Edit`, so a read-only policy
/// denies it and the approval gate must be consulted.
pub struct Touch {
    pub ran: Arc<std::sync::atomic::AtomicUsize>,
    /// When set, the tool waits here before it returns.
    ///
    /// A test that needs a command to arrive **while a turn is busy** cannot race an
    /// instant tool: the turn boundary is gone before the write lands. This lets the test
    /// hold the tool open, send its command, and then release it. `Notify` remembers one
    /// `notify_one` that arrives early, so there is no race either way.
    pub release: Option<Arc<tokio::sync::Notify>>,
}

#[async_trait]
impl Tool for Touch {
    fn name(&self) -> &str {
        "touch"
    }
    fn description(&self) -> &str {
        "Record that the tool ran."
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Edit
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": { "path": { "type": "string" } } })
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        self.ran.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(release) = &self.release {
            release.notified().await;
        }
        Ok(ToolOutput::text("touched"))
    }
}

/// Build a session over a scripted provider, confined to a temporary directory.
///
/// The root is a `tempfile::TempDir`, so no test reads the real `~/.rho` and no test
/// result changes per machine. The caller keeps the `TempDir` alive.
pub fn session_with(
    turns: Vec<Turn>,
    approval: Arc<dyn ApprovalPolicy>,
    root: &tempfile::TempDir,
    ran: Arc<std::sync::atomic::AtomicUsize>,
    release: Option<Arc<tokio::sync::Notify>>,
) -> Session {
    let config = SessionConfig::new("scripted-model", root.path(), approval);
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(Touch { ran, release }));
    Session::with_config(
        config,
        Arc::new(ScriptedProvider::new(turns)),
        Arc::new(tools),
        Arc::new(HookChain::new()),
        Context::new(None, Vec::new()),
    )
}

/// A factory that hands out scripted sessions.
///
/// It refuses the provider name `nosuch`, so a test can drive the unknown-provider
/// path, and `nocreds`, so a test can drive the missing-credential path.
pub struct ScriptedFactory {
    pub turns: Vec<Turn>,
    pub root: tempfile::TempDir,
    /// When set, every session uses an approval policy built from the asker.
    pub ask_for_approval: bool,
    /// How many sessions the factory built.
    ///
    /// `new_session_builds_a_fresh_session` asserts it. Without that assertion this
    /// counter was a comment describing coverage that did not exist.
    pub builds: Arc<std::sync::atomic::AtomicUsize>,
    /// How many times the `touch` tool really ran. A denial must leave it at zero.
    pub tool_ran: Arc<std::sync::atomic::AtomicUsize>,
    /// When set, every `touch` call waits on it, so a test can hold a turn open.
    pub release: Option<Arc<tokio::sync::Notify>>,
    /// The approval timeout every built session uses.
    pub approval_timeout_ms: u64,
}

impl ScriptedFactory {
    pub fn new(turns: Vec<Turn>) -> Self {
        Self {
            turns,
            root: tempfile::tempdir().expect("a temporary directory"),
            ask_for_approval: false,
            builds: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            tool_ran: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            release: None,
            approval_timeout_ms: 50,
        }
    }

    /// Hold every `touch` call until the returned handle is notified.
    pub fn holding(mut self) -> (Self, Arc<tokio::sync::Notify>) {
        let gate = Arc::new(tokio::sync::Notify::new());
        self.release = Some(Arc::clone(&gate));
        (self, gate)
    }

    /// Every session this factory builds then asks the client before a mutating tool
    /// runs. The timeout is short, so a timeout test needs no long wait.
    pub fn asking(mut self) -> Self {
        self.ask_for_approval = true;
        self
    }

    /// Ask before a mutating tool, with a stated approval timeout.
    ///
    /// A test that proves an abort ends a dialog wait needs a timeout long enough that
    /// waiting it out is not a way to pass.
    pub fn asking_with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.ask_for_approval = true;
        self.approval_timeout_ms = timeout_ms;
        self
    }
}

#[async_trait]
impl SessionFactory for ScriptedFactory {
    async fn build(
        &self,
        request: &SessionRequest,
        asker: Arc<dyn Asker>,
    ) -> Result<Session, FactoryError> {
        self.builds
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        match request.provider.as_str() {
            "nosuch" => {
                return Err(FactoryError::UnknownProvider {
                    name: request.provider.clone(),
                });
            }
            "nocreds" => {
                return Err(FactoryError::MissingCredential {
                    provider: request.provider.clone(),
                    variable: "SCRIPTED_KEY".to_string(),
                });
            }
            "badmodel" => {
                return Err(FactoryError::RefusedModel {
                    provider: request.provider.clone(),
                    model_id: request.model_id.clone(),
                    reason: "no such deployment".to_string(),
                });
            }
            "broken" => return Err(FactoryError::Internal("the disk is on fire".to_string())),
            _ => {}
        }
        let approval: Arc<dyn ApprovalPolicy> = if self.ask_for_approval {
            Arc::new(rho_jsonl::DialogApproval::with_timeout_ms(
                asker,
                self.approval_timeout_ms,
            ))
        } else {
            Arc::new(AllowAllPolicy)
        };
        Ok(session_with(
            self.turns.clone(),
            approval,
            &self.root,
            Arc::clone(&self.tool_ran),
            self.release.clone(),
        ))
    }
}
