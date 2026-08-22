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
    // `Thinking { thinking, signature }` is split into a trace and a replay block. The
    // rule this test guards is unchanged: an absent payload writes no key. The typed
    // signature is gone, because a payload is now opaque and owned by one provider. See
    // `D-reasoning-replay-is-opaque-provider-state`.
    let block = ContentBlock::ReasoningReplay {
        text: "reasoning".to_string(),
        state: None,
    };
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "type": "thinking", "thinking": "reasoning" })
    );
    assert!(json.get("state").is_none());
    assert!(json.get("signature").is_none());
}

#[test]
fn content_block_tool_call_roundtrips_json() {
    let block = ContentBlock::ToolCall {
        id: "call_1".to_string(),
        name: "read".to_string(),
        arguments: serde_json::json!({ "path": "a.txt" }),
        state: None,
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

// ---- The reasoning split, and the opaque replay payload. ----
// See SPEC-reasoning-across-providers section 4, and the two decisions
// D-reasoning-replay-is-opaque-provider-state and D-two-variants-cannot-share-a-serde-tag.

use rho_core::{ProviderState, ReasoningOwner};

fn state(provider: &str, model: &str) -> ProviderState {
    ProviderState {
        owner: ReasoningOwner {
            provider: provider.to_string(),
            model: model.to_string(),
        },
        value: serde_json::json!({ "signature": "sig-1" }),
    }
}

#[test]
fn a_trace_serialises_under_the_old_thinking_tag() {
    let block = ContentBlock::ReasoningTrace {
        text: "reasoning".to_string(),
    };
    let json = serde_json::to_value(&block).unwrap();
    // An old rho reads `thinking` and ignores what it does not know, so it still loads.
    assert_eq!(
        json,
        serde_json::json!({ "type": "thinking", "thinking": "reasoning" })
    );
}

#[test]
fn a_state_round_trips_through_the_session_file() {
    let block = ContentBlock::ReasoningReplay {
        text: "reasoning".to_string(),
        state: Some(state("bedrock", "claude")),
    };
    let line = serde_json::to_string(&block).unwrap();
    let back: ContentBlock = serde_json::from_str(&line).unwrap();
    assert_eq!(back, block, "the payload survives the file unchanged");
    let json: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        json["type"], "thinking",
        "the old tag carries both variants"
    );
    assert_eq!(json["replay"], true);
}

#[test]
fn an_old_thinking_block_imports_as_a_trace() {
    // A file written before this change. Its signature is stale, so it never replays.
    let json = serde_json::json!({
        "type": "thinking", "thinking": "old", "signature": "stale"
    });
    let back: ContentBlock = serde_json::from_value(json).unwrap();
    assert_eq!(
        back,
        ContentBlock::ReasoningTrace {
            text: "old".to_string()
        }
    );
}

#[test]
fn a_replay_key_with_no_state_reads_as_a_trace() {
    // Fail closed. There is nothing to replay, so it is history.
    let json = serde_json::json!({ "type": "thinking", "thinking": "x", "replay": true });
    let back: ContentBlock = serde_json::from_value(json).unwrap();
    assert_eq!(
        back,
        ContentBlock::ReasoningTrace {
            text: "x".to_string()
        }
    );
}

#[test]
fn a_new_block_is_readable_by_an_old_rho() {
    // The old shape: a tagged enum with one `thinking` variant and no `replay` key.
    #[derive(serde::Deserialize, Debug, PartialEq)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum OldBlock {
        Thinking {
            thinking: String,
            #[serde(default)]
            signature: Option<String>,
        },
    }
    let line = serde_json::to_string(&ContentBlock::ReasoningReplay {
        text: "new".to_string(),
        state: Some(state("bedrock", "claude")),
    })
    .unwrap();
    let old: OldBlock = serde_json::from_str(&line).expect("an old rho loads the file");
    assert_eq!(
        old,
        OldBlock::Thinking {
            thinking: "new".to_string(),
            signature: None
        }
    );
}

#[test]
fn a_state_answers_only_for_its_own_owner() {
    // Rule 8, in one place, so no provider hand-rolls it. The value is unreachable for
    // any other provider or model.
    let state = state("bedrock", "claude-haiku");
    assert!(state.for_owner("bedrock", "claude-haiku").is_some());
    assert!(
        state.for_owner("bedrock", "claude-opus").is_none(),
        "another model gets nothing"
    );
    assert!(
        state.for_owner("openrouter", "claude-haiku").is_none(),
        "another provider gets nothing"
    );
}

#[test]
fn a_tool_call_carries_a_state() {
    // Gemini binds a replay token to the call itself. The field exists now, so that crate
    // needs no edit to shared code.
    let block = ContentBlock::ToolCall {
        id: "1".to_string(),
        name: "read".to_string(),
        arguments: serde_json::json!({}),
        state: Some(state("gemini", "flash")),
    };
    let line = serde_json::to_string(&block).unwrap();
    let back: ContentBlock = serde_json::from_str(&line).unwrap();
    assert_eq!(back, block);
}

#[test]
fn an_old_tool_call_loads_with_no_state() {
    let json = serde_json::json!({
        "type": "tool_call", "id": "1", "name": "read", "arguments": {}
    });
    let back: ContentBlock = serde_json::from_value(json).unwrap();
    assert!(matches!(back, ContentBlock::ToolCall { state: None, .. }));
}
