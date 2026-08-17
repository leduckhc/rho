//! Naming tests for `rho-mcp`, all offline.
//!
//! A tool name is namespaced and validated before it becomes a registry key.
//! See `SPEC-09` sections 5 and 6.

mod common;

use std::sync::Arc;

use common::{FakeFactory, test_limits};
use rho_mcp::{
    McpError, McpPool, McpSchemaCache, McpServerConfig, McpToolDef, McpTransport, dispatch_name,
    tools_for, validate_tool_name,
};

#[test]
fn dispatch_name_prefixes_the_server_and_replaces_hyphens() {
    let name = dispatch_name("my-server", "search-files");
    assert_eq!(name, "mcp__my_server__search_files");
    assert!(!name.contains('-'), "no hyphen survives");
}

#[test]
fn a_tool_name_with_a_path_separator_is_refused() {
    let error = validate_tool_name("srv", "../etc/passwd").unwrap_err();
    assert!(matches!(error, McpError::InvalidToolName { .. }));
    let error = validate_tool_name("srv", "a\\b").unwrap_err();
    assert!(matches!(error, McpError::InvalidToolName { .. }));
}

#[test]
fn a_tool_name_with_a_control_character_is_refused() {
    let error = validate_tool_name("srv", "bad\u{7}name").unwrap_err();
    assert!(matches!(error, McpError::InvalidToolName { .. }));
    // A leading dot is refused too.
    let error = validate_tool_name("srv", ".hidden").unwrap_err();
    assert!(matches!(error, McpError::InvalidToolName { .. }));
}

fn config(name: &str) -> McpServerConfig {
    McpServerConfig {
        name: name.to_string(),
        transport: McpTransport::Stdio {
            command: name.to_string(),
            args: vec![],
        },
        env: Default::default(),
        shared: true,
        call_timeout_ms: None,
    }
}

fn tool_def(name: &str) -> McpToolDef {
    McpToolDef {
        name: name.to_string(),
        description: "A tool.".to_string(),
        input_schema: serde_json::json!({ "type": "object" }),
    }
}

#[tokio::test]
async fn two_servers_that_collide_after_replacement_are_an_error() {
    // `srv-a` and `srv_a` both become `srv_a` after the hyphen replacement, so a
    // shared tool name collides. The error must name both servers.
    let pool = McpPool::with_factory(test_limits(), Arc::new(FakeFactory));
    let config_a = config("srv-a");
    let config_b = config("srv_a");
    let mut cache = McpSchemaCache::new();
    cache.update(&config_a, vec![tool_def("x")]);
    cache.update(&config_b, vec![tool_def("x")]);

    let error = match tools_for(&pool, &[config_a, config_b], &cache).await {
        Ok(_) => panic!("a collision must be an error"),
        Err(error) => error,
    };
    match error {
        McpError::DuplicateToolName {
            name,
            first,
            second,
        } => {
            assert_eq!(name, "mcp__srv_a__x");
            assert_eq!(first, "srv-a");
            assert_eq!(second, "srv_a");
        }
        other => panic!("wrong error: {other}"),
    }
}
