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

    let session = Session::new(&config, model).unwrap();
    assert_eq!(
        session.tool_names(),
        vec![
            "agent",
            "bash",
            "complete_task",
            "edit_file",
            "glob",
            "grep",
            "list_directory",
            "read_file",
            "set_task",
            "task_create",
            "task_get",
            "task_list",
            "task_output",
            "task_stop",
            "task_update",
            "write_file"
        ]
    );

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
async fn agent_tool_spawns_a_child_session() {
    let dir = std::env::temp_dir().join(format!("rk-app-agent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let config = config_at(&dir);

    // Shared script (FakeLanguageModel clones share the queue): parent calls the
    // agent tool; the child turn pops "child result"; the parent then finishes.
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "agent".into(),
            args: json!({"task": "do the subtask"}),
        }],
        vec![Scripted::Text("child result".into())],
        vec![Scripted::Text("parent done".into())],
    ]);
    let session = Session::new(&config, model).unwrap();
    let outcome = session.send("delegate this").await.unwrap();
    assert_eq!(outcome.reply, "parent done");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn destructive_bash_is_blocked_by_bashguard() {
    let dir = std::env::temp_dir().join(format!("rk-app-bash-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let config = config_at(&dir);
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "bash".into(),
            args: json!({"command": "rm -rf / --no-preserve-root"}),
        }],
        vec![Scripted::Text("could not".into())],
    ]);
    let session = Session::new(&config, model).unwrap();
    let outcome = session.send("delete everything").await.unwrap();

    // BashGuard blocks it → UNVERIFIED with a permission_block attribution.
    assert!(!outcome.report.verified);
    assert!(outcome
        .report
        .attributions
        .iter()
        .any(|a| a.category == "permission_block"));

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

    let session = Session::new(&config, model).unwrap();
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_only_mode_blocks_a_write_turn() {
    let dir = std::env::temp_dir().join(format!("rk-app-readonly-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let ws = dir.to_string_lossy().into_owned();
    let config = Config::resolve(|k| match k {
        "RUSTYKEYS_MODEL" => Some("fake".into()),
        "RUSTYKEYS_WORKSPACE" => Some(ws.clone()),
        "RUSTYKEYS_PERMISSION_MODE" => Some("read_only".into()),
        _ => None,
    })
    .unwrap();

    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "write_file".into(),
            args: json!({"path": "out.txt", "content": "nope"}),
        }],
        vec![Scripted::Text("could not write".into())],
    ]);

    let session = Session::new(&config, model).unwrap();
    assert_eq!(session.permission_mode(), "read_only");
    let outcome = session.send("write a file").await.unwrap();

    // The mode gate blocks the write → UNVERIFIED with a permission_block attribution.
    assert!(!outcome.report.verified);
    let a = outcome
        .report
        .attributions
        .iter()
        .find(|a| a.category == "permission_block")
        .expect("permission_block attribution");
    assert_eq!(a.layer, "constrain/policy");
    // The write never happened.
    assert!(!dir.join("out.txt").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn followup_after_unverified_turn_counts_toward_mhir() {
    let dir = std::env::temp_dir().join(format!("rk-app-mhir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let config = config_at(&dir);
    // Turn 1: a blocked read → UNVERIFIED. Turn 2: a clean text reply (the
    // follow-up) → records an unverified_followup intervention against turn 2.
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "read_file".into(),
            args: json!({"path": "/etc/shadow"}),
        }],
        vec![Scripted::Text("blocked".into())],
        vec![Scripted::Text("ok now".into())],
    ]);

    let session = Session::new(&config, model).unwrap();
    let first = session.send("read it").await.unwrap();
    assert!(!first.report.verified);

    let second = session.send("never mind, summarize").await.unwrap();
    assert!(second.report.verified);

    // 2 turns journaled; 1 avoidable (unverified_followup) intervention.
    let m = session.mhir().unwrap();
    assert_eq!(m.n_turns, 2);
    assert_eq!(m.n_interventions, 1);
    assert!((m.rate - 0.5).abs() < 1e-9);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_task_tool_updates_task_state_and_persists() {
    let dir = std::env::temp_dir().join(format!("rk-app-task-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let config = config_at(&dir);

    // The model drives set_task, then replies.
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "set_task".into(),
            args: json!({"goal": "add empty-password validation", "success_criteria": ["rejects empty password"]}),
        }],
        vec![Scripted::Text("task noted".into())],
    ]);
    let session = Session::new(&config, model).unwrap();
    session.send("please set up the task").await.unwrap();

    let t = session.task_state();
    assert_eq!(t.goal, "add empty-password validation");
    assert_eq!(
        t.success_criteria,
        vec!["rejects empty password".to_string()]
    );
    // Persisted to task.json.
    let json = std::fs::read_to_string(dir.join(".rustykeys/task.json")).unwrap();
    assert!(json.contains("add empty-password validation"));

    let _ = std::fs::remove_dir_all(&dir);
}

async fn judged_turn(dir_tag: &str, judge_emit: &str) -> rk_compose::VerificationReport {
    let dir = std::env::temp_dir().join(format!("rk-app-judge-{dir_tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let config = config_at(&dir);

    // The reply turn (1 entry), then the judge call consumes the next entry.
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::Text("I added the validation.".into())],
        vec![Scripted::Text(judge_emit.into())],
    ]);
    let session = Session::new(&config, model).unwrap();
    session.set_task(
        "add validation",
        vec!["adds a unit test".into()],
        Vec::new(),
    );

    let outcome = session.send("do it").await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    outcome.report
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn criteria_judge_pass_verifies_and_fail_attributes_criteria_unmet() {
    // Judge passes → verified, judge_ran.
    let pass = judged_turn("pass", r#"{"verdict":"pass","criteria":[{"criterion":"adds a unit test","met":true,"reason":"test added"}]}"#).await;
    assert!(pass.verified);
    assert!(pass.judge_ran);
    assert!(pass
        .checks
        .iter()
        .any(|c| c.name == "criteria_judge" && c.passed));

    // Judge fails → unverified with criteria_unmet → f_model @ compose/semantic.
    let fail = judged_turn("fail", r#"{"verdict":"fail","criteria":[{"criterion":"adds a unit test","met":false,"reason":"no test"}]}"#).await;
    assert!(!fail.verified);
    let a = fail
        .attributions
        .iter()
        .find(|a| a.category == "criteria_unmet")
        .unwrap();
    assert_eq!(a.layer, "compose/semantic");
    assert_eq!(
        serde_json::to_value(a.failure_type).unwrap(),
        json!("f_model")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unparseable_judge_is_unavailable_not_a_pass() {
    let report = judged_turn("unavail", "the criteria look fine to me").await;
    assert!(
        !report.verified,
        "an unavailable judge must never read as verified"
    );
    assert!(!report.judge_ran);
    let a = report
        .attributions
        .iter()
        .find(|a| a.category == "judge_unavailable")
        .unwrap();
    assert_eq!(
        serde_json::to_value(a.failure_type).unwrap(),
        json!("f_verify")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn semantic_recall_matches_a_paraphrase() {
    use std::sync::Arc;
    let dir = std::env::temp_dir().join(format!("rk-app-sem-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let config = config_at(&dir);
    let embedder = Arc::new(rk_feed::HashEmbedder::new(128));

    // Plant a fact via /reflect — it is embedded on the way into the store.
    let emit = r#"{"memories":[{"op":"create","type":"fact","title":"build system",
        "body":"compile the project using cargo","importance":0.6}]}"#;
    let model_a = FakeLanguageModel::new(vec![vec![Scripted::Text(emit.into())]]);
    let a = Session::new(&config, model_a)
        .unwrap()
        .with_embedder(embedder.clone());
    assert_eq!(a.reflect().await.unwrap().created, 1);
    drop(a);

    // A paraphrase that shares no distinctive keyword with the title is recalled
    // via embedding cosine.
    let b = Session::new(&config, FakeLanguageModel::new(vec![]))
        .unwrap()
        .with_embedder(embedder);
    let block = b
        .recall_block("how do I compile this project with cargo")
        .await
        .unwrap();
    assert!(
        block.contains("build system"),
        "semantic recall missed the paraphrase: {block:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fact_taught_in_session_a_is_recalled_in_session_b() {
    let dir = std::env::temp_dir().join(format!("rk-app-recall-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let config = config_at(&dir);

    // Session A: /reflect consolidates a planted fact (the fake model "emits"
    // the consolidation JSON) into the shared long-term store.
    let emit = r#"{"memories":[{"op":"create","type":"fact","title":"build tool",
        "body":"the project builds with cargo zzz","importance":0.6}]}"#;
    let model_a = FakeLanguageModel::new(vec![vec![Scripted::Text(emit.into())]]);
    let a = Session::new(&config, model_a).unwrap();
    let stats = a.reflect().await.unwrap();
    assert_eq!(stats.created, 1);
    drop(a);

    // Session B: a fresh session over the SAME workspace recalls the fact.
    let model_b = FakeLanguageModel::new(vec![]);
    let b = Session::new(&config, model_b).unwrap();
    let block = b.recall_block("cargo build").await.unwrap();
    assert!(
        block.contains("build tool"),
        "expected recall to surface the planted fact, got: {block:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
