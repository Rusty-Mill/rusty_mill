//! Integration test: a scripted model drives a full policy-vetted tool loop
//! with no live provider (testing-strategy.md: every LLM path testable in CI).

use std::sync::Arc;

use rk_spike::fake::FakeChatModel;
use rk_spike::kernel::{run_turn, ChatMessage, ModelStep};
use rk_spike::outcome::{ToolOutcome, ToolStatus};
use rk_spike::policy::WorkspacePolicy;
use rk_spike::tool::{register_builtins, ToolDispatch, ToolRegistry};
use serde_json::json;

fn registry_at(root: &std::path::Path) -> ToolRegistry {
    let policy = Arc::new(WorkspacePolicy::new(root.to_path_buf()));
    let mut registry = ToolRegistry::new(policy);
    register_builtins(&mut registry);
    registry
}

#[tokio::test]
async fn full_turn_reads_in_workspace_file() {
    let dir = std::env::temp_dir().join(format!("rk-spike-{}", std::process::id()));
    tokio::fs::create_dir_all(&dir).await.unwrap();
    tokio::fs::write(dir.join("hello.txt"), "hi there").await.unwrap();

    let registry = registry_at(&dir);
    let abs = dir.join("hello.txt").to_string_lossy().into_owned();
    let model = FakeChatModel::new(vec![
        vec![ModelStep::ToolCall {
            name: "read_file".into(),
            args: json!({ "path": abs }),
        }],
        vec![ModelStep::Text("done".into())],
    ]);

    let mut history = vec![ChatMessage::User("read it".into())];
    let reply = run_turn(&model, &registry, "sys", &mut history).await.unwrap();

    assert_eq!(reply, "done");
    let tool_result = history.iter().find_map(|m| match m {
        ChatMessage::ToolResult { content, .. } => Some(content.clone()),
        _ => None,
    });
    assert_eq!(tool_result.as_deref(), Some("hi there"));

    tokio::fs::remove_dir_all(&dir).await.ok();
}

#[tokio::test]
async fn policy_blocks_out_of_workspace_path_without_running_tool() {
    let dir = std::env::temp_dir().join(format!("rk-spike-block-{}", std::process::id()));
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let registry = registry_at(&dir);

    let outcome = registry.dispatch("read_file", json!({ "path": "/etc/passwd" })).await;
    assert_eq!(outcome.status, ToolStatus::Blocked);
    assert!(outcome.payload.contains("outside the workspace"));

    tokio::fs::remove_dir_all(&dir).await.ok();
}

#[tokio::test]
async fn unknown_tool_is_an_error_not_a_panic() {
    let dir = std::env::temp_dir();
    let registry = registry_at(&dir);
    let outcome = registry.dispatch("no_such_tool", json!({})).await;
    assert_eq!(outcome.status, ToolStatus::Error);
}

#[test]
fn outcome_render_carries_status_structurally() {
    assert_eq!(ToolOutcome::ok("payload").render(), "payload");
    assert_eq!(ToolOutcome::blocked("nope").render(), "[blocked] nope");
}

#[tokio::test]
async fn builtin_schemas_are_advertised() {
    let registry = registry_at(&std::env::temp_dir());
    let names: Vec<_> = registry.schemas().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, vec!["list_directory".to_string(), "read_file".to_string()]);
}
