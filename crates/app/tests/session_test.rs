//! Phase-1 DoD: a FakeLanguageModel integration test drives a full
//! `Session::send`, exercising the real tool registry + workspace policy +
//! aisdk loop with no live provider.

use rk_app::Session;
use rk_config::Config;
use rk_kernel::fake::{FakeLanguageModel, Scripted};
use serde_json::json;

fn config_at(workspace: &std::path::Path) -> Config {
    let ws = workspace.to_string_lossy().into_owned();
    Config::resolve(|k| match k {
        "RUSTYKEYS_MODEL" => Some("fake".into()),
        "RUSTYKEYS_WORKSPACE" => Some(ws.clone()),
        _ => None,
    })
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_reads_workspace_file_then_replies() {
    let dir = std::env::temp_dir().join(format!("rk-app-{}", std::process::id()));
    tokio::fs::create_dir_all(&dir).await.unwrap();
    tokio::fs::write(dir.join("note.txt"), "hello from the workspace")
        .await
        .unwrap();

    let config = config_at(&dir);
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "read_file".into(),
            args: json!({"path": "note.txt"}),
        }],
        vec![Scripted::Text("I read the note.".into())],
    ]);

    let session = Session::new(&config, model);
    assert_eq!(session.tool_names(), vec!["list_directory", "read_file"]);

    let reply = session.send("read note.txt").await.unwrap();
    assert_eq!(reply, "I read the note.");

    tokio::fs::remove_dir_all(&dir).await.ok();
}
