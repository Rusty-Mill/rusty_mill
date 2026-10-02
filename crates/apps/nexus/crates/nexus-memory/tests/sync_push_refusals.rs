//! Push never passes a memory the hub refused (design review 2.7 follow-up).
//!
//! The hub answers a push with the ids it took and those it refused. The
//! client used to ignore that reply and advance its push cursor over the
//! whole page, so a refused memory was never sent again. A refusal for a
//! timestamp too far ahead of the hub's clock did worse: the cursor jumped
//! to that future time and every edit made since sat behind it, unsent.

use std::sync::Arc;

use chrono::{Duration, Utc};
use nexus_memory::core_plugin::{MemoryCorePlugin, HANDLER_SYNC};
use nexus_memory::db::MemoryDb;
use nexus_memory::model::Memory;
use nexus_memory_hub::{AppState, HubStore};
use nexus_plugins::CorePlugin;
use serde_json::{json, Value};

const SECRET: &str = "refusal-secret";
const NODE: &str = "node-a";

/// An in-memory hub on an ephemeral port: its base URL and its store.
async fn spawn_hub() -> (String, HubStore) {
    let store = HubStore::open_in_memory().expect("hub store");
    let state = AppState {
        store: store.clone(),
        secret: Arc::new(SECRET.to_string()),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        nexus_memory_hub::serve(listener, state)
            .await
            .expect("serve");
    });
    (format!("http://{addr}"), store)
}

async fn run_sync(db: &MemoryDb, hub: &str) -> Value {
    let mut plugin = MemoryCorePlugin::with_db(db.clone());
    let fut = plugin
        .dispatch_async(
            HANDLER_SYNC,
            &json!({
                "hub_url": hub,
                "secret": SECRET,
                "node_id": NODE,
                // The hub runs on loopback; opt out of the SSRF guard.
                "allow_private_hub": true,
            }),
        )
        .expect("sync must be async");
    fut.await.expect("sync ok")
}

/// A memory authored here, dated a day ahead: the hub refuses it (more
/// than its allowed clock skew in the future).
fn future_memory() -> Memory {
    let mut m = Memory::new("written on a clock running a day fast");
    m.updated_at = Utc::now() + Duration::days(1);
    m
}

#[tokio::test]
async fn a_refused_memory_neither_vanishes_nor_hides_later_edits() {
    let (hub, store) = spawn_hub().await;
    let db = MemoryDb::open_in_memory().unwrap();
    db.insert(&future_memory()).unwrap();

    let first = run_sync(&db, &hub).await;
    assert_eq!(first["pushed"], 0);
    assert_eq!(first["push_refused"], 1, "{first}");
    assert_eq!(first["push_dead_letters"], 1, "kept for retry: {first}");
    assert_eq!(store.count().unwrap(), 0);

    // An ordinary edit made after the refusal still reaches the hub: the
    // cursor did not jump to the refused memory's future timestamp.
    db.insert(&Memory::new("an ordinary edit, made now"))
        .unwrap();
    let second = run_sync(&db, &hub).await;
    assert_eq!(second["pushed"], 1, "{second}");
    assert_eq!(store.count().unwrap(), 1);
    // The refused memory was sent again, refused again, and is still kept.
    assert_eq!(second["push_refused"], 1, "{second}");
    assert_eq!(second["push_dead_letters"], 1, "{second}");
}

#[tokio::test]
async fn a_dead_letter_the_hub_now_accepts_is_sent_and_cleared() {
    let (hub, store) = spawn_hub().await;
    let db = MemoryDb::open_in_memory().unwrap();
    let kept = Memory::new("refused once, fine now");
    db.insert(&kept).unwrap();
    // As left by an earlier refusal, with the cursor already past it.
    db.record_push_page(
        &[],
        &[
            (kept.id.to_string(), "an earlier refusal".to_string()),
            ("mem_gone".to_string(), "deleted since".to_string()),
        ],
        &[("sync.push.updated_at", &Utc::now().to_rfc3339())],
    )
    .unwrap();

    let report = run_sync(&db, &hub).await;
    assert_eq!(report["pushed"], 1, "the retry sent it: {report}");
    assert_eq!(report["push_refused"], 0, "{report}");
    assert_eq!(
        report["push_dead_letters"], 0,
        "accepted, and the missing one dropped: {report}"
    );
    assert_eq!(store.count().unwrap(), 1);
    assert!(db.push_rejected_ids().unwrap().is_empty());
}

#[tokio::test]
async fn a_cursor_left_in_the_future_starts_over() {
    let (hub, store) = spawn_hub().await;
    let db = MemoryDb::open_in_memory().unwrap();
    // Where an older build left the cursor after passing a future-dated row.
    let ahead = (Utc::now() + Duration::days(1)).to_rfc3339();
    db.sync_state_set("sync.push.updated_at", &ahead).unwrap();
    db.insert(&Memory::new("edited while the cursor was ahead"))
        .unwrap();

    let report = run_sync(&db, &hub).await;
    assert_eq!(report["pushed"], 1, "{report}");
    assert_eq!(store.count().unwrap(), 1);
}
