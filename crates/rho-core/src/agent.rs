//! The agent loop.
//!
//! The loop drives one full run. A run may span several provider turns because a
//! turn can call tools. The loop appends every message to the `Context`. It
//! emits an `AgentEvent` stream. A frontend renders the stream.

use crate::{
    ApprovalDecision, ApprovalPolicy, CancelToken, ContentBlock, Message, Role, SandboxMode,
};
use crate::{
    CompletionRequest, Context, Error, HookChain, HookOutcome, Provider, StopReason, StreamEvent,
    ToolCallView, ToolContext, ToolError, ToolKind, ToolOutput, ToolRegistry,
};
use futures::Stream;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

/// Why a full agent run stopped. The wire names match the ACP `StopReason` set,
/// so `rho-acp` maps the values one-to-one onto a `session/prompt` response.
/// See SPEC-acp.
///
/// One name needs an explicit rename. ACP spells the cancelled reason with two
/// letters `l`, as `cancelled`. Rust names the variant `Canceled` with one `l`,
/// which `rename_all = "snake_case"` would turn into `canceled`. That value is
/// not valid in ACP. The `serde(rename)` attribute below corrects it. Do not
/// remove the attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStopReason {
    /// The model finished and asked for no more tools.
    EndTurn,
    /// A turn hit the token limit.
    MaxTokens,
    /// The run hit its tool-call budget.
    ///
    /// A turn cap counts provider round trips. It does not bound a run that makes
    /// forty tool calls inside one turn, so the budget counts the work instead.
    MaxToolCalls,
    /// The loop hit its per-run turn cap. See section 9.
    MaxTurnRequests,
    /// The model refused, or a content filter stopped the output.
    Refusal,
    /// The caller cancelled the run. The wire value is `cancelled`.
    #[serde(rename = "cancelled")]
    Canceled,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvent {
    /// One provider turn begins.
    TurnStart,
    /// A normalised provider event.
    Stream(StreamEvent),
    /// A tool begins execution, after hooks and the approval policy pass.
    ToolStart {
        id: String,
        name: String,
        kind: ToolKind,
    },
    /// A streamed line of tool output.
    ToolUpdate { id: String, output: String },
    /// A tool finished. The output feeds the next turn.
    ToolEnd { id: String, output: ToolOutput },
    /// One provider turn ended.
    TurnEnd { stop_reason: StopReason },
    /// A background task started.
    TaskStart {
        id: crate::TaskId,
        command: String,
        reason: crate::BackgroundReason,
    },
    /// A task reported progress.
    TaskProgressed {
        id: crate::TaskId,
        progress: crate::TaskProgress,
    },
    /// A task reached a final state. Always emitted, success or failure.
    TaskEnd {
        id: crate::TaskId,
        state: crate::TaskState,
        output_tail: String,
    },
    /// The run is fully settled. No further turn will run.
    AgentEnd { stop_reason: AgentStopReason },
    /// A subagent was spawned under this session. See SPEC-subagents section 9.
    ///
    /// These variants are new, not reused `TaskStart` ones. A task is an
    /// operating-system command with an exit code. An agent has turns, token
    /// usage, and a summary. Sharing one variant would force a frontend to guess
    /// which it held.
    AgentSpawned {
        id: crate::AgentId,
        agent: String,
        depth: u32,
    },
    /// A subagent reported progress: its turn count and summed usage so far.
    AgentProgressed {
        id: crate::AgentId,
        turns: u32,
        usage: crate::Usage,
    },
    /// A subagent finished. The report carries the summary, not the transcript.
    AgentFinished {
        id: crate::AgentId,
        report: crate::AgentReport,
    },
}

/// Configuration for one agent run.
#[derive(Clone, Copy, Debug)]
pub struct AgentConfig {
    /// The per-run turn cap. The loop stops with `MaxTurnRequests` at the cap.
    pub max_turns: u32,
    /// The per-run tool-call budget. The loop stops with `MaxToolCalls` at the cap.
    pub max_tool_calls: u32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_turns: 32,
            // Sixteen turns of four calls each. High enough that ordinary work
            // never notices, low enough that a loop cannot run all night.
            max_tool_calls: 64,
        }
    }
}

/// The event stream of one agent run.
///
/// Drop this value to cancel the run. `Drop` aborts the driver task, which drops
/// the provider stream and any running tool future. No task leaks.
pub struct AgentEvents {
    rx: tokio::sync::mpsc::Receiver<Result<AgentEvent, Error>>,
    handle: tokio::task::JoinHandle<()>,
}

impl Stream for AgentEvents {
    type Item = Result<AgentEvent, Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

impl Drop for AgentEvents {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// The configuration one `Session` needs before it can run.
///
/// It carries the model id, the confinement root, the approval policy, and the
/// per-run turn cap. `Session` holds no default for any of these. A caller states
/// each value, so a security boundary is never set by accident. See decision
/// D-session-config.
#[derive(Clone)]
pub struct SessionConfig {
    /// The model id sent in every `CompletionRequest`. An empty id fails at the
    /// provider with a useless message, so the caller must set a real id.
    pub model: String,
    /// The path confinement root. Every tool resolves its paths under this root.
    /// It has no default. A caller states it.
    pub session_root: PathBuf,
    /// The approval policy. Tool dispatch consults it before it runs a tool.
    pub approval: Arc<dyn ApprovalPolicy>,
    /// The per-run turn cap. The loop stops with `MaxTurnRequests` at the cap.
    pub max_turns: u32,
    /// The per-run tool-call budget. The loop stops with `MaxToolCalls` at the cap.
    pub max_tool_calls: u32,
    /// The `bash` confinement mode. `SessionConfig::new` sets `Off`, so the
    /// default is stated here, not hidden. Use `with_sandbox` to change it. See
    /// `SPEC-bash-sandbox` and decision D-bash-os-sandbox.
    pub sandbox: SandboxMode,
}

impl SessionConfig {
    /// Build a config with an explicit model, root, and policy. The turn cap uses
    /// the `AgentConfig` default.
    pub fn new(
        model: impl Into<String>,
        session_root: impl Into<PathBuf>,
        approval: Arc<dyn ApprovalPolicy>,
    ) -> Self {
        Self {
            model: model.into(),
            session_root: session_root.into(),
            approval,
            max_turns: AgentConfig::default().max_turns,
            max_tool_calls: AgentConfig::default().max_tool_calls,
            // State the default out loud. `Off` runs `bash` unconfined, which is
            // today's behaviour. A caller opts in with `with_sandbox`.
            sandbox: SandboxMode::Off,
        }
    }

    /// Build a config that confines paths to the current directory. This states
    /// the choice in the calling code, so a current-directory root is never an
    /// accident. It fails when the current directory cannot be read.
    pub fn for_current_dir(
        model: impl Into<String>,
        approval: Arc<dyn ApprovalPolicy>,
    ) -> std::io::Result<Self> {
        Ok(Self::new(model, std::env::current_dir()?, approval))
    }

    /// Override the per-run turn cap.
    pub fn with_max_turns(mut self, max_turns: u32) -> Self {
        self.max_turns = max_turns;
        self
    }

    /// Set the per-run tool-call budget.
    pub fn with_max_tool_calls(mut self, max_tool_calls: u32) -> Self {
        self.max_tool_calls = max_tool_calls;
        self
    }

    /// Set the `bash` confinement mode. `new` leaves it `Off`. See `SPEC-bash-sandbox`.
    pub fn with_sandbox(mut self, sandbox: SandboxMode) -> Self {
        self.sandbox = sandbox;
        self
    }
}

pub struct Session {
    inner: Arc<SessionInner>,
}

struct SessionInner {
    provider: Arc<dyn Provider>,
    tools: Arc<ToolRegistry>,
    hooks: Arc<HookChain>,
    context: tokio::sync::Mutex<Context>,
    config: SessionConfig,
}

impl Session {
    /// Build a session with an explicit `SessionConfig`. This is the primary
    /// constructor. See decision D-session-config.
    pub fn with_config(
        config: SessionConfig,
        provider: Arc<dyn Provider>,
        tools: Arc<ToolRegistry>,
        hooks: Arc<HookChain>,
        context: Context,
    ) -> Self {
        Self {
            inner: Arc::new(SessionInner {
                provider,
                tools,
                hooks,
                context: tokio::sync::Mutex::new(context),
                config,
            }),
        }
    }

    /// Build a session with a test configuration. The config confines paths to
    /// the current directory and allows every tool call. Use it in a test where
    /// the model id and the root do not matter. A production caller uses
    /// `with_config` and states a real config.
    /// Start one agent run. Append `input` to the context, then drive the loop.
    /// `cancel` stops the run. Dropping the returned value also stops the run.
    pub fn prompt(&self, input: Vec<ContentBlock>, cancel: CancelToken) -> AgentEvents {
        // The channel carries events from the driver task to the caller. A
        // bounded channel applies backpressure. A dropped receiver makes `send`
        // fail, so the driver task stops on its own.
        let (tx, rx) = tokio::sync::mpsc::channel(EVENT_CHANNEL_CAPACITY);
        let driver = Driver {
            tool_calls: std::sync::atomic::AtomicU32::new(0),
            config: AgentConfig {
                max_turns: self.inner.config.max_turns,
                max_tool_calls: self.inner.config.max_tool_calls,
            },
            inner: Arc::clone(&self.inner),
            tx,
            cancel,
        };
        let handle = tokio::spawn(driver.run(input));
        AgentEvents { rx, handle }
    }

    /// Read the conversation so far. The context is append-only, so this returns
    /// a read-only snapshot. A lock guards the context, so this clones the
    /// messages instead of borrowing them.
    pub async fn messages(&self) -> Vec<Message> {
        self.inner.context.lock().await.messages().to_vec()
    }
}

/// The event channel buffer size. It applies backpressure to the driver task.
const EVENT_CHANNEL_CAPACITY: usize = 64;

/// The result of one provider turn.
enum TurnOutcome {
    /// The run must stop with this reason.
    Stop(AgentStopReason),
    /// The model asked to call these tools. Run them, then loop.
    ToolCalls(Vec<PendingToolCall>),
    /// The caller cancelled the turn.
    Canceled,
    /// The provider or a tool failed. The error is already sent.
    Failed,
    /// The caller dropped the event stream.
    Closed,
}

/// One tool call the model requested in a turn.
struct PendingToolCall {
    id: String,
    name: String,
    arguments: serde_json::Value,
}

/// The task that drives one agent run.
struct Driver {
    inner: Arc<SessionInner>,
    tx: tokio::sync::mpsc::Sender<Result<AgentEvent, Error>>,
    cancel: CancelToken,
    config: AgentConfig,
    /// Tool calls made in this run, against `AgentConfig::max_tool_calls`.
    tool_calls: std::sync::atomic::AtomicU32,
}

impl Driver {
    /// Drive the whole run. Append the user input, then run turns until a stop.
    async fn run(self, input: Vec<ContentBlock>) {
        {
            let mut context = self.inner.context.lock().await;
            context.append(Message {
                role: Role::User,
                content: input,
            });
        }

        let mut turns = 0u32;
        let stop_reason = loop {
            if self.cancel.is_cancelled() {
                // The cancel landed before this turn started. Emit a paired
                // `TurnStart` and `TurnEnd`, so a frontend never sees an
                // unpaired `TurnEnd`. See the tests in agent_loop.rs.
                if self.emit(AgentEvent::TurnStart).await.is_err() {
                    return;
                }
                if self
                    .emit(AgentEvent::TurnEnd {
                        stop_reason: StopReason::Canceled,
                    })
                    .await
                    .is_err()
                {
                    return;
                }
                break AgentStopReason::Canceled;
            }
            if turns >= self.config.max_turns {
                break AgentStopReason::MaxTurnRequests;
            }
            turns += 1;

            match self.run_turn().await {
                TurnOutcome::Stop(reason) => break reason,
                TurnOutcome::Canceled => break AgentStopReason::Canceled,
                TurnOutcome::Failed | TurnOutcome::Closed => return,
                TurnOutcome::ToolCalls(calls) => match self.dispatch(calls).await {
                    DispatchOutcome::BudgetSpent => break AgentStopReason::MaxToolCalls,
                    DispatchOutcome::Continue => continue,
                    DispatchOutcome::Canceled => {
                        if self
                            .emit(AgentEvent::TurnEnd {
                                stop_reason: StopReason::Canceled,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                        break AgentStopReason::Canceled;
                    }
                    DispatchOutcome::Closed => return,
                },
            }
        };

        let _ = self.emit(AgentEvent::AgentEnd { stop_reason }).await;
    }

    /// Run one provider turn. Forward each stream event. Build the assistant
    /// message. Append it to the context. Return the next step.
    async fn run_turn(&self) -> TurnOutcome {
        if self.emit(AgentEvent::TurnStart).await.is_err() {
            return TurnOutcome::Closed;
        }

        let request = self.build_request().await;
        let mut stream = match self
            .inner
            .provider
            .stream(request, self.cancel.clone())
            .await
        {
            Ok(stream) => stream,
            Err(error) => {
                let _ = self.tx.send(Err(Error::from(error))).await;
                return TurnOutcome::Failed;
            }
        };

        let mut builder = AssistantBuilder::default();
        let mut stop_reason = None;
        loop {
            if self.cancel.is_cancelled() {
                if self
                    .emit(AgentEvent::TurnEnd {
                        stop_reason: StopReason::Canceled,
                    })
                    .await
                    .is_err()
                {
                    return TurnOutcome::Closed;
                }
                return TurnOutcome::Canceled;
            }

            let next = tokio::select! {
                biased;
                _ = self.cancel.cancelled() => {
                    if self
                        .emit(AgentEvent::TurnEnd {
                            stop_reason: StopReason::Canceled,
                        })
                        .await
                        .is_err()
                    {
                        return TurnOutcome::Closed;
                    }
                    return TurnOutcome::Canceled;
                }
                item = stream.next() => item,
            };

            match next {
                Some(Ok(event)) => {
                    if let StreamEvent::Done {
                        stop_reason: reason,
                    } = &event
                    {
                        stop_reason = Some(*reason);
                    }
                    builder.observe(&event);
                    if self.emit(AgentEvent::Stream(event)).await.is_err() {
                        return TurnOutcome::Closed;
                    }
                    if stop_reason.is_some() {
                        break;
                    }
                }
                Some(Err(error)) => {
                    let _ = self.tx.send(Err(Error::from(error))).await;
                    return TurnOutcome::Failed;
                }
                None => break,
            }
        }

        let (message, tool_calls) = builder.finish();
        {
            let mut context = self.inner.context.lock().await;
            context.append(message);
        }

        match stop_reason {
            Some(StopReason::ToolUse) => {
                // Close the turn before the tools run. Every other exit path emits
                // `TurnEnd`, and this one used to skip it, so a tool-calling turn left
                // its `TurnStart` unpaired forever.
                //
                // A frontend pairs the two events to track state. `rho-tui` decides
                // whether the agent is running, and `rho-acp` maps the pair onto a
                // session update. An unpaired start leaks that state.
                if self
                    .emit(AgentEvent::TurnEnd {
                        stop_reason: StopReason::ToolUse,
                    })
                    .await
                    .is_err()
                {
                    return TurnOutcome::Closed;
                }
                TurnOutcome::ToolCalls(tool_calls)
            }
            Some(reason) => {
                if self
                    .emit(AgentEvent::TurnEnd {
                        stop_reason: reason,
                    })
                    .await
                    .is_err()
                {
                    return TurnOutcome::Closed;
                }
                TurnOutcome::Stop(map_stop_reason(reason))
            }
            None => {
                if self.cancel.is_cancelled() {
                    if self
                        .emit(AgentEvent::TurnEnd {
                            stop_reason: StopReason::Canceled,
                        })
                        .await
                        .is_err()
                    {
                        return TurnOutcome::Closed;
                    }
                    return TurnOutcome::Canceled;
                }
                // The provider ended the stream with no `Done` event. Report a
                // decode fault so the caller can retry or stop.
                let _ = self
                    .tx
                    .send(Err(Error::from(crate::ProviderError::Decode(
                        "the provider stream ended without a done event".to_string(),
                    ))))
                    .await;
                TurnOutcome::Failed
            }
        }
    }

    /// Run every requested tool call, one at a time, in call order.
    ///
    /// It stops at the tool-call budget. The turn cap counts provider round trips,
    /// so it cannot bound a turn that asks for forty tools at once. The budget
    /// counts the work, and it is checked **before** each call, so the cap is never
    /// exceeded rather than merely noticed afterwards.
    async fn dispatch(&self, calls: Vec<PendingToolCall>) -> DispatchOutcome {
        for call in calls {
            if self.tool_calls.load(std::sync::atomic::Ordering::SeqCst)
                >= self.config.max_tool_calls
            {
                return DispatchOutcome::BudgetSpent;
            }
            self.tool_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match self.dispatch_one(call).await {
                DispatchOutcome::Continue => continue,
                other => return other,
            }
        }
        DispatchOutcome::Continue
    }

    /// Run the hooks, the approval step, and one tool. Append the result.
    async fn dispatch_one(&self, call: PendingToolCall) -> DispatchOutcome {
        if self.cancel.is_cancelled() {
            return DispatchOutcome::Canceled;
        }

        let PendingToolCall {
            id,
            name,
            mut arguments,
        } = call;

        // Run `before_tool_call` in registration order. The first block wins.
        let mut blocked: Option<String> = None;
        for hook in self.inner.hooks.hooks() {
            let mut view = ToolCallView {
                name: &name,
                arguments: &mut arguments,
            };
            match hook.before_tool_call(&mut view).await {
                HookOutcome::Continue => {}
                HookOutcome::Block { reason } => {
                    blocked = Some(reason);
                    break;
                }
            }
        }

        if let Some(reason) = blocked {
            let output = error_output(reason);
            return self.finish_tool(&id, output).await;
        }

        let tool = self.inner.tools.get(&name).cloned();
        let Some(tool) = tool else {
            let output = error_output(format!("the tool {name} is not registered"));
            return self.finish_tool(&id, output).await;
        };

        // Consult the approval policy after the hooks. A denial produces an error
        // tool result from the typed `ToolError::Denied` variant. The tool never
        // runs. See SPEC-core-runtime section 9 and SPEC-tool-interface section 5.
        if self
            .inner
            .config
            .approval
            .approve(&name, tool.kind(), &arguments)
            .await
            == ApprovalDecision::Deny
        {
            let output = error_output(ToolError::Denied.to_string());
            return self.finish_tool(&id, output).await;
        }

        if self
            .emit(AgentEvent::ToolStart {
                id: id.clone(),
                name: name.clone(),
                kind: tool.kind(),
            })
            .await
            .is_err()
        {
            return DispatchOutcome::Closed;
        }

        // A tool streams output lines on this channel. Forward each line as a
        // `ToolUpdate` while the tool runs.
        let (updates_tx, mut updates_rx) =
            tokio::sync::mpsc::channel::<String>(EVENT_CHANNEL_CAPACITY);
        // A tool that runs a subagent sends typed events here. Forward each one
        // unchanged, so a frontend sees the child while it runs.
        let (agent_tx, mut agent_rx) =
            tokio::sync::mpsc::channel::<AgentEvent>(EVENT_CHANNEL_CAPACITY);
        let context = ToolContext {
            session_root: self.inner.config.session_root.clone(),
            cancel: self.cancel.clone(),
            updates: updates_tx,
            agent_events: agent_tx,
        };
        let execute = tool.execute(arguments, context);
        tokio::pin!(execute);

        let result = loop {
            tokio::select! {
                biased;
                _ = self.cancel.cancelled() => return DispatchOutcome::Canceled,
                line = updates_rx.recv() => {
                    if let Some(line) = line
                        && self
                            .emit(AgentEvent::ToolUpdate {
                                id: id.clone(),
                                output: line,
                            })
                            .await
                            .is_err()
                    {
                        return DispatchOutcome::Closed;
                    }
                }
                event = agent_rx.recv() => {
                    if let Some(event) = event
                        && self.emit(event).await.is_err()
                    {
                        return DispatchOutcome::Closed;
                    }
                }
                done = &mut execute => break done,
            }
        };

        // Drain any agent event the tool sent just before it returned. This is the
        // arm that carries `AgentFinished`, so skipping it would lose the report.
        while let Ok(event) = agent_rx.try_recv() {
            if self.emit(event).await.is_err() {
                return DispatchOutcome::Closed;
            }
        }

        // Drain any output line the tool sent just before it returned.
        while let Ok(line) = updates_rx.try_recv() {
            if self
                .emit(AgentEvent::ToolUpdate {
                    id: id.clone(),
                    output: line,
                })
                .await
                .is_err()
            {
                return DispatchOutcome::Closed;
            }
        }
        let mut output = match result {
            Ok(output) => output,
            Err(error) => {
                // A tool failure is a result, not the end of the run.
                //
                // A live smoke test found the opposite behaviour. A missing file
                // aborted the whole session, and the model never learned why. That
                // makes the harness unusable, because almost every real session has a
                // tool error: a file is missing, a grep matches nothing, a command
                // exits non-zero. The model must see the failure so it can try
                // something else.
                //
                // Only a transport or provider fault ends a run. A tool error becomes
                // an error tool result, exactly like a blocked call or an unknown tool.
                error_output(error.to_string())
            }
        };

        // Run `after_tool_result` in registration order. A hook may edit output.
        for hook in self.inner.hooks.hooks() {
            hook.after_tool_result(&name, &mut output).await;
        }

        self.finish_tool(&id, output).await
    }

    /// Emit `ToolEnd` and append the tool-result message to the context.
    async fn finish_tool(&self, id: &str, output: ToolOutput) -> DispatchOutcome {
        let message = Message {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                tool_call_id: id.to_string(),
                content: output.content.clone(),
                is_error: output.is_error,
            }],
        };
        {
            let mut context = self.inner.context.lock().await;
            context.append(message);
        }
        if self
            .emit(AgentEvent::ToolEnd {
                id: id.to_string(),
                output,
            })
            .await
            .is_err()
        {
            return DispatchOutcome::Closed;
        }
        DispatchOutcome::Continue
    }

    /// Build the next request from the current context. The system prompt and
    /// the tool list form the stable prefix. The messages grow by appending.
    async fn build_request(&self) -> CompletionRequest {
        let context = self.inner.context.lock().await;
        CompletionRequest {
            model: self.inner.config.model.clone(),
            system: context.system().map(str::to_string),
            messages: context.messages().to_vec(),
            tools: self.inner.tools.specs(),
            max_tokens: None,
            temperature: None,
        }
    }

    /// Send one event. Return `Err` when the caller dropped the receiver.
    async fn emit(&self, event: AgentEvent) -> Result<(), ()> {
        self.tx.send(Ok(event)).await.map_err(|_| ())
    }
}

/// The result of running one or more tool calls.
///
/// There is deliberately no `Failed` variant. A tool failure is a result that goes
/// back to the model, not an end to the run. Only a transport or provider fault
/// ends a run. See the tool-error branch in `dispatch_one`.
enum DispatchOutcome {
    /// The loop may run the next turn.
    Continue,
    /// The run spent its tool-call budget. The loop stops.
    BudgetSpent,
    /// The caller cancelled the run.
    Canceled,
    /// The caller dropped the event stream.
    Closed,
}

/// Map a provider stop reason onto an agent stop reason. See SPEC-core-runtime section 9.
fn map_stop_reason(reason: StopReason) -> AgentStopReason {
    match reason {
        StopReason::EndTurn | StopReason::StopSequence => AgentStopReason::EndTurn,
        StopReason::MaxTokens => AgentStopReason::MaxTokens,
        StopReason::ContentFiltered => AgentStopReason::Refusal,
        StopReason::Canceled => AgentStopReason::Canceled,
        // A `ToolUse` stop is not terminal. The loop handles it before this call.
        StopReason::ToolUse => AgentStopReason::EndTurn,
    }
}

/// Build an error tool result that carries a plain-text reason for the model.
fn error_output(reason: impl Into<String>) -> ToolOutput {
    ToolOutput {
        content: vec![ContentBlock::Text {
            text: reason.into(),
        }],
        is_error: true,
    }
}

/// Assembles the assistant message from a turn's stream events. It closes each
/// content block on its `*End` event, in `index` order.
#[derive(Default)]
struct AssistantBuilder {
    content: Vec<ContentBlock>,
    tool_calls: Vec<PendingToolCall>,
    text: Option<String>,
    thinking: Option<String>,
    tool_call: Option<(String, String)>,
}

impl AssistantBuilder {
    /// Fold one stream event into the message under construction.
    fn observe(&mut self, event: &StreamEvent) {
        match event {
            StreamEvent::TextStart { .. } => self.text = Some(String::new()),
            StreamEvent::TextDelta { delta, .. } => {
                if let Some(text) = self.text.as_mut() {
                    text.push_str(delta);
                }
            }
            StreamEvent::TextEnd { .. } => {
                if let Some(text) = self.text.take() {
                    self.content.push(ContentBlock::Text { text });
                }
            }
            StreamEvent::ThinkingStart { .. } => self.thinking = Some(String::new()),
            StreamEvent::ThinkingDelta { delta, .. } => {
                if let Some(thinking) = self.thinking.as_mut() {
                    thinking.push_str(delta);
                }
            }
            StreamEvent::ThinkingEnd { signature, .. } => {
                if let Some(thinking) = self.thinking.take() {
                    self.content.push(ContentBlock::Thinking {
                        thinking,
                        signature: signature.clone(),
                    });
                }
            }
            StreamEvent::ToolCallStart { id, name, .. } => {
                self.tool_call = Some((id.clone(), name.clone()));
            }
            StreamEvent::ToolCallEnd { arguments, .. } => {
                if let Some((id, name)) = self.tool_call.take() {
                    self.content.push(ContentBlock::ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: arguments.clone(),
                    });
                    self.tool_calls.push(PendingToolCall {
                        id,
                        name,
                        arguments: arguments.clone(),
                    });
                }
            }
            StreamEvent::MessageStart { .. }
            | StreamEvent::ToolCallDelta { .. }
            | StreamEvent::Usage(_)
            | StreamEvent::Done { .. } => {}
        }
    }

    /// Finish the message. Return it with the tool calls it requested.
    fn finish(self) -> (Message, Vec<PendingToolCall>) {
        (
            Message {
                role: Role::Assistant,
                content: self.content,
            },
            self.tool_calls,
        )
    }
}
