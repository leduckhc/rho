//! The AWS Bedrock provider.
//!
//! The wire-to-event mapping is a set of pure functions. A test drives them
//! against recorded `ConverseStream` event payloads, with no AWS client and no
//! network. See `SPEC-02` section 5.
//!
//! The real `Provider::stream` uses `aws-sdk-bedrockruntime` and signs with
//! SigV4 from the standard credential chain. Stage S6 fills it in. It feeds the
//! same pure mapping functions the tests exercise. This split lets the tests
//! avoid the AWS event-stream binary framing.

use async_trait::async_trait;
use rho_core::{
    CancelToken, CompletionRequest, Provider, ProviderError, ProviderStream, StreamEvent,
};
use serde::Deserialize;

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
    tool_inputs: std::collections::HashMap<u32, String>,
    text_started: std::collections::HashSet<u32>,
}

/// Map one `ConverseStream` event to zero or more normalised events. See
/// `SPEC-02` section 5.
pub fn map_converse_event(
    state: &mut BedrockMapState,
    event: ConverseStreamEvent,
) -> Vec<StreamEvent> {
    let _ = (&state.tool_inputs, &state.text_started, &event);
    todo!("SPEC-02 section 5: map a ConverseStream event to StreamEvent")
}

/// Map a Bedrock exception name to a provider error. See `SPEC-02` section 5.
pub fn map_converse_error(exception_name: &str) -> ProviderError {
    let _ = exception_name;
    todo!("SPEC-02 section 5: map a Bedrock exception to ProviderError")
}

/// Turn a recorded event list into a provider stream. The mapping runs in
/// order and the stream yields each event as it maps. This is the fake
/// transport for the shared contract.
pub fn events_to_stream(events: Vec<ConverseStreamEvent>, cancel: CancelToken) -> ProviderStream {
    let _ = (events, cancel);
    todo!("SPEC-02 section 5: stream mapped events in order")
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
        let _ = (&self.config, &request, &cancel);
        todo!("SPEC-02 section 5: call ConverseStream and feed map_converse_event")
    }
}
