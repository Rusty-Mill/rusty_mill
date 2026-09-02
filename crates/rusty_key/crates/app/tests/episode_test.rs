//! Phase 10 DoD: at H3 a bug-fix turn writes a complete eight-trace episode
//! package with reproduction + attribution + verification linked to requirement
//! ids; `action_trace` is populated and distinct from `tool_trace`; the outcome
//! classifier lands on `autonomous_verified_success`. Deterministic replay.

use rk_app::Session;
use rk_config::Config;
use rk_kernel::fake::{FakeLanguageModel, Scripted};
use serde_json::{json, Value};

fn h3_config(workspace: &std::path::Path) -> Config {
    let ws = workspace.to_string_lossy().into_owned();
    Config::resolve(move |k| match k {
        "RUSTYKEYS_MODEL" => Some("fake".into()),
        "RUSTYKEYS_WORKSPACE" => Some(ws.clone()),
        "RUSTYKEYS_HARNESS_LEVEL" => Some("h3".into()),
        _ => None,
    })
    .unwrap()
}

fn read_only_package(dir: &std::path::Path) -> Value {
    let episodes = dir.join(".rustykeys").join("episodes");
    let entry = std::fs::read_dir(&episodes)
        .expect("episodes dir exists")
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().is_some_and(|x| x == "json"))
        .expect("one episode package");
    let body = std::fs::read_to_string(entry.path()).unwrap();
    serde_json::from_str(&body).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h3_bug_fix_turn_writes_complete_episode_package() {
    let dir = std::env::temp_dir().join(format!("rk-app-h3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let config = h3_config(&dir);
    // A canonical bug-fix turn: reproduce → edit → verification_report → reply.
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "reproduce".into(),
            args: json!({"check": "empty_password_probe", "observed": "panics", "expected": "rejects"}),
        }],
        vec![Scripted::ToolCall {
            name: "write_file".into(),
            args: json!({"path": "auth.rs", "content": "fn check() { /* fixed */ }"}),
        }],
        vec![Scripted::ToolCall {
            name: "verification_report".into(),
            args: json!({"requirements": [{"requirement": "req-1", "met": true, "evidence": "probe now rejects"}]}),
        }],
        vec![Scripted::Text("fixed the empty-password panic".into())],
    ]);

    let session = Session::new(&config, model).unwrap();
    let outcome = session.send("fix the empty password bug").await.unwrap();
    assert!(outcome.report.verified);

    let pkg = read_only_package(&dir);
    assert_eq!(pkg["schema_version"], 1);
    assert_eq!(pkg["harness_level"], "h3");
    assert!(pkg["episode_id"].as_str().unwrap().starts_with("ep_"));
    assert_eq!(pkg["outcome"], "autonomous_verified_success");

    // Reproduction recorded.
    assert_eq!(pkg["reproduction_log"]["check"], "empty_password_probe");

    // Verification linked to requirement ids.
    let reqs = pkg["verification_report"]["requirements"]
        .as_array()
        .unwrap();
    assert_eq!(reqs[0]["requirement"], "req-1");
    assert_eq!(reqs[0]["met"], true);

    // action_trace populated and distinct from tool_trace.
    let actions = pkg["action_trace"].as_array().unwrap();
    let tools = pkg["tool_trace"].as_array().unwrap();
    assert!(!actions.is_empty());
    // 3 tool calls (reproduce, edit_file, verification_report); reproduce is not
    // an action, so action_trace (edit_file, write_report) is shorter + distinct.
    assert_eq!(tools.len(), 3);
    assert_eq!(actions.len(), 2);
    let ops: Vec<&str> = actions.iter().map(|a| a["op"].as_str().unwrap()).collect();
    assert!(ops.contains(&"edit_file"));
    assert!(ops.contains(&"write_report"));

    // The verification_trace covers every check (incl. the H3 process checks).
    let vt = pkg["verification_trace"].as_array().unwrap();
    assert!(vt.iter().any(|v| v["method"] == "bug_reproduction"));
    assert!(vt.iter().any(|v| v["method"] == "patch_review"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registered_checks_drive_verification_trace_and_verdict() {
    let dir = std::env::temp_dir().join(format!("rk-app-h3-checks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    tokio::fs::create_dir_all(dir.join(".rustykeys"))
        .await
        .unwrap();
    // A registered check that fails (output won't contain the expected string).
    std::fs::write(
        dir.join(".rustykeys/checks.toml"),
        "[[check]]\nname = \"unit\"\ncommand = \"echo broken\"\nexpected_substring = \"all green\"\ncovers = [\"req-9\"]\nmethod = \"registered_test\"\n",
    )
    .unwrap();

    let config = h3_config(&dir);
    // reproduce → write_file → verification_report → reply: the H3 spine passes,
    // but the registered check fails, so the turn is UNVERIFIED.
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "reproduce".into(),
            args: json!({"check": "probe", "observed": "x", "expected": "y"}),
        }],
        vec![Scripted::ToolCall {
            name: "write_file".into(),
            args: json!({"path": "a.rs", "content": "fn f() {}"}),
        }],
        vec![Scripted::ToolCall {
            name: "verification_report".into(),
            args: json!({"requirements": [{"requirement": "req-9", "met": true, "evidence": "e"}]}),
        }],
        vec![Scripted::Text("done".into())],
    ]);
    let session = Session::new(&config, model).unwrap();
    let outcome = session.send("fix it").await.unwrap();

    // The failing registered check makes the turn unverified + attributed.
    assert!(!outcome.report.verified);
    assert!(outcome
        .report
        .attributions
        .iter()
        .any(|a| a.category == "registered_check_failed"));

    let pkg = read_only_package(&dir);
    assert_eq!(pkg["outcome"], "failed");
    // The registered check appears in verification_trace with its method + covers.
    let vt = pkg["verification_trace"].as_array().unwrap();
    let reg = vt
        .iter()
        .find(|v| v["method"] == "registered_test")
        .expect("registered check in verification_trace");
    assert_eq!(reg["result"], "fail");
    assert_eq!(reg["covers"][0], "req-9");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h3_edit_without_reproduce_is_unverified_and_attributed() {
    let dir = std::env::temp_dir().join(format!("rk-app-h3-fail-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let config = h3_config(&dir);
    // Edits a file without reproducing first → reproduce_before_edit fails.
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "edit_file".into(),
            args: json!({"path": "auth.rs", "content": "fn check() {}"}),
        }],
        vec![Scripted::Text("changed it".into())],
    ]);

    let session = Session::new(&config, model).unwrap();
    let outcome = session.send("just fix it").await.unwrap();
    assert!(!outcome.report.verified);
    // The H3 discipline failure is attributed.
    assert!(outcome
        .report
        .attributions
        .iter()
        .any(|a| a.category == "reproduction_skipped"));

    let pkg = read_only_package(&dir);
    assert_eq!(pkg["outcome"], "failed");

    let _ = std::fs::remove_dir_all(&dir);
}
