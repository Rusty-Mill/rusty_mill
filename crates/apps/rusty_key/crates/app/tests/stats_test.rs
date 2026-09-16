//! Phase 13: `/stats` data — `Session::stats()` reflects turns, cumulative tool
//! calls, and the entropy delta after real turns.

use rk_app::Session;
use rk_config::Config;
use rk_kernel::fake::{FakeLanguageModel, Scripted};
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stats_track_turns_and_tool_calls() {
    let dir = std::env::temp_dir().join(format!("rk-app-stats-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    tokio::fs::create_dir_all(&dir).await.unwrap();
    tokio::fs::write(dir.join("a.txt"), "hi").await.unwrap();

    let config = config_at(&dir);
    // Turn 1: one tool call + reply. Turn 2: a plain reply.
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "read_file".into(),
            args: json!({"path": "a.txt"}),
        }],
        vec![Scripted::Text("read it".into())],
        vec![Scripted::Text("nothing else".into())],
    ]);
    let session = Session::new(&config, model).unwrap();

    session.send("read a.txt").await.unwrap();
    session.send("anything else?").await.unwrap();

    let s = session.stats();
    assert_eq!(s.turns, 2);
    assert_eq!(s.tool_calls, 1); // one read_file across the two turns
    assert_eq!(s.tokens_limit, 200_000); // default context window
    assert_eq!(s.entropy_delta, 0); // no burdensome edits

    let _ = std::fs::remove_dir_all(&dir);
}
