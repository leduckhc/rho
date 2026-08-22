//! The AWS Bedrock provider.
//!
//! The wire-to-event mapping is a set of pure functions. A test drives them
//! against recorded `ConverseStream` event payloads, with no AWS client and no
//! network. See `SPEC-provider-interface` section 5.
//!
//! The real `Provider::stream` uses `aws-sdk-bedrockruntime` and signs with
//! SigV4 from the standard credential chain. It converts each SDK event into the
//! same recorded mirror the tests use, then feeds `map_converse_event`. This
//! split lets the tests avoid the AWS event-stream binary framing.

use async_stream::stream;
use async_trait::async_trait;
use aws_sdk_bedrockruntime::error::ProvideErrorMetadata;
use aws_smithy_types::Document;
use aws_smithy_types::Number;
use rho_core::{
    CancelToken, CompletionRequest, Message, Provider, ProviderError, ProviderStream, Role,
    StopReason, StreamEvent, Usage,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

// --- The recorded event mirror. -----------------------------------------
//
// These types deserialise the JSON shape of a `ConverseStream` event. AWS uses
// camelCase on the wire. The provider matches the SDK enums in production, but
// the mapping consumes this mirror, so a test can build events from a fixture.

/// One `ConverseStream` event. Exactly one field is set per event.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct ConverseStreamEvent {
    pub message_start: Option<MessageStart>,
    pub content_block_start: Option<ContentBlockStart>,
    pub content_block_delta: Option<ContentBlockDelta>,
    pub content_block_stop: Option<ContentBlockStop>,
    pub message_stop: Option<MessageStop>,
    pub metadata: Option<Metadata>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageStart {
    pub role: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentBlockStart {
    pub content_block_index: u32,
    pub start: BlockStart,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockStart {
    pub tool_use: Option<ToolUseStart>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUseStart {
    pub tool_use_id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentBlockDelta {
    pub content_block_index: u32,
    pub delta: BlockDelta,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockDelta {
    pub text: Option<String>,
    pub tool_use: Option<ToolUseDelta>,
    pub reasoning_content: Option<ReasoningDelta>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUseDelta {
    pub input: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningDelta {
    /// The readable reasoning. Absent on a signature-only or redacted delta.
    #[serde(default)]
    pub text: Option<String>,
    /// The token that proves the model wrote the text. The AWS SDK states the rule: "If you
    /// pass a reasoning block back to the API in a multi-turn conversation, include the text
    /// and its signature unmodified." rho read the text and dropped this on the floor.
    #[serde(default)]
    pub signature: Option<String>,
    /// Reasoning the provider encrypted. It is opaque, and it replays as it arrived.
    #[serde(default)]
    pub redacted_content: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentBlockStop {
    pub content_block_index: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageStop {
    pub stop_reason: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub usage: BedrockUsage,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BedrockUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub cache_write_input_tokens: u64,
}

// --- The pure mapping. ---------------------------------------------------

/// The state the mapping carries between events. It buffers per-block data,
/// for example a partial tool-use input, until the block closes.
#[derive(Clone, Debug, Default)]
pub struct BedrockMapState {
    /// The tool name and buffered input JSON, keyed by content-block index.
    tool_inputs: HashMap<u32, String>,
    /// The indexes that already emitted a `TextStart`.
    text_started: HashSet<u32>,
    /// The indexes that already emitted a `ThinkingStart`.
    thinking_started: HashSet<u32>,
    /// The signature of each open reasoning block, keyed by index. Bedrock sends it in its
    /// own delta, after the text.
    signatures: HashMap<u32, String>,
    /// The encrypted reasoning of each open block, keyed by index.
    redacted: HashMap<u32, String>,
    /// The model of this request. A payload names its owner, and only the stream knows it.
    model: String,
}

impl BedrockMapState {
    /// Build a state for one model. A payload needs the model id to name its owner, and
    /// rule 8 drops a payload whose owner does not match the next request.
    pub fn for_model(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..Self::default()
        }
    }
}

/// Map one `ConverseStream` event to zero or more normalised events. See
/// `SPEC-provider-interface` section 5.
///
/// The function never panics on decoded input. A malformed tool-use buffer maps
/// to a JSON null, so a bad chunk cannot crash the caller.
pub fn map_converse_event(
    state: &mut BedrockMapState,
    event: ConverseStreamEvent,
) -> Vec<StreamEvent> {
    let mut out = Vec::new();

    if let Some(start) = event.message_start {
        out.push(StreamEvent::MessageStart {
            role: role_from_wire(&start.role),
        });
    }

    if let Some(block) = event.content_block_start
        && let Some(tool) = block.start.tool_use
    {
        state
            .tool_inputs
            .entry(block.content_block_index)
            .or_default();
        out.push(StreamEvent::ToolCallStart {
            index: block.content_block_index,
            id: tool.tool_use_id,
            name: tool.name,
        });
    }

    if let Some(block) = event.content_block_delta {
        let index = block.content_block_index;
        if let Some(text) = block.delta.text {
            if state.text_started.insert(index) {
                out.push(StreamEvent::TextStart { index });
            }
            out.push(StreamEvent::TextDelta { index, delta: text });
        }
        if let Some(tool) = block.delta.tool_use {
            state
                .tool_inputs
                .entry(index)
                .or_default()
                .push_str(&tool.input);
            out.push(StreamEvent::ToolCallDelta {
                index,
                delta: tool.input,
            });
        }
        if let Some(reasoning) = block.delta.reasoning_content {
            // A signature-only delta must not open a block on its own, and an empty text
            // must add nothing. See behaviour rules 1 and 2.
            if let Some(text) = reasoning.text.filter(|text| !text.is_empty()) {
                if state.thinking_started.insert(index) {
                    out.push(StreamEvent::ThinkingStart { index });
                }
                out.push(StreamEvent::ThinkingDelta { index, delta: text });
            }
            if let Some(signature) = reasoning.signature {
                state
                    .signatures
                    .entry(index)
                    .or_default()
                    .push_str(&signature);
            }
            if let Some(redacted) = reasoning.redacted_content {
                state.redacted.entry(index).or_default().push_str(&redacted);
            }
        }
    }

    if let Some(stop) = event.content_block_stop {
        let index = stop.content_block_index;
        if let Some(buffer) = state.tool_inputs.remove(&index) {
            // Parse only after concatenation. An empty buffer means no argument,
            // which is a valid empty object.
            let arguments = if buffer.trim().is_empty() {
                Value::Object(serde_json::Map::new())
            } else {
                serde_json::from_str(&buffer).unwrap_or(Value::Null)
            };
            out.push(StreamEvent::ToolCallEnd {
                index,
                arguments,
                // Bedrock binds no replay payload to a tool call. Gemini does.
                state: None,
            });
        } else if state.text_started.remove(&index) {
            out.push(StreamEvent::TextEnd { index });
        } else if state.thinking_started.remove(&index) {
            out.push(StreamEvent::ThinkingEnd {
                index,
                state: take_reasoning_state(state, index),
            });
        }
    }

    if let Some(stop) = event.message_stop {
        out.push(StreamEvent::Done {
            stop_reason: stop_reason_from_wire(&stop.stop_reason),
        });
    }

    if let Some(metadata) = event.metadata {
        out.push(StreamEvent::Usage(Usage {
            input_tokens: metadata.usage.input_tokens,
            output_tokens: metadata.usage.output_tokens,
            cache_read_tokens: metadata.usage.cache_read_input_tokens,
            cache_write_tokens: metadata.usage.cache_write_input_tokens,
            // Bedrock reports no charge on the stream, so the field stays empty rather
            // than guessing from a price table. See decision D-measured-cost-and-cache.
            cost_usd: None,
        }));
    }

    out
}

/// Map a Bedrock exception name to a provider error. See `SPEC-provider-interface` section 5.
///
/// The match ignores case, because the recorded fixtures use a lower-first name
/// such as `throttlingException`, but the SDK reports a upper-first name such as
/// `ThrottlingException`.
pub fn map_converse_error(exception_name: &str) -> ProviderError {
    let name = exception_name.to_ascii_lowercase();
    match name.as_str() {
        "throttlingexception" => ProviderError::RateLimited {
            retry_after_ms: None,
        },
        "serviceunavailableexception" => ProviderError::Server { status: 503 },
        "internalserverexception" | "modelstreamerrorexception" => {
            ProviderError::Server { status: 500 }
        }
        "validationexception" => ProviderError::Client {
            status: 400,
            message:
                "Bedrock rejected the request as invalid. Check the model id and the request shape."
                    .to_string(),
        },
        // An unknown exception is treated as a server fault. A retry may clear a
        // transient server-side problem, and this keeps a new exception name safe.
        _ => ProviderError::Server { status: 500 },
    }
}

/// Map the Bedrock role string to the normalised role.
fn role_from_wire(role: &str) -> Role {
    match role {
        "user" => Role::User,
        _ => Role::Assistant,
    }
}

/// Map the Bedrock stop reason string to the normalised stop reason.
fn stop_reason_from_wire(reason: &str) -> StopReason {
    match reason {
        "end_turn" => StopReason::EndTurn,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        "stop_sequence" => StopReason::StopSequence,
        "guardrail_intervened" | "content_filtered" => StopReason::ContentFiltered,
        _ => StopReason::EndTurn,
    }
}

/// Turn a recorded event list into a provider stream. The mapping runs in
/// order and the stream yields each event as it maps. This is the fake
/// transport for the shared contract.
///
/// The terminal `Done` is held back and yielded last. AWS sends the `metadata`
/// usage event after `messageStop`, so a plain in-order map would place `Usage`
/// after `Done`. `Done` is the end of a turn, so it must be the last event.
pub fn events_to_stream(events: Vec<ConverseStreamEvent>, cancel: CancelToken) -> ProviderStream {
    let mapped = stream! {
        let mut state = BedrockMapState::default();
        let mut deferred_done: Option<StreamEvent> = None;
        for event in events {
            if cancel.is_cancelled() {
                break;
            }
            for normalised in map_converse_event(&mut state, event) {
                match normalised {
                    done @ StreamEvent::Done { .. } => deferred_done = Some(done),
                    other => yield Ok(other),
                }
            }
        }
        if let Some(done) = deferred_done {
            yield Ok(done);
        }
    };
    Box::pin(mapped)
}

// --- The provider. -------------------------------------------------------

/// The Bedrock provider configuration. It holds no static credential. The AWS
/// credential chain resolves credentials at request time.
#[derive(Clone, Debug)]
pub struct BedrockConfig {
    /// The AWS region, for example `us-east-1`.
    pub region: String,
}

impl BedrockConfig {
    /// Build a configuration for a region.
    pub fn new(region: impl Into<String>) -> Self {
        Self {
            region: region.into(),
        }
    }
}

/// The AWS Bedrock provider.
#[derive(Clone, Debug)]
pub struct BedrockProvider {
    config: BedrockConfig,
}

impl BedrockProvider {
    /// Build the provider from a configuration.
    pub fn new(config: BedrockConfig) -> Self {
        Self { config }
    }

    /// The configured region.
    pub fn region(&self) -> &str {
        &self.config.region
    }
}

#[async_trait]
impl Provider for BedrockProvider {
    fn id(&self) -> &str {
        "bedrock"
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        cancel: CancelToken,
    ) -> Result<ProviderStream, ProviderError> {
        use aws_config::BehaviorVersion;
        use aws_sdk_bedrockruntime::Client;
        use aws_sdk_bedrockruntime::config::Region;

        // The standard credential chain signs with SigV4: environment, shared
        // profile, SSO cache, then IMDS. The provider never signs by hand.
        let sdk_config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(self.config.region.clone()))
            .load()
            .await;
        let client = Client::new(&sdk_config);

        let builder = apply_request(client.converse_stream(), &request);

        let output = builder
            .send()
            .await
            .map_err(|error| map_sdk_error(error.code(), error.to_string()))?;

        let mut receiver = output.stream;
        let model_id = request.model.clone();
        let stream = stream! {
            let mut state = BedrockMapState::for_model(model_id);
            let mut deferred_done: Option<StreamEvent> = None;
            loop {
                let next = tokio::select! {
                    biased;
                    () = cancel.cancelled() => break,
                    received = receiver.recv() => received,
                };
                match next {
                    Ok(Some(sdk_event)) => {
                        if let Some(mirror) = sdk_event_to_mirror(sdk_event) {
                            for normalised in map_converse_event(&mut state, mirror) {
                                match normalised {
                                    done @ StreamEvent::Done { .. } => {
                                        deferred_done = Some(done);
                                    }
                                    other => yield Ok(other),
                                }
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        let code = error.code().map(str::to_string);
                        let message = error.to_string();
                        yield Err(map_sdk_error(code.as_deref(), message));
                        return;
                    }
                }
            }
            if let Some(done) = deferred_done {
                yield Ok(done);
            }
        };
        Ok(Box::pin(stream))
    }
}

/// Map an SDK error code and message to a provider error.
fn map_sdk_error(code: Option<&str>, message: String) -> ProviderError {
    match code {
        Some(name) => map_converse_error(name),
        // No modelled code means a transport or credential fault. A signing
        // failure surfaces here. Report it as auth so the user checks the
        // credential chain, unless it reads like a plain network fault.
        None => ProviderError::Transport(message),
    }
}

/// Convert one SDK `ConverseStreamOutput` into the recorded mirror. The mirror
/// then flows through the same `map_converse_event` the tests exercise.
fn sdk_event_to_mirror(
    event: aws_sdk_bedrockruntime::types::ConverseStreamOutput,
) -> Option<ConverseStreamEvent> {
    use aws_sdk_bedrockruntime::types::{
        ContentBlockDelta as SdkDelta, ContentBlockStart as SdkStart, ConverseStreamOutput as Out,
        ReasoningContentBlockDelta as SdkReasoning,
    };

    let mut mirror = ConverseStreamEvent {
        message_start: None,
        content_block_start: None,
        content_block_delta: None,
        content_block_stop: None,
        message_stop: None,
        metadata: None,
    };

    match event {
        Out::MessageStart(start) => {
            mirror.message_start = Some(MessageStart {
                role: start.role.as_str().to_string(),
            });
        }
        Out::ContentBlockStart(start) => {
            let tool_use = match start.start {
                Some(SdkStart::ToolUse(tool)) => Some(ToolUseStart {
                    tool_use_id: tool.tool_use_id,
                    name: tool.name,
                }),
                _ => None,
            };
            mirror.content_block_start = Some(ContentBlockStart {
                content_block_index: start.content_block_index.max(0) as u32,
                start: BlockStart { tool_use },
            });
        }
        Out::ContentBlockDelta(delta) => {
            let block_delta = match delta.delta {
                Some(SdkDelta::Text(text)) => BlockDelta {
                    text: Some(text),
                    tool_use: None,
                    reasoning_content: None,
                },
                Some(SdkDelta::ToolUse(tool)) => BlockDelta {
                    text: None,
                    tool_use: Some(ToolUseDelta { input: tool.input }),
                    reasoning_content: None,
                },
                Some(SdkDelta::ReasoningContent(SdkReasoning::Text(text))) => BlockDelta {
                    text: None,
                    tool_use: None,
                    reasoning_content: Some(ReasoningDelta {
                        text: Some(text),
                        signature: None,
                        redacted_content: None,
                    }),
                },
                // The signature arrives in its own delta, after the text. This arm was a
                // wildcard, so the live path dropped every signature and rho had nothing to
                // replay. The unit tests could not see it, because they build the mirror
                // shape directly.
                Some(SdkDelta::ReasoningContent(SdkReasoning::Signature(signature))) => {
                    BlockDelta {
                        text: None,
                        tool_use: None,
                        reasoning_content: Some(ReasoningDelta {
                            text: None,
                            signature: Some(signature),
                            redacted_content: None,
                        }),
                    }
                }
                // Encrypted reasoning. It is opaque, and it replays as it arrived. The blob
                // is not valid UTF-8 in general, so it is carried as base64.
                Some(SdkDelta::ReasoningContent(SdkReasoning::RedactedContent(blob))) => {
                    use base64::Engine;
                    BlockDelta {
                        text: None,
                        tool_use: None,
                        reasoning_content: Some(ReasoningDelta {
                            text: None,
                            signature: None,
                            redacted_content: Some(
                                base64::engine::general_purpose::STANDARD.encode(blob.as_ref()),
                            ),
                        }),
                    }
                }
                _ => BlockDelta {
                    text: None,
                    tool_use: None,
                    reasoning_content: None,
                },
            };
            mirror.content_block_delta = Some(ContentBlockDelta {
                content_block_index: delta.content_block_index.max(0) as u32,
                delta: block_delta,
            });
        }
        Out::ContentBlockStop(stop) => {
            mirror.content_block_stop = Some(ContentBlockStop {
                content_block_index: stop.content_block_index.max(0) as u32,
            });
        }
        Out::MessageStop(stop) => {
            mirror.message_stop = Some(MessageStop {
                stop_reason: stop.stop_reason.as_str().to_string(),
            });
        }
        Out::Metadata(metadata) => {
            let usage = metadata.usage?;
            mirror.metadata = Some(Metadata {
                usage: BedrockUsage {
                    input_tokens: usage.input_tokens.max(0) as u64,
                    output_tokens: usage.output_tokens.max(0) as u64,
                    cache_read_input_tokens: usage.cache_read_input_tokens.unwrap_or(0).max(0)
                        as u64,
                    cache_write_input_tokens: usage.cache_write_input_tokens.unwrap_or(0).max(0)
                        as u64,
                },
            });
        }
        _ => return None,
    }
    Some(mirror)
}

/// Build the request messages for one model. The model decides whether a stored reasoning
/// payload may travel, per rule 8.
pub fn build_messages_for_model(
    messages: &[Message],
    model: &str,
) -> Vec<aws_sdk_bedrockruntime::types::Message> {
    // A review deleted the old `build_messages`, which passed an empty model. It made the
    // replay path look covered by tests that could never reach it, because `for_owner`
    // refuses an empty name. One function now, and every caller states its model.
    //
    // Only the current tool loop replays its reasoning. The prompt is append-only, so a
    // block re-sent on every later turn costs bytes for the whole session: twenty turns
    // re-upload turn one's trace nineteen times. Anthropic needs the thinking of the
    // assistant turns that carry the pending call, and nothing older. A performance review
    // measured the growth as O(turns squared). See `D-replay-only-the-current-loop`.
    let loop_start = messages
        .iter()
        .rposition(|message| message.role == Role::User)
        .unwrap_or(0);
    use aws_sdk_bedrockruntime::types::{
        ContentBlock as SdkBlock, ConversationRole, Message as SdkMessage, ToolResultBlock,
        ToolResultContentBlock, ToolUseBlock,
    };
    use rho_core::ContentBlock;

    // Collect the blocks per role, merging a run of messages that share a role.
    //
    // Converse requires strictly alternating roles. `rho-core` records one
    // `Role::Tool` message per tool result, and Bedrock has no tool role, so every
    // result maps to `user`. Two tool calls in one turn therefore produced two
    // consecutive user messages, and Bedrock answered 400.
    //
    // One tool call worked, which is why the unit tests and the first live check both
    // passed. A live run with two calls found it.
    //
    // Merging is also what Bedrock wants: all tool results for one turn belong in a
    // single user message.
    let mut grouped: Vec<(ConversationRole, Vec<SdkBlock>)> = Vec::new();
    for (position, message) in messages.iter().enumerate() {
        let role = match message.role {
            Role::User | Role::Tool => ConversationRole::User,
            Role::Assistant => ConversationRole::Assistant,
        };
        let mut blocks: Vec<SdkBlock> = Vec::new();
        for block in &message.content {
            match block {
                ContentBlock::Text { text } => {
                    blocks.push(SdkBlock::Text(text.clone()));
                }
                ContentBlock::ToolCall {
                    id,
                    name,
                    arguments,
                    // Bedrock binds no replay payload to a call. Gemini does, and that crate
                    // reads this field without any change to shared code.
                    state: _,
                } => {
                    if let Ok(tool_use) = ToolUseBlock::builder()
                        .tool_use_id(id.clone())
                        .name(name.clone())
                        .input(json_to_document(arguments))
                        .build()
                    {
                        blocks.push(SdkBlock::ToolUse(tool_use));
                    }
                }
                ContentBlock::ToolResult {
                    tool_call_id,
                    content,
                    is_error,
                } => {
                    let mut result = ToolResultBlock::builder()
                        .tool_use_id(tool_call_id.clone())
                        .status(if *is_error {
                            aws_sdk_bedrockruntime::types::ToolResultStatus::Error
                        } else {
                            aws_sdk_bedrockruntime::types::ToolResultStatus::Success
                        });
                    for inner in content {
                        if let ContentBlock::Text { text } = inner {
                            result = result.content(ToolResultContentBlock::Text(text.clone()));
                        }
                    }
                    if let Ok(result) = result.build() {
                        blocks.push(SdkBlock::ToolResult(result));
                    }
                }
                // A trace is for the reader, and it never reaches a provider. That is the
                // whole point of the split, and the type enforces it here.
                ContentBlock::ReasoningTrace { .. } => {}
                // A replay block travels only when the payload is ours and the model still
                // matches. Otherwise it is dropped in this named arm, never by `_ => {}`.
                ContentBlock::ReasoningReplay { text, state } => {
                    // Out of the current loop, so it is history. It is dropped whole, and it
                    // never becomes prose, because prose would read as an answer.
                    if position >= loop_start
                        && let Some(reasoning) = replay_block(text, state, model)
                    {
                        blocks.push(SdkBlock::ReasoningContent(reasoning));
                    }
                }
                // Image input in a request is out of scope for sprint 1. Drop it in a named
                // arm, so a new block kind cannot hide behind a wildcard.
                ContentBlock::Image { .. } => {}
            }
        }
        if blocks.is_empty() {
            continue;
        }
        match grouped.last_mut() {
            Some((last_role, last_blocks)) if *last_role == role => {
                last_blocks.extend(blocks);
            }
            _ => grouped.push((role, blocks)),
        }
    }

    let mut out = Vec::new();
    for (role, blocks) in grouped {
        let mut builder = SdkMessage::builder().role(role);
        for block in blocks {
            builder = builder.content(block);
        }
        if let Ok(message) = builder.build() {
            out.push(message);
        }
    }
    out
}

/// Build the inference configuration when the request sets any limit.
/// Put one request onto a `converse_stream` builder.
///
/// Every field the wire needs is set here, and `stream` calls nothing else. So a test can
/// build the same request offline and read it back with `as_input`, instead of grepping this
/// file for a call. A source guard cannot tell a live call from a comment, and two reviews
/// found exactly that hole.
fn apply_request(
    builder: aws_sdk_bedrockruntime::operation::converse_stream::builders::ConverseStreamFluentBuilder,
    request: &CompletionRequest,
) -> aws_sdk_bedrockruntime::operation::converse_stream::builders::ConverseStreamFluentBuilder {
    let mut builder =
        builder
            .model_id(request.model.clone())
            .set_messages(Some(build_messages_for_model(
                &request.messages,
                &request.model,
            )));
    if let Some(system) = &request.system {
        builder = builder.system(aws_sdk_bedrockruntime::types::SystemContentBlock::Text(
            system.clone(),
        ));
    }
    if let Some(config) = build_inference_config(request) {
        builder = builder.inference_config(config);
    }
    if let Some(fields) = build_thinking_fields(request) {
        builder = builder.additional_model_request_fields(fields);
    }
    if let Some(tools) = build_tool_config(request) {
        builder = builder.tool_config(tools);
    }
    builder
}

/// The provider id every payload carries. It is `Provider::id` for this crate.
const PROVIDER_ID: &str = "bedrock";

/// The payload for one finished reasoning block, or `None` when the model signed nothing.
///
/// An unsigned block is not replayable. Bedrock rejects a reasoning block without its
/// signature, so rho keeps the text as history instead of sending a block it knows is bad.
fn take_reasoning_state(
    state: &mut BedrockMapState,
    index: u32,
) -> Option<rho_core::ProviderState> {
    let signature = state.signatures.remove(&index);
    let redacted = state.redacted.remove(&index);
    if signature.is_none() && redacted.is_none() {
        return None;
    }
    // A payload with no model has no owner, and an unowned payload can never be replayed.
    // Minting one would write a dead payload into a session file, so it is refused here as
    // well as in `for_owner`. `events_to_stream` is the caller that has no model.
    if state.model.is_empty() {
        tracing::warn!("a reasoning payload arrived with no model, so it carries no owner");
        return None;
    }
    let mut value = serde_json::Map::new();
    if let Some(signature) = signature {
        value.insert("signature".to_string(), Value::String(signature));
    }
    if let Some(redacted) = redacted {
        value.insert("redacted".to_string(), Value::String(redacted));
    }
    Some(rho_core::ProviderState {
        owner: rho_core::ReasoningOwner {
            provider: PROVIDER_ID.to_string(),
            model: state.model.clone(),
        },
        value: Value::Object(value),
    })
}

/// Turn one replay payload into a Bedrock reasoning block, when it is ours to replay.
///
/// Rule 8 runs first, through `ProviderState::for_owner`. A payload from another provider or
/// another model is dropped, because a signature is bound to the model that made it.
fn replay_block(
    text: &str,
    state: &Option<rho_core::ProviderState>,
    model: &str,
) -> Option<aws_sdk_bedrockruntime::types::ReasoningContentBlock> {
    use aws_sdk_bedrockruntime::types::{ReasoningContentBlock, ReasoningTextBlock};

    let state = state.as_ref()?;
    let value = match state.for_owner(PROVIDER_ID, model) {
        Some(value) => value,
        None => {
            // Rule 8 says a drop is never silent. A review found that every one of these
            // paths returned `None` with nothing said, which is the same silence the rule
            // forbids. The report names the owner and the reason, and never the payload.
            // Every one of these strings can come from a session file, which is untrusted
            // input. A security review found that a crafted `owner.model` could carry
            // terminal escapes or a forged newline straight into a log. `rho-redact` is the
            // one home for that, per `D-one-redaction-home`.
            tracing::warn!(
                owner_provider = %rho_redact::sanitize_line(&state.owner.provider),
                owner_model = %rho_redact::sanitize_line(&state.owner.model),
                request_model = %rho_redact::sanitize_line(model),
                "a reasoning payload belongs to another owner, so it was not replayed"
            );
            return None;
        }
    };
    if let Some(redacted) = value.get("redacted").and_then(Value::as_str) {
        use base64::Engine;
        // The payload holds base64, because a blob is not valid UTF-8 in general. A failed
        // decode sends nothing: a wrong blob is worse than a missing one, because Bedrock
        // would reject the whole turn.
        match base64::engine::general_purpose::STANDARD.decode(redacted) {
            Ok(bytes) => {
                return Some(ReasoningContentBlock::RedactedContent(
                    aws_smithy_types::Blob::new(bytes),
                ));
            }
            Err(_) => {
                tracing::warn!(
                    "an encrypted reasoning payload did not decode, so it was not replayed"
                );
                return None;
            }
        }
    }
    let Some(signature) = value.get("signature").and_then(Value::as_str) else {
        tracing::warn!("a reasoning payload carries no signature, so it was not replayed");
        return None;
    };
    match ReasoningTextBlock::builder()
        .text(text)
        .signature(signature)
        .build()
    {
        Ok(block) => Some(ReasoningContentBlock::ReasoningText(block)),
        Err(error) => {
            tracing::warn!(%error, "a reasoning block did not build, so it was not replayed");
            None
        }
    }
}

/// Report an unreadable model id once per model, not once per turn.
///
/// A review found the spam: the comment said "once" and the code warned on every request. A
/// warning a user learns to scroll past is a warning that no longer works.
fn report_once(model: &str) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};

    static REPORTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let reported = REPORTED.get_or_init(|| Mutex::new(HashSet::new()));
    let first = match reported.lock() {
        Ok(mut set) => set.insert(model.to_string()),
        // A poisoned lock must not silence a report, so it reports again instead.
        Err(_) => true,
    };
    if first {
        tracing::warn!(
            model = %rho_redact::sanitize_line(model),
            "this model is not known to support extended thinking, so rho asked for none"
        );
    }
}

/// One model family that supports extended thinking, and the version it starts at.
///
/// A review asked for a table instead of a parse, and it was right for one reason: a reader
/// can check a table. The parse stays, because a Bedrock id really does carry its version in
/// its name, but the families and their thresholds are now data, and each row is tested.
///
/// A family absent from this table is asked for nothing. That is fail-closed, and
/// `build_thinking_fields` reports it, because a field the endpoint does not know is a 400
/// for the whole turn.
struct ThinkingFamily {
    /// The substring that names the family inside a Bedrock model id.
    marker: &'static str,
    /// The lowest version that supports extended thinking, as (major, minor).
    since: (u32, u32),
}

/// The families rho knows. Anthropic added extended thinking in Claude 3.7.
const THINKING_FAMILIES: &[ThinkingFamily] = &[ThinkingFamily {
    marker: "anthropic.claude",
    since: (3, 7),
}];

/// Does this model id support extended thinking?
///
/// It fails closed. An id rho cannot read answers `false`, and so does a family it does not
/// know. A known limit: an application inference profile ARN hides the model behind an opaque
/// id, so a capable model reads as incapable. `build_thinking_fields` reports that rather
/// than dropping the request in silence, because the user asked for a level.
fn model_supports_thinking(model: &str) -> bool {
    let id = model.to_ascii_lowercase();
    THINKING_FAMILIES.iter().any(|family| {
        // A Bedrock id may carry a region prefix, as in `us.anthropic.claude-...`. It may
        // also carry no vendor prefix at all: `claude-3-7-sonnet-20250219-v1:0` is a real id
        // that a review found reading as incapable. So the family name matches on its last
        // segment too.
        let short = family.marker.rsplit('.').next().unwrap_or(family.marker);
        let Some(after) = id
            .split(family.marker)
            .nth(1)
            .or_else(|| id.split(short).nth(1))
        else {
            return false;
        };
        // The first two numbers after the family name are the version. `claude-3-5-sonnet`
        // gives 3 and 5, and `claude-haiku-4-5-2025...` gives 4 and 5.
        //
        // A version part is always under 100. A release date is not, and
        // `anthropic.claude-3-haiku-20240307` read its date as minor version 20240307, which
        // made a model without thinking claim it. A test found that, so the bound stays.
        // A version part is short. A date part is not, and a review found a second date
        // shape that slipped through: `claude-3-opus-2025-07-15` offered `07` as a minor
        // version, which read as 3.7 and would have earned a 400. So a group of three or
        // more digits ends the version, and nothing after it counts.
        let mut version = after
            .split(|c: char| !c.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .take_while(|part| part.len() <= 2)
            .filter_map(|part| part.parse::<u32>().ok());
        let Some(major) = version.next() else {
            return false;
        };
        let minor = version.next().unwrap_or(0);
        (major, minor) >= family.since
    })
}

/// Build the `additionalModelRequestFields` that ask Claude for extended thinking.
///
/// `None` means rho asks for nothing: no effort, `Off`, or a model that cannot think.
fn build_thinking_fields(request: &CompletionRequest) -> Option<Document> {
    let budget = request.reasoning?.budget_tokens()?;
    if !model_supports_thinking(&request.model) {
        // The user asked for thinking and will not get it, so rho says so once. A review
        // found the case that makes this necessary: an application inference profile ARN
        // hides the model behind an opaque id, so a thinking-capable model reads as
        // incapable. The answer stays fail-closed, because an unknown field is a 400 for
        // the whole turn, but it is no longer silent.
        report_once(&request.model);
        return None;
    }
    let thinking = HashMap::from([
        ("type".to_string(), Document::String("enabled".to_string())),
        (
            "budget_tokens".to_string(),
            Document::Number(Number::PosInt(u64::from(budget))),
        ),
    ]);
    Some(Document::Object(HashMap::from([(
        "thinking".to_string(),
        Document::Object(thinking),
    )])))
}

/// The head room rho leaves for the answer above a thinking budget.
///
/// Anthropic rejects a request whose `max_tokens` is not above `budget_tokens`, and rho
/// sends no `max_tokens` of its own, so the SDK default would sit below the budget.
const ANSWER_HEAD_ROOM: u32 = 4096;

fn build_inference_config(
    request: &CompletionRequest,
) -> Option<aws_sdk_bedrockruntime::types::InferenceConfiguration> {
    // A thinking budget changes both numbers, so it is read first. See section 9 rules 2
    // and 3: the budget needs room above it, and Anthropic refuses a stated temperature
    // while it thinks.
    let budget = match build_thinking_fields(request) {
        Some(_) => request.reasoning.and_then(|effort| effort.budget_tokens()),
        None => None,
    };
    if budget.is_none() && request.max_tokens.is_none() && request.temperature.is_none() {
        return None;
    }
    let mut builder = aws_sdk_bedrockruntime::types::InferenceConfiguration::builder();
    match (budget, request.max_tokens) {
        // Rule 2. Keep the caller's number only when it leaves room for an answer as well.
        //
        // A review found the hole: `max_tokens = budget + 1` clears Anthropic's check and
        // leaves one token for the answer, so rho honoured a number that starves the reply
        // exactly when thinking is on.
        (Some(budget), Some(max_tokens)) if max_tokens > budget + ANSWER_HEAD_ROOM => {
            builder = builder.max_tokens(max_tokens as i32);
        }
        (Some(budget), _) => {
            builder = builder.max_tokens((budget + ANSWER_HEAD_ROOM) as i32);
        }
        (None, Some(max_tokens)) => {
            builder = builder.max_tokens(max_tokens as i32);
        }
        (None, None) => {}
    }
    // Rule 3. A temperature travels only when rho did not ask for thinking.
    if let Some(temperature) = request.temperature
        && budget.is_none()
    {
        builder = builder.temperature(temperature);
    }
    Some(builder.build())
}

/// Build the tool configuration from the advertised tools.
fn build_tool_config(
    request: &CompletionRequest,
) -> Option<aws_sdk_bedrockruntime::types::ToolConfiguration> {
    use aws_sdk_bedrockruntime::types::{
        Tool, ToolConfiguration, ToolInputSchema, ToolSpecification,
    };

    if request.tools.is_empty() {
        return None;
    }
    let mut builder = ToolConfiguration::builder();
    for spec in &request.tools {
        let Ok(specification) = ToolSpecification::builder()
            .name(spec.name.clone())
            .description(spec.description.clone())
            .input_schema(ToolInputSchema::Json(json_to_document(&spec.input_schema)))
            .build()
        else {
            continue;
        };
        builder = builder.tools(Tool::ToolSpec(specification));
    }
    builder.build().ok()
}

/// Convert a JSON value into the AWS `Document` type the SDK uses.
fn json_to_document(value: &Value) -> Document {
    match value {
        Value::Null => Document::Null,
        Value::Bool(flag) => Document::Bool(*flag),
        Value::Number(number) => {
            if let Some(unsigned) = number.as_u64() {
                Document::Number(Number::PosInt(unsigned))
            } else if let Some(signed) = number.as_i64() {
                Document::Number(Number::NegInt(signed))
            } else {
                Document::Number(Number::Float(number.as_f64().unwrap_or(0.0)))
            }
        }
        Value::String(text) => Document::String(text.clone()),
        Value::Array(items) => Document::Array(items.iter().map(json_to_document).collect()),
        Value::Object(map) => Document::Object(
            map.iter()
                .map(|(key, inner)| (key.clone(), json_to_document(inner)))
                .collect(),
        ),
    }
}

/// One source line with any comment removed, including a trailing one.
///
/// A trailing comment is the hole a review left open in the first version: the break
/// `let _ = fields; // builder.additional_model_request_fields(fields)` deleted the call and
/// kept the words, and a guard that drops only whole comment lines passed it. A `://` inside
/// a URL is left alone.
#[cfg(test)]
fn code_only(line: &str) -> &str {
    let mut search = 0;
    while let Some(found) = line[search..].find("//") {
        let at = search + found;
        if at > 0 && line.as_bytes()[at - 1] == b':' {
            search = at + 2;
            continue;
        }
        return &line[..at];
    }
    line
}

/// The production half of a source file, with every comment removed.
///
/// A source guard needs this. Prose that quotes the call it guards would otherwise satisfy
/// the guard after the call itself was deleted.
#[cfg(test)]
fn production_code(source: &str) -> String {
    source
        .split("#[cfg(test)]")
        .next()
        .expect("a source file has a first part")
        .lines()
        .map(code_only)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod thinking_request_tests {
    use super::*;
    use rho_core::{ContentBlock, Message, ReasoningEffort, Role};

    fn request(model: &str, effort: Option<ReasoningEffort>) -> CompletionRequest {
        CompletionRequest {
            model: model.to_string(),
            system: None,
            messages: vec![Message {
                role: Role::User,
                content: vec![ContentBlock::Text {
                    text: "hello".to_string(),
                }],
            }],
            tools: Vec::new(),
            max_tokens: None,
            temperature: None,
            reasoning: effort,
        }
    }

    /// The budget inside `additionalModelRequestFields`, or `None` when rho asked nothing.
    fn asked_budget(request: &CompletionRequest) -> Option<i64> {
        let fields = build_thinking_fields(request)?;
        let Document::Object(root) = fields else {
            panic!("the thinking fields are a JSON object");
        };
        let Some(Document::Object(thinking)) = root.get("thinking") else {
            panic!("the fields carry a thinking object");
        };
        assert_eq!(
            thinking.get("type"),
            Some(&Document::String("enabled".to_string())),
            "the thinking request is enabled"
        );
        match thinking.get("budget_tokens") {
            Some(Document::Number(Number::PosInt(budget))) => Some(*budget as i64),
            other => panic!("the budget is a positive integer, not {other:?}"),
        }
    }

    #[test]
    fn a_thinking_model_gets_the_thinking_request() {
        // Defect 1 of section 0: without the ask, Claude writes `<thinking>` into text.
        let request = request(
            "anthropic.claude-haiku-4-5-20251001-v1:0",
            Some(ReasoningEffort::Medium),
        );
        assert_eq!(
            asked_budget(&request),
            ReasoningEffort::Medium.budget_tokens().map(i64::from)
        );
    }

    #[test]
    fn an_unsupported_model_asks_for_nothing() {
        // Rule 1 of section 9, against real Bedrock ids. A field the endpoint does not
        // know is a 400 for the whole turn, so this fails closed.
        for model in [
            "amazon.titan-text-express-v1",
            "anthropic.claude-3-5-sonnet-20240620-v1:0",
            "anthropic.claude-3-haiku-20240307-v1:0",
            "meta.llama3-70b-instruct-v1:0",
            "",
        ] {
            let request = request(model, Some(ReasoningEffort::High));
            assert!(
                build_thinking_fields(&request).is_none(),
                "{model} must ask for nothing"
            );
        }
    }

    #[test]
    fn every_thinking_model_family_is_recognised() {
        for model in [
            "anthropic.claude-3-7-sonnet-20250219-v1:0",
            "anthropic.claude-sonnet-4-20250514-v1:0",
            "anthropic.claude-haiku-4-5-20251001-v1:0",
            "us.anthropic.claude-opus-4-1-20250805-v1:0",
        ] {
            let request = request(model, Some(ReasoningEffort::Low));
            assert!(
                build_thinking_fields(&request).is_some(),
                "{model} supports extended thinking"
            );
        }
    }

    #[test]
    fn an_absent_effort_asks_for_nothing() {
        let request = request("anthropic.claude-haiku-4-5-20251001-v1:0", None);
        assert!(build_thinking_fields(&request).is_none());
    }

    #[test]
    fn an_off_effort_asks_for_nothing() {
        let request = request(
            "anthropic.claude-haiku-4-5-20251001-v1:0",
            Some(ReasoningEffort::Off),
        );
        assert!(build_thinking_fields(&request).is_none());
    }

    #[test]
    fn the_budget_leaves_room_for_the_answer() {
        // Rule 2 of section 9. Anthropic rejects a request whose max_tokens is not above
        // the budget, and rho sends no max_tokens of its own today.
        let request = request(
            "anthropic.claude-haiku-4-5-20251001-v1:0",
            Some(ReasoningEffort::XHigh),
        );
        let config = build_inference_config(&request).expect("thinking sets an inference config");
        let budget = ReasoningEffort::XHigh.budget_tokens().unwrap() as i32;
        assert!(
            config.max_tokens().expect("max tokens is set") > budget,
            "max_tokens must clear the budget of {budget}"
        );
    }

    #[test]
    fn thinking_drops_a_temperature() {
        // Rule 3 of section 9. Anthropic allows only the default temperature with
        // extended thinking, so a stated temperature must not travel.
        let mut request = request(
            "anthropic.claude-haiku-4-5-20251001-v1:0",
            Some(ReasoningEffort::Low),
        );
        request.temperature = Some(0.2);
        let config = build_inference_config(&request).expect("an inference config exists");
        assert_eq!(config.temperature(), None, "no temperature with thinking");
    }

    /// A source guard, because the SDK builder needs live AWS to observe. `build_thinking_fields`
    /// can be perfect and still never reach a request, which is the wiring defect this
    /// branch has already shipped twice.
    #[test]
    fn the_thinking_fields_reach_the_request() {
        // Search the production half, with comments removed. Two reviews shaped this: the
        // first version searched the whole file and matched its own assertion string, and
        // the second pointed out that the literal surviving in a comment would pass after
        // the real call was deleted.
        let source = include_str!("lib.rs");
        let code = production_code(source);
        assert!(
            code.contains("builder.additional_model_request_fields(fields)"),
            "the converse builder must send the thinking fields"
        );
    }

    /// A caller's `max_tokens` is honoured only when it also leaves room for an answer.
    ///
    /// A review found that `budget + 1` cleared Anthropic's check and starved the reply.
    #[test]
    fn a_starving_max_tokens_is_raised() {
        let mut request = request(
            "anthropic.claude-haiku-4-5-20251001-v1:0",
            Some(ReasoningEffort::Medium),
        );
        let budget = ReasoningEffort::Medium.budget_tokens().unwrap();
        request.max_tokens = Some(budget + 1);
        let config = build_inference_config(&request).expect("a config exists");
        assert!(
            config.max_tokens().expect("max tokens") > (budget + ANSWER_HEAD_ROOM) as i32 - 1,
            "a number that clears the budget but starves the answer is raised"
        );
    }

    /// The boundary Anthropic actually rejects: `max_tokens` equal to the budget.
    ///
    /// A mutation review found that `>` could become `>=` with no test failing, and the API
    /// requires strictly more than the budget. This is the case that would have shipped a 400.
    #[test]
    fn a_max_tokens_equal_to_the_budget_is_raised() {
        let mut request = request(
            "anthropic.claude-haiku-4-5-20251001-v1:0",
            Some(ReasoningEffort::XHigh),
        );
        let budget = ReasoningEffort::XHigh.budget_tokens().unwrap();
        request.max_tokens = Some(budget);
        let config = build_inference_config(&request).expect("a config exists");
        assert!(
            config.max_tokens().expect("max tokens") > budget as i32,
            "max_tokens must be strictly above the budget"
        );
    }

    /// A generous caller keeps its own number.
    #[test]
    fn a_generous_max_tokens_is_kept() {
        let mut request = request(
            "anthropic.claude-haiku-4-5-20251001-v1:0",
            Some(ReasoningEffort::Low),
        );
        request.max_tokens = Some(90_000);
        let config = build_inference_config(&request).expect("a config exists");
        assert_eq!(config.max_tokens(), Some(90_000));
    }

    #[test]
    fn no_thinking_keeps_the_temperature() {
        let mut request = request("anthropic.claude-haiku-4-5-20251001-v1:0", None);
        request.temperature = Some(0.2);
        let config = build_inference_config(&request).expect("an inference config exists");
        assert_eq!(config.temperature(), Some(0.2));
    }
}

#[cfg(test)]
mod replay_tests {
    use super::*;
    use aws_sdk_bedrockruntime::types::ContentBlock as SdkBlock;
    use rho_core::{ContentBlock, Message, ProviderState, ReasoningOwner, Role};

    const MODEL: &str = "us.anthropic.claude-haiku-4-5-20251001-v1:0";

    /// One reasoning delta event in the wire mirror.
    fn reasoning_delta_event(
        index: u32,
        text: Option<&str>,
        signature: Option<&str>,
    ) -> ConverseStreamEvent {
        ConverseStreamEvent {
            content_block_delta: Some(ContentBlockDelta {
                content_block_index: index,
                delta: BlockDelta {
                    text: None,
                    tool_use: None,
                    reasoning_content: Some(ReasoningDelta {
                        text: text.map(str::to_string),
                        signature: signature.map(str::to_string),
                        redacted_content: None,
                    }),
                },
            }),
            ..Default::default()
        }
    }

    fn owned_state(provider: &str, model: &str, signature: &str) -> ProviderState {
        ProviderState {
            owner: ReasoningOwner {
                provider: provider.to_string(),
                model: model.to_string(),
            },
            value: serde_json::json!({ "signature": signature }),
        }
    }

    fn assistant(block: ContentBlock) -> Vec<Message> {
        vec![Message {
            role: Role::Assistant,
            content: vec![block],
        }]
    }

    /// The reasoning blocks Bedrock received, as (text, signature) pairs.
    fn sent_reasoning(messages: &[Message], model: &str) -> Vec<(String, String)> {
        build_messages_for_model(messages, model)
            .iter()
            .flat_map(|message| message.content().iter())
            .filter_map(|block| match block {
                SdkBlock::ReasoningContent(
                    aws_sdk_bedrockruntime::types::ReasoningContentBlock::ReasoningText(text),
                ) => Some((
                    text.text().to_string(),
                    text.signature().unwrap_or_default().to_string(),
                )),
                _ => None,
            })
            .collect()
    }

    /// A signature must come back inside a tool loop, and the AWS SDK says so: "If you pass
    /// a reasoning block back to the API in a multi-turn conversation, include the text and
    /// its signature unmodified."
    #[test]
    fn a_state_replays_for_the_same_owner() {
        let messages = assistant(ContentBlock::ReasoningReplay {
            text: "a plan".to_string(),
            state: Some(owned_state("bedrock", MODEL, "sig-1")),
        });
        assert_eq!(
            sent_reasoning(&messages, MODEL),
            vec![("a plan".to_string(), "sig-1".to_string())]
        );
    }

    /// Rule 8. Another model's payload is dropped, because a signature is bound to the
    /// model that made it and Bedrock rejects a foreign one.
    #[test]
    fn a_state_is_dropped_for_another_model() {
        let messages = assistant(ContentBlock::ReasoningReplay {
            text: "a plan".to_string(),
            state: Some(owned_state("bedrock", "another-model", "sig-1")),
        });
        assert!(sent_reasoning(&messages, MODEL).is_empty());
    }

    /// Rule 8, the other half. fx checks neither, and its own code cannot tell one
    /// provider's payload from another's.
    #[test]
    fn a_state_is_dropped_for_another_provider() {
        let messages = assistant(ContentBlock::ReasoningReplay {
            text: "a plan".to_string(),
            state: Some(owned_state("openrouter", MODEL, "sig-1")),
        });
        assert!(sent_reasoning(&messages, MODEL).is_empty());
    }

    /// A trace never travels. That is the whole reason for the split.
    #[test]
    fn a_trace_never_reaches_a_provider() {
        let messages = assistant(ContentBlock::ReasoningTrace {
            text: "a plan".to_string(),
        });
        assert!(sent_reasoning(&messages, MODEL).is_empty());
        // And it does not arrive as text either, which would look like an answer.
        let sent = build_messages_for_model(&messages, MODEL);
        assert!(sent.is_empty(), "a trace-only message carries nothing");
    }

    /// A replay block with no payload has nothing to send.
    #[test]
    fn an_absent_state_replays_nothing() {
        let messages = assistant(ContentBlock::ReasoningReplay {
            text: "a plan".to_string(),
            state: None,
        });
        assert!(sent_reasoning(&messages, MODEL).is_empty());
    }

    /// A payload with no signature is not a signed block, so it is dropped rather than
    /// sent with an empty signature, which Bedrock would reject.
    #[test]
    fn a_state_with_no_signature_is_dropped() {
        let mut state = owned_state("bedrock", MODEL, "sig-1");
        state.value = serde_json::json!({ "unrelated": true });
        let messages = assistant(ContentBlock::ReasoningReplay {
            text: "a plan".to_string(),
            state: Some(state),
        });
        assert!(sent_reasoning(&messages, MODEL).is_empty());
    }

    /// The stream must capture the signature, or there is nothing to replay. rho parsed the
    /// reasoning text and dropped the signature field on the floor.
    #[test]
    fn the_stream_captures_the_signature() {
        let mut state = BedrockMapState::for_model(MODEL);
        let events = vec![
            reasoning_delta_event(0, Some("a plan"), None),
            reasoning_delta_event(0, None, Some("sig-1")),
            ConverseStreamEvent {
                content_block_stop: Some(ContentBlockStop {
                    content_block_index: 0,
                }),
                ..Default::default()
            },
        ];
        let mut out = Vec::new();
        for event in events {
            out.extend(map_converse_event(&mut state, event));
        }
        let end = out
            .iter()
            .find_map(|event| match event {
                StreamEvent::ThinkingEnd { state, .. } => Some(state.clone()),
                _ => None,
            })
            .expect("the stream ends the thinking block");
        let state = end.expect("the payload carries the signature");
        assert_eq!(state.owner.provider, "bedrock");
        assert_eq!(state.owner.model, MODEL);
        assert_eq!(state.value["signature"], "sig-1");
    }

    /// The live SDK translation must carry a signature into the mirror.
    ///
    /// Every unit test above builds the mirror shape directly, so all of them passed while
    /// the real path dropped the signature in a wildcard arm. This test drives the SDK
    /// types, which is the only way to see that.
    #[test]
    fn the_sdk_translation_carries_a_signature() {
        use aws_sdk_bedrockruntime::types::{
            ContentBlockDelta as SdkDelta, ContentBlockDeltaEvent, ConverseStreamOutput as Out,
            ReasoningContentBlockDelta as SdkReasoning,
        };
        let event = Out::ContentBlockDelta(
            ContentBlockDeltaEvent::builder()
                .content_block_index(0)
                .delta(SdkDelta::ReasoningContent(SdkReasoning::Signature(
                    "sig-live".to_string(),
                )))
                .build()
                .expect("the delta builds"),
        );
        let mirror = sdk_event_to_mirror(event).expect("the event maps");
        let reasoning = mirror
            .content_block_delta
            .expect("a delta arrived")
            .delta
            .reasoning_content
            .expect("the reasoning survives the translation");
        assert_eq!(reasoning.signature.as_deref(), Some("sig-live"));
        assert_eq!(reasoning.text, None, "a signature delta carries no text");
    }

    /// The same for encrypted reasoning, which arrives as a blob and rides as base64.
    #[test]
    fn the_sdk_translation_carries_redacted_reasoning() {
        use aws_sdk_bedrockruntime::types::{
            ContentBlockDelta as SdkDelta, ContentBlockDeltaEvent, ConverseStreamOutput as Out,
            ReasoningContentBlockDelta as SdkReasoning,
        };
        let event = Out::ContentBlockDelta(
            ContentBlockDeltaEvent::builder()
                .content_block_index(0)
                .delta(SdkDelta::ReasoningContent(SdkReasoning::RedactedContent(
                    aws_smithy_types::Blob::new(vec![0xff, 0x00, 0x10]),
                )))
                .build()
                .expect("the delta builds"),
        );
        let mirror = sdk_event_to_mirror(event).expect("the event maps");
        let reasoning = mirror
            .content_block_delta
            .expect("a delta arrived")
            .delta
            .reasoning_content
            .expect("the reasoning survives the translation");
        let encoded = reasoning
            .redacted_content
            .expect("the blob survives as base64");
        use base64::Engine;
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .expect("valid base64"),
            vec![0xff, 0x00, 0x10],
            "a blob is not valid utf8, so it must ride as base64"
        );
    }

    /// A turn with no signature yields no payload, so the reducer keeps a trace.
    #[test]
    fn a_stream_with_no_signature_yields_no_state() {
        let mut state = BedrockMapState::for_model(MODEL);
        let events = vec![
            reasoning_delta_event(0, Some("a plan"), None),
            ConverseStreamEvent {
                content_block_stop: Some(ContentBlockStop {
                    content_block_index: 0,
                }),
                ..Default::default()
            },
        ];
        let mut out = Vec::new();
        for event in events {
            out.extend(map_converse_event(&mut state, event));
        }
        assert!(
            out.iter()
                .any(|event| matches!(event, StreamEvent::ThinkingEnd { state: None, .. }))
        );
    }
}

#[cfg(test)]
mod drop_report_tests {
    use super::*;
    use rho_core::{ContentBlock, Message, ProviderState, ReasoningOwner, Role};
    use std::sync::{Arc, Mutex};

    const MODEL: &str = "us.anthropic.claude-haiku-4-5-20251001-v1:0";

    #[derive(Clone)]
    struct BufferWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for BufferWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufferWriter {
        type Writer = BufferWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Build messages under a log capture, and return what was logged.
    fn logged_while_building(block: ContentBlock, model: &str) -> String {
        let buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(BufferWriter(Arc::clone(&buffer)))
            .with_max_level(tracing::Level::TRACE)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            build_messages_for_model(
                &[Message {
                    role: Role::Assistant,
                    content: vec![block],
                }],
                model,
            );
            // Prove the capture works before trusting what it holds.
            tracing::warn!("the capture is live");
        });
        String::from_utf8(buffer.lock().unwrap().clone()).expect("valid utf8")
    }

    fn state(provider: &str, model: &str, value: serde_json::Value) -> Option<ProviderState> {
        Some(ProviderState {
            owner: ReasoningOwner {
                provider: provider.to_string(),
                model: model.to_string(),
            },
            value,
        })
    }

    /// Rule 8: the drop is never silent. Every one of these paths said nothing before a
    /// second review found them.
    #[test]
    fn a_dropped_payload_is_reported() {
        let cases: Vec<(&str, ContentBlock, &str)> = vec![
            (
                "another model",
                ContentBlock::ReasoningReplay {
                    text: "plan".to_string(),
                    state: state(
                        "bedrock",
                        "other-model",
                        serde_json::json!({"signature": "s"}),
                    ),
                },
                "another owner",
            ),
            (
                "another provider",
                ContentBlock::ReasoningReplay {
                    text: "plan".to_string(),
                    state: state("openrouter", MODEL, serde_json::json!({"signature": "s"})),
                },
                "another owner",
            ),
            (
                "no signature",
                ContentBlock::ReasoningReplay {
                    text: "plan".to_string(),
                    state: state("bedrock", MODEL, serde_json::json!({"unrelated": true})),
                },
                "no signature",
            ),
            (
                "bad base64",
                ContentBlock::ReasoningReplay {
                    text: "plan".to_string(),
                    state: state(
                        "bedrock",
                        MODEL,
                        serde_json::json!({"redacted": "!!not base64!!"}),
                    ),
                },
                "did not decode",
            ),
        ];
        for (name, block, needle) in cases {
            let logged = logged_while_building(block, MODEL);
            assert!(
                logged.contains("the capture is live"),
                "{name}: capture works"
            );
            assert!(
                logged.contains(needle),
                "{name}: the drop must be reported, and say why: {logged}"
            );
        }
    }

    /// A payload that replays says nothing, because there is nothing to report.
    #[test]
    fn a_replayed_payload_is_not_reported_as_a_drop() {
        let logged = logged_while_building(
            ContentBlock::ReasoningReplay {
                text: "plan".to_string(),
                state: state("bedrock", MODEL, serde_json::json!({"signature": "s"})),
            },
            MODEL,
        );
        assert!(
            !logged.contains("not replayed"),
            "a good payload is quiet: {logged}"
        );
    }

    /// The report never carries the payload, per rule 9.
    #[test]
    fn a_drop_report_never_names_the_payload() {
        let logged = logged_while_building(
            ContentBlock::ReasoningReplay {
                text: "plan".to_string(),
                state: state(
                    "bedrock",
                    "other-model",
                    serde_json::json!({ "signature": "secret-signature-value" }),
                ),
            },
            MODEL,
        );
        assert!(!logged.contains("secret-signature-value"), "{logged}");
    }
}

#[cfg(test)]
mod capability_report_tests {
    use super::*;
    use rho_core::{ContentBlock, Message, ReasoningEffort, Role};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct BufferWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for BufferWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufferWriter {
        type Writer = BufferWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn logged_for(model: &str, effort: Option<ReasoningEffort>) -> String {
        let buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(BufferWriter(Arc::clone(&buffer)))
            .with_max_level(tracing::Level::TRACE)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let request = CompletionRequest {
                model: model.to_string(),
                system: None,
                messages: vec![Message {
                    role: Role::User,
                    content: vec![ContentBlock::Text {
                        text: "hi".to_string(),
                    }],
                }],
                tools: Vec::new(),
                max_tokens: None,
                temperature: None,
                reasoning: effort,
            };
            build_thinking_fields(&request);
            tracing::warn!("the capture is live");
        });
        String::from_utf8(buffer.lock().unwrap().clone()).expect("valid utf8")
    }

    /// An id rho cannot read still fails closed, and it no longer does so in silence.
    ///
    /// An application inference profile hides the model, so a thinking-capable model reads
    /// as incapable. A review found it. The user asked for a level, so the user hears why
    /// nothing happened.
    #[test]
    fn an_unreadable_model_id_reports_that_it_asked_for_nothing() {
        let arn = "arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/abc123";
        let logged = logged_for(arn, Some(ReasoningEffort::High));
        assert!(logged.contains("the capture is live"), "the capture works");
        assert!(
            logged.contains("asked for none"),
            "the refusal is reported: {logged}"
        );
    }

    /// A user who asked for nothing hears nothing. A warning on every plain turn would be
    /// noise, and noise is how a real warning gets ignored.
    #[test]
    fn an_absent_effort_reports_nothing() {
        let logged = logged_for("amazon.nova-lite-v1:0", None);
        assert!(
            !logged.contains("asked for none"),
            "no report without a request: {logged}"
        );
    }

    /// `off` is an answer, not an ask, so it is quiet too.
    #[test]
    fn an_off_effort_reports_nothing() {
        let logged = logged_for("amazon.nova-lite-v1:0", Some(ReasoningEffort::Off));
        assert!(!logged.contains("asked for none"), "{logged}");
    }
}

#[cfg(test)]
mod unowned_stream_tests {
    use super::*;

    /// `events_to_stream` has no model, so it must mint no payload.
    ///
    /// A review found this. The fake transport built its state with `BedrockMapState::default()`,
    /// whose model is `""`, and the legacy `build_messages` also passes `""`. Two empty strings
    /// compare equal, so an unowned payload would have replayed on an unowned request. The
    /// stream now refuses to mint one, and `for_owner` refuses an empty name as well.
    #[tokio::test]
    async fn the_fake_transport_mints_no_unowned_payload() {
        use futures::StreamExt;

        let events = vec![
            ConverseStreamEvent {
                content_block_delta: Some(ContentBlockDelta {
                    content_block_index: 0,
                    delta: BlockDelta {
                        text: None,
                        tool_use: None,
                        reasoning_content: Some(ReasoningDelta {
                            text: Some("a plan".to_string()),
                            signature: Some("sig".to_string()),
                            redacted_content: None,
                        }),
                    },
                }),
                ..Default::default()
            },
            ConverseStreamEvent {
                content_block_stop: Some(ContentBlockStop {
                    content_block_index: 0,
                }),
                ..Default::default()
            },
        ];
        let mut stream = events_to_stream(events, CancelToken::new());
        let mut ends = Vec::new();
        while let Some(Ok(event)) = stream.next().await {
            if let StreamEvent::ThinkingEnd { state, .. } = event {
                ends.push(state);
            }
        }
        assert_eq!(
            ends,
            vec![None],
            "a stream with no model carries no owner, so it carries no payload"
        );
    }
}

#[cfg(test)]
mod wire_tests {
    //! The request rho really builds, read back from the SDK builder.
    //!
    //! Two reviews found the same hole from two directions: a source guard cannot tell a live
    //! call from a comment, and no test proved the thinking fields or the replayed signature
    //! reached a request at all. `apply_request` is the one place that assembles a request, so
    //! these tests build it offline and read it back with `as_input`. No network, no
    //! credentials, and no source text.

    use super::*;
    use aws_sdk_bedrockruntime::types::ContentBlock as SdkBlockAlias;
    use aws_sdk_bedrockruntime::types::ReasoningContentBlock;
    use rho_core::{ContentBlock, Message, ProviderState, ReasoningEffort, ReasoningOwner, Role};

    const MODEL: &str = "us.anthropic.claude-haiku-4-5-20251001-v1:0";

    /// An offline client. It signs nothing and sends nothing, because no test may use a
    /// network. See `AGENTS.md` step 5.
    fn offline_client() -> aws_sdk_bedrockruntime::Client {
        let config = aws_sdk_bedrockruntime::Config::builder()
            .behavior_version(aws_config::BehaviorVersion::latest())
            .region(aws_sdk_bedrockruntime::config::Region::new("us-east-1"))
            .build();
        aws_sdk_bedrockruntime::Client::from_conf(config)
    }

    fn request(
        model: &str,
        effort: Option<ReasoningEffort>,
        content: Vec<ContentBlock>,
    ) -> CompletionRequest {
        CompletionRequest {
            model: model.to_string(),
            system: None,
            messages: vec![Message {
                role: Role::Assistant,
                content,
            }],
            tools: Vec::new(),
            max_tokens: None,
            temperature: None,
            reasoning: effort,
        }
    }

    fn state(provider: &str, model: &str, signature: &str) -> Option<ProviderState> {
        Some(ProviderState {
            owner: ReasoningOwner {
                provider: provider.to_string(),
                model: model.to_string(),
            },
            value: serde_json::json!({ "signature": signature }),
        })
    }

    /// The thinking request reaches the real request object, not just a helper's return value.
    #[test]
    fn the_request_carries_the_thinking_fields() {
        let request = request(
            MODEL,
            Some(ReasoningEffort::Medium),
            vec![ContentBlock::Text {
                text: "hello".to_string(),
            }],
        );
        let builder = apply_request(offline_client().converse_stream(), &request);
        let input = builder.as_input();
        let fields = input
            .get_additional_model_request_fields()
            .as_ref()
            .expect("the request carries the thinking fields");
        let Document::Object(root) = fields else {
            panic!("the fields are an object");
        };
        let Some(Document::Object(thinking)) = root.get("thinking") else {
            panic!("the fields carry a thinking object");
        };
        assert_eq!(
            thinking.get("type"),
            Some(&Document::String("enabled".to_string()))
        );
        // And the model id and the messages travel with it.
        assert_eq!(input.get_model_id().as_deref(), Some(MODEL));
        assert_eq!(
            input.get_messages().as_ref().map(Vec::len),
            Some(1),
            "the message list reaches the request"
        );
    }

    /// A model that cannot think gets no field on the real request.
    #[test]
    fn a_plain_model_carries_no_thinking_fields() {
        let request = request(
            "amazon.nova-lite-v1:0",
            Some(ReasoningEffort::High),
            vec![ContentBlock::Text {
                text: "hello".to_string(),
            }],
        );
        let builder = apply_request(offline_client().converse_stream(), &request);
        assert!(
            builder
                .as_input()
                .get_additional_model_request_fields()
                .is_none()
        );
    }

    /// The replayed signature reaches the real request. This is the defect the live 400
    /// proved, now pinned offline.
    #[test]
    fn the_request_carries_a_replayed_signature() {
        let request = request(
            MODEL,
            Some(ReasoningEffort::Low),
            vec![ContentBlock::ReasoningReplay {
                text: "a plan".to_string(),
                state: state("bedrock", MODEL, "sig-wire"),
            }],
        );
        let builder = apply_request(offline_client().converse_stream(), &request);
        let messages = builder
            .as_input()
            .get_messages()
            .clone()
            .expect("the request carries messages");
        let signatures: Vec<String> = messages
            .iter()
            .flat_map(|message| message.content().iter())
            .filter_map(|block| match block {
                SdkBlockAlias::ReasoningContent(ReasoningContentBlock::ReasoningText(text)) => {
                    Some(text.signature().unwrap_or_default().to_string())
                }
                _ => None,
            })
            .collect();
        assert_eq!(signatures, vec!["sig-wire".to_string()]);
    }

    /// A foreign payload never reaches the request, whatever the helpers do.
    #[test]
    fn the_request_carries_no_foreign_signature() {
        let request = request(
            MODEL,
            Some(ReasoningEffort::Low),
            vec![ContentBlock::ReasoningReplay {
                text: "a plan".to_string(),
                state: state("bedrock", "another-model", "sig-wire"),
            }],
        );
        let builder = apply_request(offline_client().converse_stream(), &request);
        let messages = builder
            .as_input()
            .get_messages()
            .clone()
            .unwrap_or_default();
        let has_reasoning = messages
            .iter()
            .flat_map(|message| message.content().iter())
            .any(|block| matches!(block, SdkBlockAlias::ReasoningContent(_)));
        assert!(!has_reasoning, "a foreign payload must not reach the wire");
    }

    /// Rule 2 and rule 3 on the real request: the budget has room, and no temperature rides
    /// along with a thinking request.
    #[test]
    fn the_request_bounds_the_budget_and_drops_the_temperature() {
        let mut request = request(
            MODEL,
            Some(ReasoningEffort::XHigh),
            vec![ContentBlock::Text {
                text: "hello".to_string(),
            }],
        );
        request.temperature = Some(0.3);
        let builder = apply_request(offline_client().converse_stream(), &request);
        let config = builder
            .as_input()
            .get_inference_config()
            .clone()
            .expect("the request carries an inference config");
        let budget = ReasoningEffort::XHigh.budget_tokens().unwrap() as i32;
        assert!(config.max_tokens().expect("max tokens") > budget);
        assert_eq!(config.temperature(), None);
    }
}

#[cfg(test)]
mod family_table_tests {
    use super::*;

    /// Every row of the table is reachable, and each threshold is exact.
    ///
    /// A table nobody checks is worse than a parse, so each row gets a pair: the version
    /// below the threshold, and the version at it.
    #[test]
    fn every_family_row_is_tested() {
        assert_eq!(
            THINKING_FAMILIES.len(),
            1,
            "a new family row needs a pair of cases below, so this count is deliberate"
        );
        // anthropic.claude, since 3.7.
        assert!(!model_supports_thinking(
            "anthropic.claude-3-5-sonnet-20240620-v1:0"
        ));
        assert!(model_supports_thinking(
            "anthropic.claude-3-7-sonnet-20250219-v1:0"
        ));
    }

    /// A family rho does not know is asked for nothing, whatever its version.
    #[test]
    fn an_unknown_family_is_never_capable() {
        for model in [
            "meta.llama4-90b-instruct-v9:0",
            "amazon.nova-pro-v1:0",
            "mistral.mistral-large-2407-v1:0",
            "cohere.command-r-plus-v1:0",
        ] {
            assert!(!model_supports_thinking(model), "{model}");
        }
    }

    /// The 3.7 threshold, pinned from below as well as above.
    ///
    /// A mutation review found that `minor >= 7` could become `minor >= 6` and no test
    /// noticed, so an off-by-one would have enabled thinking on a model without it.
    #[test]
    fn the_threshold_is_exact_from_below() {
        assert!(!model_supports_thinking(
            "anthropic.claude-3-6-sonnet-20250101-v1:0"
        ));
        assert!(model_supports_thinking(
            "anthropic.claude-3-7-sonnet-20250219-v1:0"
        ));
    }

    /// A prefix-less id is still a Claude id. A review found this false negative, and the
    /// exact string it named.
    #[test]
    fn an_id_with_no_vendor_prefix_is_still_capable() {
        assert!(model_supports_thinking("claude-3-7-sonnet-20250219-v1:0"));
        assert!(model_supports_thinking("claude-haiku-4-5-20251001-v1:0"));
        assert!(!model_supports_thinking("claude-3-5-sonnet-20240620-v1:0"));
    }

    /// A dash-dated id must not offer its month as a minor version. A review found this
    /// false positive, which would have earned a 400 on a model without thinking.
    #[test]
    fn a_dash_dated_id_does_not_read_its_month_as_a_version() {
        assert!(
            !model_supports_thinking("anthropic.claude-3-opus-2025-07-15"),
            "the month 07 must not read as minor version 7"
        );
        // And the same shape on a capable major version still works.
        assert!(model_supports_thinking(
            "anthropic.claude-4-opus-2025-07-15"
        ));
    }

    /// The limit rho cannot fix by reading an id, stated as a test so nobody forgets it.
    ///
    /// An application inference profile hides the model. rho fails closed and reports, and
    /// `capability_report_tests` proves the report.
    #[test]
    fn an_inference_profile_arn_reads_as_incapable() {
        assert!(!model_supports_thinking(
            "arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/abc123"
        ));
    }
}

#[cfg(test)]
mod log_safety_tests {
    //! A session file is untrusted input, and its strings reach a log.
    //!
    //! A security review found that `owner.provider` and `owner.model` were logged with
    //! `Display` and no sanitiser, so a crafted file could inject a terminal escape or forge
    //! a log line. `rho-redact` is the one home for that, per `D-one-redaction-home`.

    use super::*;
    use rho_core::{ContentBlock, Message, ProviderState, ReasoningOwner, Role};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct BufferWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for BufferWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufferWriter {
        type Writer = BufferWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn a_hostile_owner_cannot_inject_a_terminal_escape() {
        let buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(BufferWriter(Arc::clone(&buffer)))
            .with_max_level(tracing::Level::TRACE)
            // The subscriber's own colours are escape bytes too. Turn them off, or the test
            // cannot tell the attacker's escape from the formatter's.
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let hostile = ProviderState {
                owner: ReasoningOwner {
                    provider: "bedrock".to_string(),
                    // An escape, a title-setting sequence, and a forged log line.
                    model: "\u{1b}]0;pwned\u{7}\nWARN forged".to_string(),
                },
                value: serde_json::json!({ "signature": "s" }),
            };
            build_messages_for_model(
                &[Message {
                    role: Role::Assistant,
                    content: vec![ContentBlock::ReasoningReplay {
                        text: "a plan".to_string(),
                        state: Some(hostile),
                    }],
                }],
                "us.anthropic.claude-haiku-4-5-20251001-v1:0",
            );
            tracing::warn!("the capture is live");
        });
        let logged = String::from_utf8(buffer.lock().unwrap().clone()).expect("utf8");
        assert!(logged.contains("the capture is live"), "the capture works");
        assert!(
            !logged.contains('\u{1b}'),
            "no escape byte reaches a log: {logged:?}"
        );
        assert!(
            !logged.contains("\nWARN forged"),
            "no forged line reaches a log: {logged:?}"
        );
    }
}

#[cfg(test)]
mod replay_scope_tests {
    //! Only the current tool loop replays its reasoning.
    //!
    //! A performance review found the cost: the prompt is append-only, so every stored
    //! reasoning block was re-sent on every later turn. Twenty turns re-upload turn one's
    //! trace nineteen times, which is O(turns squared) in bytes.
    //!
    //! Anthropic needs the thinking of the assistant turn that made the pending tool call, so
    //! that the signature chain of the current loop stays whole. A block from before the last
    //! user prompt is not needed, and re-sending it buys nothing.

    use super::*;
    use rho_core::{ContentBlock, Message, ProviderState, ReasoningOwner, Role};

    const MODEL: &str = "us.anthropic.claude-haiku-4-5-20251001-v1:0";

    fn replay(text: &str) -> ContentBlock {
        ContentBlock::ReasoningReplay {
            text: text.to_string(),
            state: Some(ProviderState {
                owner: ReasoningOwner {
                    provider: "bedrock".to_string(),
                    model: MODEL.to_string(),
                },
                value: serde_json::json!({ "signature": format!("sig-{text}") }),
            }),
        }
    }

    fn user(text: &str) -> Message {
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
        }
    }

    fn assistant(content: Vec<ContentBlock>) -> Message {
        Message {
            role: Role::Assistant,
            content,
        }
    }

    /// The reasoning texts that reached the wire, in order.
    fn sent(messages: &[Message]) -> Vec<String> {
        use aws_sdk_bedrockruntime::types::{
            ContentBlock as SdkBlock, ReasoningContentBlock as SdkReasoning,
        };
        build_messages_for_model(messages, MODEL)
            .iter()
            .flat_map(|message| message.content().iter())
            .filter_map(|block| match block {
                SdkBlock::ReasoningContent(SdkReasoning::ReasoningText(text)) => {
                    Some(text.text().to_string())
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn only_the_current_loop_replays_its_reasoning() {
        let messages = vec![
            user("first question"),
            assistant(vec![
                replay("old reasoning"),
                ContentBlock::Text {
                    text: "first answer".to_string(),
                },
            ]),
            user("second question"),
            assistant(vec![replay("current reasoning")]),
        ];
        assert_eq!(
            sent(&messages),
            vec!["current reasoning".to_string()],
            "a block from before the last prompt buys nothing and costs every turn"
        );
    }

    /// Inside one loop, every assistant turn keeps its reasoning, because the chain of the
    /// pending call must stay whole.
    #[test]
    fn a_whole_tool_loop_keeps_its_reasoning() {
        let messages = vec![
            user("do it"),
            assistant(vec![replay("first thought")]),
            Message {
                role: Role::Tool,
                content: vec![ContentBlock::ToolResult {
                    tool_call_id: "1".to_string(),
                    content: vec![ContentBlock::Text {
                        text: "result".to_string(),
                    }],
                    is_error: false,
                }],
            },
            assistant(vec![replay("second thought")]),
        ];
        assert_eq!(
            sent(&messages),
            vec!["first thought".to_string(), "second thought".to_string()],
            "a tool result does not end the loop, so both turns replay"
        );
    }

    /// The text of a dropped block does not leak into the request as prose either.
    #[test]
    fn an_out_of_scope_block_leaves_no_text_behind() {
        let messages = vec![
            user("first"),
            assistant(vec![replay("old reasoning")]),
            user("second"),
        ];
        let built = build_messages_for_model(&messages, MODEL);
        let text: String = built
            .iter()
            .flat_map(|message| message.content().iter())
            .filter_map(|block| block.as_text().ok())
            .cloned()
            .collect();
        assert!(
            !text.contains("old reasoning"),
            "a dropped block never becomes prose: {text}"
        );
    }
}
