//! Tests for the content-block model. These prove the stable JSON shape.

use rho_core::{ContentBlock, ImageSource};

#[test]
fn content_block_text_roundtrips_json() {
    let block = ContentBlock::Text {
        text: "hello".to_string(),
    };
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(json, serde_json::json!({ "type": "text", "text": "hello" }));
    let back: ContentBlock = serde_json::from_value(json).unwrap();
    assert_eq!(back, block);
}

#[test]
fn content_block_thinking_omits_absent_signature() {
    let block = ContentBlock::Thinking {
        thinking: "reasoning".to_string(),
        signature: None,
    };
    let json = serde_json::to_value(&block).unwrap();
    // The `signature` key must be absent when it is `None`.
    assert_eq!(
        json,
        serde_json::json!({ "type": "thinking", "thinking": "reasoning" })
    );
    assert!(json.get("signature").is_none());
}

#[test]
fn content_block_tool_call_roundtrips_json() {
    let block = ContentBlock::ToolCall {
        id: "call_1".to_string(),
        name: "read".to_string(),
        arguments: serde_json::json!({ "path": "a.txt" }),
    };
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "type": "tool_call",
            "id": "call_1",
            "name": "read",
            "arguments": { "path": "a.txt" }
        })
    );
    let back: ContentBlock = serde_json::from_value(json).unwrap();
    assert_eq!(back, block);
}

#[test]
fn content_block_tool_result_roundtrips_json() {
    let block = ContentBlock::ToolResult {
        tool_call_id: "call_1".to_string(),
        content: vec![
            ContentBlock::Text {
                text: "ok".to_string(),
            },
            ContentBlock::Image {
                source: ImageSource {
                    data: "AAAA".to_string(),
                    mime_type: "image/png".to_string(),
                },
            },
        ],
        is_error: true,
    };
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(json["type"], "tool_result");
    assert_eq!(json["tool_call_id"], "call_1");
    assert_eq!(json["is_error"], true);
    assert_eq!(
        json["content"][0],
        serde_json::json!({ "type": "text", "text": "ok" })
    );
    let back: ContentBlock = serde_json::from_value(json).unwrap();
    assert_eq!(back, block);
}
