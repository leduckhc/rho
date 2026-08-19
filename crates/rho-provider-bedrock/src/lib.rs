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
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    pub text: String,
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
            if state.thinking_started.insert(index) {
                out.push(StreamEvent::ThinkingStart { index });
            }
            out.push(StreamEvent::ThinkingDelta {
                index,
                delta: reasoning.text,
            });
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
            out.push(StreamEvent::ToolCallEnd { index, arguments });
        } else if state.text_started.remove(&index) {
            out.push(StreamEvent::TextEnd { index });
        } else if state.thinking_started.remove(&index) {
            out.push(StreamEvent::ThinkingEnd {
                index,
                signature: None,
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
            .set_messages(Some(build_messages(&request.messages)));
        if let Some(system) = &request.system {
            builder = builder.system(aws_sdk_bedrockruntime::types::SystemContentBlock::Text(
                system.clone(),
            ));
        }
        if let Some(config) = build_inference_config(&request) {
            builder = builder.inference_config(config);
        }
        if let Some(tools) = build_tool_config(&request) {
            builder = builder.tool_config(tools);
        }

        let output = builder
            .send()
            .await
            .map_err(|error| map_sdk_error(error.code(), error.to_string()))?;

        let mut receiver = output.stream;
        let stream = stream! {
            let mut state = BedrockMapState::default();
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
                    reasoning_content: Some(ReasoningDelta { text }),
                },
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
                // Reasoning never travels to Bedrock in phase 1. rho does not yet ask for
                // extended thinking, and replaying a reasoning block needs the per-endpoint
                // rules that are phase 2. Drop it here in a named arm, never by `_ => {}`,
                // so the drop is stated. See SPEC-reasoning-across-providers section 3 "Three".
                ContentBlock::Thinking { .. } => {}
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
fn build_inference_config(
    request: &CompletionRequest,
) -> Option<aws_sdk_bedrockruntime::types::InferenceConfiguration> {
    if request.max_tokens.is_none() && request.temperature.is_none() {
        return None;
    }
    let mut builder = aws_sdk_bedrockruntime::types::InferenceConfiguration::builder();
    if let Some(max_tokens) = request.max_tokens {
        builder = builder.max_tokens(max_tokens as i32);
    }
    if let Some(temperature) = request.temperature {
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
