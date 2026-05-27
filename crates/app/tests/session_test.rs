//! Phase-1/2 DoD: a FakeLanguageModel integration test drives a full
//! `Session::send` through the real registry + workspace policy + aisdk loop +
//! deterministic verifier + evidence journal, offline.

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
async fn send_reads_workspace_file_then_replies_verified() {
    let dir = std::env::temp_dir().join(format!("rk-app-ok-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
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

    let outcome = session.send("read note.txt").await.unwrap();
    assert_eq!(outcome.reply, "I read the note.");
    // In-workspace read succeeds → deterministic verification passes.
    assert!(outcome.report.verified);

    // The turn was journaled.
    let journal = std::fs::read_to_string(dir.join(".rustykeys/evidence.jsonl")).unwrap();
    assert!(journal.contains("\"kind\":\"turn\""));
    assert!(journal.contains("\"verified\":true"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn out_of_workspace_read_is_blocked_and_unverified() {
    let dir = std::env::temp_dir().join(format!("rk-app-block-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let config = config_at(&dir);
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "read_file".into(),
            args: json!({"path": "/etc/passwd"}),
        }],
        vec![Scripted::Text("could not read it".into())],
    ]);

    let session = Session::new(&config, model);
    let outcome = session.send("read /etc/passwd").await.unwrap();

    // The blocked tool makes the turn UNVERIFIED with a permission_block attribution.
    assert!(!outcome.report.verified);
    let a = outcome
        .report
        .attributions
        .iter()
        .find(|a| a.category == "permission_block")
        .expect("permission_block attribution");
    assert_eq!(a.layer, "constrain/policy");

    let _ = std::fs::remove_dir_all(&dir);
}
