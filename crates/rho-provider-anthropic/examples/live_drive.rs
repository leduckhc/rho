//! Live drive against the xdent Anthropic route. Not a test; a bin. Run:
//!
//! ```
//! cargo run --release -p rho-provider-anthropic --example live_drive
//! ```
//!
//! It POSTs one prompt through the tunnel and prints every StreamEvent. The proxy holds
//! the credential, so a dummy key here is fine. Change the base URL to hit real Anthropic.

use futures::StreamExt;
use rho_core::{CancelToken, CompletionRequest, ContentBlock, Message, Provider, Role, Secret};
use rho_provider_anthropic::{AnthropicConfig, AnthropicProvider};
use std::time::Instant;

const XDENT_CLAUDE: &str = "http://127.0.0.1:58788/dev1/anthropic";
const MODEL: &str = "claude-sonnet-4-6";

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| "dummy".to_string());
    let base_url = std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| XDENT_CLAUDE.to_string());
    let model = std::env::var("MODEL").unwrap_or_else(|_| MODEL.to_string());
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Reply with only the word banana.".to_string());

    println!("base_url = {base_url}");
    println!("model    = {model}");
    println!("prompt   = {prompt}");
    println!("---");

    let provider = AnthropicProvider::new(AnthropicConfig::new(base_url, Secret::from(key)));
    let request = CompletionRequest {
        model,
        system: None,
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: prompt }],
        }],
        tools: Vec::new(),
        max_tokens: Some(200),
        temperature: None,
        reasoning: None,
    };

    let started = Instant::now();
    let mut stream = match provider.stream(request, CancelToken::new()).await {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("stream open failed: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };

    let mut answer = String::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(event) => {
                if let rho_core::StreamEvent::TextDelta { delta, .. } = &event {
                    answer.push_str(delta);
                }
                println!("EVENT {event:?}");
            }
            Err(error) => {
                eprintln!("stream error: {error}");
                return std::process::ExitCode::FAILURE;
            }
        }
    }
    println!("---");
    println!("elapsed = {:?}", started.elapsed());
    println!("answer  = {answer:?}");
    std::process::ExitCode::SUCCESS
}
