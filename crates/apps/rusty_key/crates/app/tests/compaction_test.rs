//! Phase 8 DoD: the line-item token budget drives 3-tier compaction; every
//! compaction is journaled (`kind: "compaction"`) and the active task — which
//! lives in the `TaskStore`, not the transcript — survives every tier.

use rk_app::Session;
use rk_config::Config;
use rk_kernel::fake::{FakeLanguageModel, Scripted};

/// Config with a tiny context window and thresholds pinned so exactly one tier
/// fires every turn (others disabled with a >1.0 fraction).
fn config_tiers(workspace: &std::path::Path, micro: f64, session: f64, full: f64) -> Config {
    let ws = workspace.to_string_lossy().into_owned();
    let micro = micro.to_string();
    let session = session.to_string();
    let full = full.to_string();
    Config::resolve(move |k| match k {
        "RUSTYKEYS_MODEL" => Some("fake".into()),
        "RUSTYKEYS_WORKSPACE" => Some(ws.clone()),
        "RUSTYKEYS_CONTEXT_LIMIT" => Some("200".into()),
        "RUSTYKEYS_COMPACT_MICRO" => Some(micro.clone()),
        "RUSTYKEYS_COMPACT_SESSION" => Some(session.clone()),
        "RUSTYKEYS_COMPACT_FULL" => Some(full.clone()),
        _ => None,
    })
    .unwrap()
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("rk-compact-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn compaction_records(dir: &std::path::Path) -> Vec<serde_json::Value> {
    let path = dir.join(".rustykeys").join("evidence.jsonl");
    let body = std::fs::read_to_string(path).unwrap_or_default();
    body.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v.get("kind").and_then(|k| k.as_str()) == Some("compaction"))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn micro_tier_drops_turn_pairs_and_journals() {
    let dir = tmp("micro");
    // Micro always fires; session/full never.
    let config = config_tiers(&dir, 0.0, 1000.0, 1000.0);
    // Eight plain-text turns — no tool calls, no summary calls.
    let turns: Vec<Vec<Scripted>> = (0..8)
        .map(|i| vec![Scripted::Text(format!("reply {i}"))])
        .collect();
    let session = Session::new(&config, FakeLanguageModel::new(turns)).unwrap();
    session.set_task("keep me", vec![], vec![]);

    for i in 0..8 {
        session
            .send(&format!("message number {i} with some length"))
            .await
            .unwrap();
    }

    let recs = compaction_records(&dir);
    assert!(!recs.is_empty(), "micro compactions should be journaled");
    assert!(recs.iter().all(|r| r["tier"] == "micro"));
    assert!(recs.iter().any(|r| r["dropped"].as_u64().unwrap_or(0) > 0));
    // The active task is never part of the transcript, so it survives.
    assert_eq!(session.task_state().goal, "keep me");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_tier_summarizes_oldest_half() {
    let dir = tmp("session");
    // Session always fires (once there are >=2 messages); micro/full never.
    let config = config_tiers(&dir, 1000.0, 0.0, 1000.0);
    // Turn 1: just a reply (history has 1 msg → half==0 → no compaction yet).
    // Turns 2+: a summary call THEN the reply.
    let mut turns: Vec<Vec<Scripted>> = vec![vec![Scripted::Text("reply 0".into())]];
    for i in 1..4 {
        turns.push(vec![Scripted::Text(format!("[summary {i}]"))]);
        turns.push(vec![Scripted::Text(format!("reply {i}"))]);
    }
    let session = Session::new(&config, FakeLanguageModel::new(turns)).unwrap();
    session.set_task("survive", vec![], vec![]);

    for i in 0..4 {
        session.send(&format!("turn {i}")).await.unwrap();
    }

    let recs = compaction_records(&dir);
    assert!(!recs.is_empty(), "session compactions should be journaled");
    assert!(recs.iter().all(|r| r["tier"] == "session"));
    assert!(recs
        .iter()
        .all(|r| r["summarized"].as_u64().unwrap_or(0) > 0));
    assert_eq!(session.task_state().goal, "survive");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_tier_resets_history_to_summary() {
    let dir = tmp("full");
    // Full always fires; each turn = summary call then reply.
    let config = config_tiers(&dir, 1000.0, 1000.0, 0.0);
    let mut turns: Vec<Vec<Scripted>> = Vec::new();
    for i in 0..3 {
        turns.push(vec![Scripted::Text(format!("[full summary {i}]"))]);
        turns.push(vec![Scripted::Text(format!("reply {i}"))]);
    }
    let session = Session::new(&config, FakeLanguageModel::new(turns)).unwrap();
    session.set_task("persist", vec![], vec![]);

    for i in 0..3 {
        session.send(&format!("turn {i}")).await.unwrap();
    }

    let recs = compaction_records(&dir);
    assert!(!recs.is_empty(), "full compactions should be journaled");
    assert!(recs.iter().all(|r| r["tier"] == "full"));
    // The task outlives a history reset.
    assert_eq!(session.task_state().goal, "persist");
    // /cost reflects accumulated compactions.
    let (_, limit, _, _, compactions) = session.cost();
    assert_eq!(limit, 200);
    assert!(compactions >= 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_compact_collapses_history() {
    let dir = tmp("manual");
    // Default thresholds (large window) → no automatic compaction.
    let ws = dir.to_string_lossy().into_owned();
    let config = Config::resolve(|k| match k {
        "RUSTYKEYS_MODEL" => Some("fake".into()),
        "RUSTYKEYS_WORKSPACE" => Some(ws.clone()),
        _ => None,
    })
    .unwrap();
    // Two plain turns, then a scripted summary for the manual /compact.
    let session = Session::new(
        &config,
        FakeLanguageModel::new(vec![
            vec![Scripted::Text("a".into())],
            vec![Scripted::Text("b".into())],
            vec![Scripted::Text("[manual summary]".into())],
        ]),
    )
    .unwrap();
    session.send("first").await.unwrap();
    session.send("second").await.unwrap();
    assert!(compaction_records(&dir).is_empty());

    let n = session.compact_now().await.unwrap();
    assert!(n > 0);
    let recs = compaction_records(&dir);
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0]["tier"], "full");
}
