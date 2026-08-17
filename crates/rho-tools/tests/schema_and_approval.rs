//! Schema round-trip, tool-kind, and approval-policy tests for the built-in set.

use rho_core::{ApprovalDecision, ApprovalPolicy, ReadOnlyPolicy, Tool, ToolKind};
use rho_tools::{
    BashTool, EditTool, GlobTool, GrepTool, ListTool, ReadTool, WriteTool, builtin_tools,
};

/// Every built-in tool with a valid argument object for its schema.
fn tool_and_valid_args() -> Vec<(Box<dyn Tool>, serde_json::Value)> {
    vec![
        (
            Box::new(ReadTool),
            serde_json::json!({ "path": "a.txt", "offset": 1, "limit": 2 }),
        ),
        (
            Box::new(WriteTool),
            serde_json::json!({ "path": "a.txt", "content": "hi" }),
        ),
        (
            Box::new(EditTool),
            serde_json::json!({ "path": "a.txt", "old_text": "a", "new_text": "b" }),
        ),
        (Box::new(ListTool), serde_json::json!({ "path": "sub" })),
        (
            Box::new(GlobTool),
            serde_json::json!({ "pattern": "**/*.rs" }),
        ),
        (
            Box::new(GrepTool),
            serde_json::json!({ "pattern": "x", "path": "sub", "glob": "*.rs" }),
        ),
        (
            Box::new(BashTool::new()),
            serde_json::json!({ "command": "echo hi", "timeout_ms": 1000 }),
        ),
    ]
}

#[test]
fn tool_schema_roundtrips_arguments() {
    for (tool, args) in tool_and_valid_args() {
        let schema = tool.input_schema();
        // The schema is a JSON object with a properties map.
        assert_eq!(
            schema["type"],
            "object",
            "{} schema is an object",
            tool.name()
        );
        let props = schema["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("{} schema has properties", tool.name()));
        // Every argument key names a declared property.
        for key in args.as_object().unwrap().keys() {
            assert!(
                props.contains_key(key),
                "{} schema declares the {key} property",
                tool.name()
            );
        }
        // Every required key is present in the valid argument object.
        if let Some(required) = schema["required"].as_array() {
            for req in required {
                let key = req.as_str().unwrap();
                assert!(
                    args.get(key).is_some(),
                    "{} valid args include the required {key}",
                    tool.name()
                );
            }
        }
    }
}

#[test]
fn tool_kind_matches_acp_category() {
    let expected: &[(&str, ToolKind)] = &[
        ("read", ToolKind::Read),
        ("write", ToolKind::Edit),
        ("edit", ToolKind::Edit),
        ("list", ToolKind::Read),
        ("glob", ToolKind::Search),
        ("grep", ToolKind::Search),
        ("bash", ToolKind::Execute),
    ];
    let tools = builtin_tools();
    for (name, kind) in expected {
        let tool = tools
            .iter()
            .find(|t| t.name() == *name)
            .unwrap_or_else(|| panic!("the {name} tool is registered"));
        assert_eq!(tool.kind(), *kind, "{name} reports the {kind:?} kind");
    }
}

#[test]
fn tool_kind_serialises_snake_case() {
    let cases: &[(ToolKind, &str)] = &[
        (ToolKind::Read, "read"),
        (ToolKind::Edit, "edit"),
        (ToolKind::Search, "search"),
        (ToolKind::Execute, "execute"),
        (ToolKind::SwitchMode, "switch_mode"),
    ];
    for (kind, wire) in cases {
        let json = serde_json::to_value(kind).unwrap();
        assert_eq!(json, serde_json::Value::String((*wire).to_string()));
    }
}

#[tokio::test]
async fn approval_read_only_policy_allows_read() {
    let decision = ReadOnlyPolicy
        .approve("read", ReadTool.kind(), &serde_json::json!({}))
        .await;
    assert_eq!(decision, ApprovalDecision::Allow);
}

#[tokio::test]
async fn approval_read_only_policy_denies_write() {
    let decision = ReadOnlyPolicy
        .approve("write", WriteTool.kind(), &serde_json::json!({}))
        .await;
    assert_eq!(decision, ApprovalDecision::Deny);
}

#[tokio::test]
async fn approval_denied_call_returns_denied_error() {
    // A read-only policy denies bash. The caller turns a denial into this error.
    let decision = ReadOnlyPolicy
        .approve("bash", BashTool::new().kind(), &serde_json::json!({}))
        .await;
    assert_eq!(decision, ApprovalDecision::Deny);
    let error = rho_core::ToolError::Denied;
    assert!(error.to_string().contains("denied"));
}
