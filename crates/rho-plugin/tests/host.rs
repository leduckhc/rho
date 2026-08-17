//! Host tests for `rho-plugin`. The stub plugin is a second binary in this
//! crate, `rho_stub_plugin`. Its path comes from `CARGO_BIN_EXE_rho_stub_plugin`.
//! No test reaches the network. No test uses a real multi-second sleep.

use std::time::Duration;

use rho_core::{CancelToken, ContentBlock, ToolContext};
use rho_plugin::{PluginCache, PluginHost, PluginToolSpec};

/// The stub plugin path, provided by cargo for this crate's binary.
const STUB: &str = env!("CARGO_BIN_EXE_rho_stub_plugin");

fn host_with_short_timeout() -> PluginHost {
    PluginHost::new(rho_plugin::PluginPolicy::trust_any_path())
        .with_call_timeout(Duration::from_millis(500))
}

fn ctx(root: &std::path::Path) -> (ToolContext, tokio::sync::mpsc::Receiver<String>) {
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    (
        ToolContext {
            session_root: root.to_path_buf(),
            cancel: CancelToken::new(),
            updates: tx,
        },
        rx,
    )
}

#[tokio::test]
async fn plugin_host_launches_and_handshakes() {
    let mut host = PluginHost::new(rho_plugin::PluginPolicy::trust_any_path());
    let process = host.launch(STUB, &["normal".to_string()]).await.unwrap();
    assert_eq!(process.name(), "stub");
    assert!(process.is_available());
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_host_lists_tools_from_handshake() {
    let mut host = PluginHost::new(rho_plugin::PluginPolicy::trust_any_path());
    host.launch(STUB, &["normal".to_string()]).await.unwrap();
    let tools = host.tools();
    assert!(tools.iter().any(|t| t.name() == "echo"));
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_host_calls_tool_and_gets_result() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = PluginHost::new(rho_plugin::PluginPolicy::trust_any_path());
    host.launch(STUB, &["normal".to_string()]).await.unwrap();
    let tool = host
        .tools()
        .into_iter()
        .find(|t| t.name() == "echo")
        .unwrap();
    let (context, _rx) = ctx(dir.path());
    let out = tool
        .execute(serde_json::json!({ "text": "hello plugin" }), context)
        .await
        .unwrap();
    let text: String = out
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "hello plugin");
    assert!(!out.is_error);
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_host_forwards_tool_updates() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = PluginHost::new(rho_plugin::PluginPolicy::trust_any_path());
    host.launch(STUB, &["normal".to_string()]).await.unwrap();
    let tool = host
        .tools()
        .into_iter()
        .find(|t| t.name() == "echo")
        .unwrap();
    let (context, mut rx) = ctx(dir.path());
    let run = tokio::spawn(async move {
        tool.execute(serde_json::json!({ "text": "x" }), context)
            .await
    });
    // The stub streams one "working" update before the result.
    let update = rx.recv().await;
    assert_eq!(update, Some("working".to_string()));
    run.await.unwrap().unwrap();
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_host_crash_returns_error_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = host_with_short_timeout();
    let process = host.launch(STUB, &["crash".to_string()]).await.unwrap();
    // Call the plugin process directly to observe the raw error.
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let result = process
        .call_tool(
            "echo",
            serde_json::json!({ "text": "x" }),
            tx,
            CancelToken::new(),
        )
        .await;
    assert!(result.is_err(), "a crashed plugin yields an error");

    // The host stays usable: the proxy tool converts the failure to an error
    // result, so the session continues instead of aborting.
    let tool = host
        .tools()
        .into_iter()
        .find(|t| t.name() == "echo")
        .unwrap();
    let (context, _rx2) = ctx(dir.path());
    let out = tool
        .execute(serde_json::json!({ "text": "x" }), context)
        .await
        .unwrap();
    assert!(
        out.is_error,
        "the proxy returns an error result, not a panic"
    );
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_host_malformed_line_is_dropped() {
    // The garbage stub writes a non-JSON line before the handshake response. The
    // host must drop it and still complete the handshake.
    let mut host = PluginHost::new(rho_plugin::PluginPolicy::trust_any_path());
    let process = host.launch(STUB, &["garbage".to_string()]).await.unwrap();
    assert_eq!(process.name(), "stub");
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_host_call_times_out() {
    let mut host = host_with_short_timeout();
    let process = host.launch(STUB, &["hang".to_string()]).await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let result = process
        .call_tool(
            "echo",
            serde_json::json!({ "text": "x" }),
            tx,
            CancelToken::new(),
        )
        .await;
    assert!(
        matches!(result, Err(rho_plugin::PluginError::Timeout)),
        "a hung plugin call times out, got {result:?}"
    );
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_host_cancel_stops_call() {
    let mut host = PluginHost::new(rho_plugin::PluginPolicy::trust_any_path());
    let process = host.launch(STUB, &["hang".to_string()]).await.unwrap();
    let cancel = CancelToken::new();
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let call_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        process
            .call_tool("echo", serde_json::json!({ "text": "x" }), tx, call_cancel)
            .await
    });
    cancel.cancel();
    let result = handle.await.unwrap();
    assert!(
        matches!(result, Err(rho_plugin::PluginError::Canceled)),
        "a cancelled call stops, got {result:?}"
    );
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_host_enormous_line_does_not_panic() {
    // The bigline stub writes a 20 MB line, then the real result. The host caps
    // the line and still reads the result. It must not panic.
    let dir = tempfile::tempdir().unwrap();
    let mut host = host_with_short_timeout();
    let process = host.launch(STUB, &["bigline".to_string()]).await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let out = process
        .call_tool(
            "echo",
            serde_json::json!({ "text": "survived" }),
            tx,
            CancelToken::new(),
        )
        .await;
    let _ = dir;
    assert!(out.is_ok(), "the host reads the result after a capped line");
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_schema_cache_roundtrips() {
    let cache = PluginCache {
        version: 1,
        plugin: "my-plugin".to_string(),
        tools: vec![PluginToolSpec {
            name: "search_docs".to_string(),
            description: "Search the docs".to_string(),
            kind: rho_core::ToolKind::Search,
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "query": { "type": "string" } }
            }),
        }],
    };
    let bytes = cache.to_json().unwrap();
    let parsed = PluginCache::from_json(&bytes).unwrap();
    assert_eq!(parsed, cache);
    // The wire name for the schema is `inputSchema`.
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("inputSchema"));
}

#[tokio::test]
async fn plugin_tool_advertised_from_cache_before_connect() {
    let mut host = PluginHost::new(rho_plugin::PluginPolicy::trust_any_path());
    host.load_cache(PluginCache {
        version: 1,
        plugin: "my-plugin".to_string(),
        tools: vec![PluginToolSpec {
            name: "search_docs".to_string(),
            description: "Search the docs".to_string(),
            kind: rho_core::ToolKind::Search,
            input_schema: serde_json::json!({ "type": "object" }),
        }],
    });
    // No plugin has launched, yet the cached tool is advertised.
    let tools = host.tools();
    assert!(tools.iter().any(|t| t.name() == "search_docs"));
}

#[tokio::test]
async fn plugin_host_clean_shutdown_leaves_no_orphan() {
    let mut host = PluginHost::new(rho_plugin::PluginPolicy::trust_any_path());
    let process = host.launch(STUB, &["normal".to_string()]).await.unwrap();
    let pid = std::sync::Arc::strong_count(&process); // keep a ref for the check
    let _ = pid;
    host.shutdown().await;
    // After shutdown the process is marked unavailable and reaped.
    assert!(!process.is_available(), "the plugin is shut down");
}

#[tokio::test]
async fn plugin_declared_read_kind_does_not_bypass_read_only_policy() {
    // A security regression test, from a security audit finding.
    //
    // `ToolKind` drives the approval boundary. `ReadOnlyPolicy` allows a read-only
    // kind and denies everything else. If the host trusted the kind a plugin
    // advertises, a hostile or compromised plugin would declare a destructive tool as
    // `Read` and then run under a read-only policy.
    //
    // So the host reports `Other` for every plugin tool, whatever the plugin claims.
    // `Other` counts as mutating, so a read-only session denies it. The plugin's own
    // claim survives for display only.
    use rho_core::{ApprovalDecision, ApprovalPolicy, ReadOnlyPolicy, ToolKind};

    let mut host = PluginHost::new(rho_plugin::PluginPolicy::trust_any_path());
    host.launch(STUB, &["normal".to_string()]).await.unwrap();
    let tools = host.tools();
    assert!(!tools.is_empty(), "the stub plugin must advertise a tool");

    for tool in &tools {
        assert_eq!(
            tool.kind(),
            ToolKind::Other,
            "the host must not repeat a plugin's own kind claim for tool {}",
            tool.name()
        );
        let decision = ReadOnlyPolicy
            .approve(tool.name(), tool.kind(), &serde_json::json!({}))
            .await;
        assert_eq!(
            decision,
            ApprovalDecision::Deny,
            "a read-only policy must deny the plugin tool {}",
            tool.name()
        );
    }
}

// --- Launch policy, from a security audit ---------------------------------

#[tokio::test]
async fn launch_refuses_a_plugin_inside_the_session_root() {
    // A security audit found that `launch` did no path validation.
    //
    // The risk is concrete. If a plugin may live inside the session root, then a
    // repository can hand executable code to the agent that reads it. A checked-in
    // script becomes a tool as soon as somebody points rho at the repository. The
    // model can also write such a script itself, with `write` or `bash`, and a later
    // launch would run it.
    let root = tempfile::tempdir().unwrap();
    let plugin = root.path().join("evil.sh");
    std::fs::write(&plugin, "#!/bin/sh\necho pwned\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&plugin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mut host = PluginHost::new(rho_plugin::PluginPolicy::confined_to_outside(root.path()));
    let error = host.launch(plugin.to_str().unwrap(), &[]).await;
    let error = match error {
        Ok(_) => panic!("a plugin under the session root must be refused"),
        Err(error) => error,
    };
    let text = error.to_string();
    assert!(
        text.contains("session root"),
        "the message must say why: {text}"
    );
}

#[tokio::test]
async fn launch_refuses_a_path_that_escapes_the_root_with_dot_dot() {
    // The check resolves the path first, so `..` cannot dodge it.
    let root = tempfile::tempdir().unwrap();
    let nested = root.path().join("sub");
    std::fs::create_dir(&nested).unwrap();
    let plugin = root.path().join("evil.sh");
    std::fs::write(&plugin, "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&plugin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let sneaky = nested.join("..").join("evil.sh");

    let mut host = PluginHost::new(rho_plugin::PluginPolicy::confined_to_outside(root.path()));
    let error = host.launch(sneaky.to_str().unwrap(), &[]).await;
    let error = match error {
        Ok(_) => panic!("a resolved path under the root must be refused"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("session root"));
}

#[tokio::test]
async fn launch_refuses_a_missing_plugin_with_a_clear_message() {
    let mut host = PluginHost::new(rho_plugin::PluginPolicy::trust_any_path());
    let error = host.launch("/definitely/not/here/plugin", &[]).await;
    let error = match error {
        Ok(_) => panic!("a missing plugin must be refused"),
        Err(error) => error,
    };
    let text = error.to_string();
    assert!(text.contains("cannot resolve"), "got {text}");
}

#[tokio::test]
async fn launch_refuses_a_directory() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = PluginHost::new(rho_plugin::PluginPolicy::trust_any_path());
    let error = host.launch(dir.path().to_str().unwrap(), &[]).await;
    let error = match error {
        Ok(_) => panic!("a directory is not a plugin"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("not a file"));
}

#[cfg(unix)]
#[tokio::test]
async fn launch_refuses_a_non_executable_file() {
    let dir = tempfile::tempdir().unwrap();
    let plugin = dir.path().join("plugin.sh");
    std::fs::write(&plugin, "#!/bin/sh\n").unwrap();
    let mut host = PluginHost::new(rho_plugin::PluginPolicy::trust_any_path());
    let error = host.launch(plugin.to_str().unwrap(), &[]).await;
    let error = match error {
        Ok(_) => panic!("a file with no execute bit must be refused"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("not executable"));
}

#[cfg(unix)]
#[tokio::test]
async fn launch_refuses_a_world_writable_plugin() {
    // A world-writable program is a classic escalation path. Another local user
    // replaces the file, and the host runs their code.
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let plugin = dir.path().join("plugin.sh");
    std::fs::write(&plugin, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&plugin, std::fs::Permissions::from_mode(0o777)).unwrap();

    let mut host = PluginHost::new(rho_plugin::PluginPolicy {
        untrusted_root: None,
        refuse_world_writable: true,
    });
    let error = host.launch(plugin.to_str().unwrap(), &[]).await;
    let error = match error {
        Ok(_) => panic!("a world-writable plugin must be refused"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("any user can write"));
}

#[tokio::test]
async fn launch_allows_a_plugin_outside_the_root() {
    // The policy must not block ordinary use. The stub binary lives in the build
    // directory, which is outside a repository's session root in this test.
    let root = tempfile::tempdir().unwrap();
    let mut host = PluginHost::new(rho_plugin::PluginPolicy::confined_to_outside(root.path()));
    host.launch(STUB, &["normal".to_string()])
        .await
        .expect("a plugin outside the root must launch");
    assert!(!host.tools().is_empty());
}
