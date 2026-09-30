//! End-to-end: a real replay through the pipeline into stored history and out
//! as a habit report — the same path `POST /api/analyze` takes with `--data-dir`.
//! Run against every storage backend, so the engine store is held to the same
//! contract as the JSON files.

use std::path::PathBuf;

use rleval_app::history::{habits, summarize, SessionRecord};
use replay_scoring::XgModel;
use rleval_app::pipeline;
use rleval_app::store::{session_key, AccountId, FsSessionStore, SaveOutcome, SessionStore};

fn analyzed_record() -> SessionRecord {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/replays/42f2.replay");
    let bytes = std::fs::read(&path).expect("sample replay");
    let analysis = pipeline::analyze(&bytes, "42f2", None, None, &XgModel::default()).expect("analyze");

    let record = SessionRecord::from_analysis(session_key(&bytes), 1, &analysis);
    assert_eq!(record.players.len(), analysis.pacifist.players.len());
    assert!(record.players.iter().all(|p| p.dimensions.len() == 8));
    assert!(
        record.players.iter().any(|p| p.platform_id.is_some()),
        "platform ids flow from the replay header into stored history"
    );
    assert!(
        record.players.iter().any(|p| p.won == Some(true))
            && record.players.iter().any(|p| p.won == Some(false)),
        "a decided match has winners and losers"
    );
    record
}

/// Save → idempotent re-save → list → summarize → habits, against `store`.
fn round_trip(store: &dyn SessionStore, record: &SessionRecord) {
    let me = AccountId::new("me").unwrap();
    assert_eq!(store.save(&me, record).unwrap(), SaveOutcome::Saved);
    assert_eq!(store.save(&me, record).unwrap(), SaveOutcome::AlreadyStored);

    let stored = store.list(&me).unwrap();
    assert_eq!(
        stored,
        vec![record.clone()],
        "what comes out is what went in"
    );
    assert_eq!(summarize(&stored).sessions.len(), 1);

    let key = &summarize(&stored).players[0].key;
    let report = habits(key, &stored).expect("player is in history");
    assert_eq!(report.matches, 1);
    // One match can't claim a win/loss habit — it falls back to weakest dimension.
    if let Some(f) = report.focus {
        assert_eq!(f.gap, None);
    }
}

fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rleval-flow-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn analysis_round_trips_through_the_json_store() {
    let root = temp_root("fs");
    round_trip(&FsSessionStore::new(&root), &analyzed_record());
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(feature = "mmdb")]
#[test]
fn analysis_round_trips_through_the_engine_store() {
    let root = temp_root("mmdb");
    {
        let store = rleval_app::store_mmdb::MmdbSessionStore::new(&root);
        round_trip(&store, &analyzed_record());
    } // dropped: releases the directory lock
      // ...and the session is still there after a reopen.
    let reopened = rleval_app::store_mmdb::MmdbSessionStore::new(&root);
    let me = AccountId::new("me").unwrap();
    assert_eq!(reopened.list(&me).unwrap().len(), 1);
    drop(reopened);
    let _ = std::fs::remove_dir_all(root);
}
