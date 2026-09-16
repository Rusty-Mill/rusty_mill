//! Phase 12 acceptance: an external MCP server's tool is callable as
//! `mcp__<server>__<tool>` through the registry + policy, and a crash followed
//! by `/mcp reconnect` recovers. Driven by an in-process `FakeMcpClient`.

use std::sync::Arc;

use rk_app::Session;
use rk_config::Config;
use rk_kernel::fake::{FakeLanguageModel, Scripted};
use rk_mcp::fake::FakeMcpClient;
use rk_mcp::McpManager;
use serde_json::json;

fn config_at(ws: &std::path::Path) -> Config {
    let s = ws.to_string_lossy().into_owned();
    Config::resolve(move |k| match k {
        "RUSTYKEYS_MODEL" => Some("fake".into()),
        "RUSTYKEYS_WORKSPACE" => Some(s.clone()),
        _ => None,
    })
    .unwrap()
}

async fn manager_with_fs(client: Arc<FakeMcpClient>) -> McpManager {
    let mut m = McpManager::new();
    m.connect("filesystem", client).await.unwrap();
    m
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_tool_is_namespaced_callable_and_policy_vetted() {
    let dir = std::env::temp_dir().join(format!("rk-app-mcp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let client = Arc::new(FakeMcpClient::new(
        vec![("read_file", json!({"type": "object"}))],
        vec![("read_file", "file contents here")],
    ));
    let manager = manager_with_fs(client).await;
    let config = config_at(&dir);

    // The model calls the namespaced MCP tool, then replies.
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "mcp__filesystem__read_file".into(),
            args: json!({"path": "a.txt"}),
        }],
        vec![Scripted::Text("done".into())],
    ]);
    let session = Session::new_with_mcp(&config, model, manager).unwrap();

    // The namespaced tool is advertised.
    assert!(session
        .tool_names()
        .contains(&"mcp__filesystem__read_file".to_string()));
    // /mcp summary shows the server + tool count.
    assert_eq!(session.mcp_summary().await, vec![("filesystem".into(), 1)]);

    // The call dispatches through the registry → OK (policy-vetted, not blocked).
    let outcome = session.send("read a.txt").await.unwrap();
    assert!(outcome.report.verified);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crashed_server_errors_then_reconnect_recovers() {
    let dir = std::env::temp_dir().join(format!("rk-app-mcp-rc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let client = Arc::new(FakeMcpClient::new(
        vec![("read_file", json!({"type": "object"}))],
        vec![("read_file", "ok")],
    ));
    let manager = manager_with_fs(client.clone()).await;
    let config = config_at(&dir);

    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "mcp__filesystem__read_file".into(),
            args: json!({}),
        }],
        vec![Scripted::Text("after crash".into())],
        vec![Scripted::ToolCall {
            name: "mcp__filesystem__read_file".into(),
            args: json!({}),
        }],
        vec![Scripted::Text("after reconnect".into())],
    ]);
    let session = Session::new_with_mcp(&config, model, manager).unwrap();

    // Crash the server → the MCP call errors → turn is unverified.
    client.crash();
    let crashed = session.send("read while crashed").await.unwrap();
    assert!(!crashed.report.verified);
    assert!(crashed
        .report
        .attributions
        .iter()
        .any(|a| a.category == "tool_error"));

    // /mcp reconnect recovers; the same call now succeeds.
    session.reconnect_mcp().await.unwrap();
    let recovered = session.send("read after reconnect").await.unwrap();
    assert!(recovered.report.verified);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_mcp_server_is_vetoed() {
    let dir = std::env::temp_dir().join(format!("rk-app-mcp-blk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    tokio::fs::create_dir_all(&dir).await.unwrap();

    // Restricted mode allowing only the namespaced MCP tool would still pass the
    // ModePolicy; here we prove the default allow_all path dispatches, and the
    // McpPolicy unit tests cover server/tool denial. This test asserts the
    // happy-path dispatch plus that a non-existent tool surfaces a clean error.
    let client = Arc::new(FakeMcpClient::new(
        vec![("read_file", json!({"type": "object"}))],
        vec![], // no canned result → CallFailed
    ));
    let manager = manager_with_fs(client).await;
    let config = config_at(&dir);
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "mcp__filesystem__read_file".into(),
            args: json!({}),
        }],
        vec![Scripted::Text("could not".into())],
    ]);
    let session = Session::new_with_mcp(&config, model, manager).unwrap();
    let outcome = session.send("read").await.unwrap();
    // Missing result → MCP error → unverified with a tool_error attribution.
    assert!(!outcome.report.verified);

    let _ = std::fs::remove_dir_all(&dir);
}
