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

        let mut builder = client
            .converse_stream()
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
        if let Some(config) = build_inference_config(&request) {
            builder = builder.inference_config(config);
        }
        if let Some(fields) = build_thinking_fields(&request) {
            builder = builder.additional_model_request_fields(fields);
        }
        if let Some(tools) = build_tool_config(&request) {
            builder = builder.tool_config(tools);
        }

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

/// Build the SDK message list from the normalised messages. Sprint 1 sends text
/// and tool calls. See `SPEC-provider-interface` section 8 for the out-of-scope block kinds.
pub fn build_messages(messages: &[Message]) -> Vec<aws_sdk_bedrockruntime::types::Message> {
    // Kept for the contract suite, which builds messages with no model in hand. A reasoning
    // payload needs the model, so a caller that replays uses `build_messages_for_model`.
    build_messages_for_model(messages, "")
}

/// Build the request messages for one model. The model decides whether a stored reasoning
/// payload may travel, per rule 8.
pub fn build_messages_for_model(
    messages: &[Message],
    model: &str,
) -> Vec<aws_sdk_bedrockruntime::types::Message> {
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
    for message in messages {
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
                    if let Some(reasoning) = replay_block(text, state, model) {
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

    let value = state.as_ref()?.for_owner(PROVIDER_ID, model)?;
    if let Some(redacted) = value.get("redacted").and_then(Value::as_str) {
        use base64::Engine;
        // The payload holds base64, because a blob is not valid UTF-8 in general. A failed
        // decode sends nothing: a wrong blob is worse than a missing one, because Bedrock
        // would reject the whole turn.
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(redacted)
            .ok()?;
        return Some(ReasoningContentBlock::RedactedContent(
            aws_smithy_types::Blob::new(bytes),
        ));
    }
    let signature = value.get("signature").and_then(Value::as_str)?;
    ReasoningTextBlock::builder()
        .text(text)
        .signature(signature)
        .build()
        .ok()
        .map(ReasoningContentBlock::ReasoningText)
}

/// Does this model id support Anthropic extended thinking?
///
/// It fails closed. Only a Claude id at version 3.7 or above answers `true`, because a
/// field the endpoint does not know is a 400 for the whole turn. An id rho cannot read is
/// treated as "no". See `SPEC-reasoning-across-providers` section 9 rule 1.
fn model_supports_thinking(model: &str) -> bool {
    let id = model.to_ascii_lowercase();
    // A Bedrock id may carry a region prefix, as in `us.anthropic.claude-...`.
    let Some(after) = id.split("anthropic.claude").nth(1) else {
        return false;
    };
    // The first two numbers after the family name are the version. `claude-3-5-sonnet`
    // gives 3 and 5, and `claude-haiku-4-5-2025...` gives 4 and 5.
    //
    // A version part is always under 100. A release date is not, and
    // `anthropic.claude-3-haiku-20240307` read its date as minor version 20240307, which
    // made a model without thinking claim it. The test found that, so the bound stays.
    let mut version = after
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u32>().ok())
        .filter(|number| *number < 100);
    let Some(major) = version.next() else {
        return false;
    };
    let minor = version.next().unwrap_or(0);
    major > 3 || (major == 3 && minor >= 7)
}

/// Build the `additionalModelRequestFields` that ask Claude for extended thinking.
///
/// `None` means rho asks for nothing: no effort, `Off`, or a model that cannot think.
fn build_thinking_fields(request: &CompletionRequest) -> Option<Document> {
    let budget = request.reasoning?.budget_tokens()?;
    if !model_supports_thinking(&request.model) {
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
        // Rule 2. Keep the caller's number when it already clears the budget.
        (Some(budget), Some(max_tokens)) if max_tokens > budget => {
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
        // Search the production half only. The first version of this guard searched the
        // whole file, so it matched its own assertion string and passed against a deleted
        // call. Step 7 caught that, and the split is the fix.
        let source = include_str!("lib.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("a source file has a first part");
        assert!(
            production.contains("builder.additional_model_request_fields(fields)"),
            "the converse builder must send the thinking fields"
        );
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
