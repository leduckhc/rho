//! The request body is one shape, and this file pins it.
//!
//! The Anthropic Messages API is close to OpenAI's Chat Completions, and close is enough
//! for a defect. Two differences bite: `system` is a top-level field on Anthropic, not a
//! message with role `system`. And `max_tokens` is required.
//!
//! Every test here asserts against a small, hand-written JSON value. A single call to
//! `build_request_body` produces it, and the test reads the fields it cares about. So a
//! future field is a compile error at the assertion site, not a silent drop.

use rho_core::{CompletionRequest, ContentBlock, Message, Role};
use rho_provider_anthropic::build_request_body;
use serde_json::json;

fn one_user_turn(text: &str) -> CompletionRequest {
    CompletionRequest {
        model: "claude-sonnet-4-6".to_string(),
        system: None,
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
        }],
        tools: Vec::new(),
        max_tokens: Some(1024),
        temperature: None,
        reasoning: None,
    }
}

#[test]
fn a_bare_prompt_carries_only_the_message_and_the_required_fields() {
    let body = build_request_body(&one_user_turn("hello"));
    assert_eq!(body["model"], "claude-sonnet-4-6");
    assert_eq!(body["max_tokens"], 1024);
    assert_eq!(body["stream"], true);
    assert_eq!(
        body["messages"],
        json!([{"role": "user", "content": [{"type": "text", "text": "hello"}]}])
    );
    assert!(
        body.get("system").is_none(),
        "no system prompt means no system field: {body}"
    );
    assert!(
        body.get("tools").is_none(),
        "no tools means no tools field: {body}"
    );
    assert!(
        body.get("thinking").is_none(),
        "no reasoning means no thinking field: {body}"
    );
}

#[test]
fn a_system_prompt_is_top_level_never_a_message() {
    // A drive against the earlier spec-authoring lesson: the wire is not OpenAI, and a
    // system message would be refused. The field lives at the top level.
    let mut request = one_user_turn("hi");
    request.system = Some("you are a test".to_string());
    let body = build_request_body(&request);
    assert_eq!(body["system"], "you are a test");
    let messages = body["messages"].as_array().expect("messages is an array");
    assert!(
        messages.iter().all(|message| message["role"] != "system"),
        "no message may carry role `system`: {body}"
    );
}

#[test]
fn max_tokens_is_required_and_absent_is_a_provider_default() {
    // Anthropic 400s a request with no `max_tokens`. rho carries `Option<u32>`, so an
    // absent value must still send a number. The spec picks 4096 as the safe default,
    // matching the sibling providers.
    let mut request = one_user_turn("hi");
    request.max_tokens = None;
    let body = build_request_body(&request);
    assert!(
        body["max_tokens"].as_u64().is_some(),
        "an absent max_tokens must send the default, not the field's absence: {body}"
    );
    assert_eq!(body["max_tokens"], 4096, "the spec's default is 4096");
}

// ---- tool calls ----

use rho_core::{ToolKind, ToolSpec};

fn read_tool() -> ToolSpec {
    ToolSpec {
        name: "read".into(),
        description: "Read a file.".into(),
        kind: ToolKind::Read,
        input_schema: json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"],
        }),
    }
}

#[test]
fn a_tools_list_carries_name_description_and_input_schema() {
    let mut request = one_user_turn("read a file");
    request.tools = vec![read_tool()];
    let body = build_request_body(&request);
    let tools = body["tools"].as_array().expect("tools is an array");
    assert_eq!(tools.len(), 1);
    let tool = &tools[0];
    // Anthropic's shape: {name, description, input_schema}. NOT wrapped in a `function`
    // object like OpenAI. NO `type: "function"` field either. A silent nesting is a defect.
    assert_eq!(tool["name"], "read");
    assert_eq!(tool["description"], "Read a file.");
    assert_eq!(tool["input_schema"]["type"], "object");
    assert!(tool.get("type").is_none(), "no `type` wrapper: {tool}");
    assert!(
        tool.get("function").is_none(),
        "no `function` wrapper (that is OpenAI, not Anthropic): {tool}"
    );
}

#[test]
fn a_tool_call_in_an_assistant_message_reads_as_tool_use() {
    let request = CompletionRequest {
        model: "claude-sonnet-4-6".to_string(),
        system: None,
        messages: vec![Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolCall {
                id: "call-1".to_string(),
                name: "read".to_string(),
                arguments: json!({"path": "fact.txt"}),
                state: None,
            }],
        }],
        tools: vec![read_tool()],
        max_tokens: Some(200),
        temperature: None,
        reasoning: None,
    };
    let body = build_request_body(&request);
    let block = &body["messages"][0]["content"][0];
    assert_eq!(block["type"], "tool_use");
    assert_eq!(block["id"], "call-1");
    assert_eq!(block["name"], "read");
    // The parsed JSON `arguments` become the `input` field verbatim.
    assert_eq!(block["input"], json!({"path": "fact.txt"}));
}

#[test]
fn a_tool_result_rides_on_a_user_turn_as_tool_result() {
    let request = CompletionRequest {
        model: "claude-sonnet-4-6".to_string(),
        system: None,
        messages: vec![Message {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                tool_call_id: "call-1".to_string(),
                content: vec![ContentBlock::Text {
                    text: "the pass phrase is turquoise".to_string(),
                }],
                is_error: false,
            }],
        }],
        tools: Vec::new(),
        max_tokens: Some(200),
        temperature: None,
        reasoning: None,
    };
    let body = build_request_body(&request);
    // The role is `user` on Anthropic. A `tool` role would 400.
    assert_eq!(body["messages"][0]["role"], "user");
    let block = &body["messages"][0]["content"][0];
    assert_eq!(block["type"], "tool_result");
    assert_eq!(block["tool_use_id"], "call-1");
    assert_eq!(
        block["content"],
        json!([{"type": "text", "text": "the pass phrase is turquoise"}])
    );
    assert!(
        block.get("is_error").is_none() || block["is_error"] == false,
        "a happy tool result carries no error flag or a false one: {block}"
    );
}

#[test]
fn a_failed_tool_result_reports_is_error_true() {
    let request = CompletionRequest {
        model: "claude-sonnet-4-6".to_string(),
        system: None,
        messages: vec![Message {
            role: Role::Tool,
            content: vec![ContentBlock::ToolResult {
                tool_call_id: "call-1".to_string(),
                content: vec![ContentBlock::Text {
                    text: "No such file".to_string(),
                }],
                is_error: true,
            }],
        }],
        tools: Vec::new(),
        max_tokens: Some(200),
        temperature: None,
        reasoning: None,
    };
    let body = build_request_body(&request);
    let block = &body["messages"][0]["content"][0];
    assert_eq!(block["is_error"], true);
}
