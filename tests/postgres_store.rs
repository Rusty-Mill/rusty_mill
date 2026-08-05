//! Contract tests for the Postgres backend's own machinery.
//!
//! The behaviour every backend shares lives in `multi_replica.rs` and runs
//! against all three. What is here is the part Postgres does differently:
//! expiry it has to enforce itself rather than getting from a key TTL, and
//! notifications that travel as a pointer because `NOTIFY` is capped at 8000
//! bytes.
//!
//! Skipped unless `ACP_TEST_POSTGRES_URL` is set; when it *is* set an
//! unreachable database fails rather than skipping.

#![cfg(all(feature = "postgres-store", feature = "server"))]

use std::time::Duration;

use futures_util::StreamExt;
use rusty_acp::server::store::{
    Notification, PostgresStore, PostgresStoreConfig, RecoveryRecord, Store,
};
use rusty_acp::types::{AgentName, AwaitResume, Event, Message, MessagePart, Run, RunStatus};

/// A store on a fresh table prefix, or `None` when Postgres is not configured.
async fn store() -> Option<PostgresStore> {
    let url = std::env::var("ACP_TEST_POSTGRES_URL").ok()?;
    let config = PostgresStoreConfig {
        table_prefix: format!("acp_t{}", uuid::Uuid::new_v4().simple()),
        ..PostgresStoreConfig::default()
    };
    Some(
        PostgresStore::connect_with(&url, config)
            .await
            .expect("ACP_TEST_POSTGRES_URL is set but Postgres is unreachable"),
    )
}

macro_rules! store_or_skip {
    () => {
        match store().await {
            Some(store) => store,
            None => {
                eprintln!("skipping: set ACP_TEST_POSTGRES_URL to run the Postgres tests");
                return;
            }
        }
    };
}

async fn seeded_run(store: &PostgresStore) -> Run {
    let run = Run::new(AgentName::new("probe").unwrap(), None);
    store.put_run(&run).await.unwrap();
    run
}

/// A lease outlives its TTL only if nobody renews it.
///
/// Redis expires the key itself; here expiry is a column, so "has it lapsed"
/// is something this backend has to get right on every read.
#[tokio::test]
async fn a_lease_survives_until_its_ttl_and_no_longer() {
    let store = store_or_skip!();
    let run = seeded_run(&store).await;

    store.renew_lease(run.run_id, "replica-a", Duration::from_secs(2)).await.unwrap();
    assert_eq!(store.lease_owner(run.run_id).await.unwrap().as_deref(), Some("replica-a"));

    // Comfortably inside the TTL.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        store.lease_owner(run.run_id).await.unwrap().as_deref(),
        Some("replica-a"),
        "a lease must not lapse early — a live run would be reaped"
    );

    // Renewing pushes the expiry out rather than leaving the original.
    store.renew_lease(run.run_id, "replica-a", Duration::from_secs(2)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1700)).await;
    assert_eq!(
        store.lease_owner(run.run_id).await.unwrap().as_deref(),
        Some("replica-a"),
        "renewal must extend the lease, not be ignored"
    );
}

/// A lease nobody renews does lapse, which is what makes an abandoned run
/// recognisable.
#[tokio::test]
async fn an_unrenewed_lease_lapses() {
    let store = store_or_skip!();
    let run = seeded_run(&store).await;

    store.renew_lease(run.run_id, "replica-a", Duration::from_millis(300)).await.unwrap();
    assert!(store.lease_owner(run.run_id).await.unwrap().is_some());

    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        store.lease_owner(run.run_id).await.unwrap(),
        None,
        "a lease nobody renewed must lapse, or an abandoned run hangs forever"
    );
}

/// Exactly one claimant wins, which is what stops two replicas recovering the
/// same run.
#[tokio::test]
async fn only_one_replica_can_claim_a_lease() {
    let store = store_or_skip!();
    let run = seeded_run(&store).await;

    // Nobody holds it: the first claim wins.
    assert!(store.try_claim_lease(run.run_id, "replica-a", Duration::from_secs(5)).await.unwrap());
    // Someone holds it, and it is live: everyone else loses.
    assert!(!store.try_claim_lease(run.run_id, "replica-b", Duration::from_secs(5)).await.unwrap());
    assert_eq!(store.lease_owner(run.run_id).await.unwrap().as_deref(), Some("replica-a"));

    // A lapsed lease is claimable again.
    store.renew_lease(run.run_id, "replica-a", Duration::from_millis(200)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(store.try_claim_lease(run.run_id, "replica-b", Duration::from_secs(5)).await.unwrap());
    assert_eq!(store.lease_owner(run.run_id).await.unwrap().as_deref(), Some("replica-b"));
}

/// Concurrent claims on the same run produce exactly one winner.
#[tokio::test]
async fn concurrent_claims_produce_one_winner() {
    let store = store_or_skip!();
    let run = seeded_run(&store).await;

    let claims = (0..8).map(|index| {
        let store = store.clone();
        let owner = format!("replica-{index}");
        async move { store.try_claim_lease(run.run_id, &owner, Duration::from_secs(5)).await }
    });

    let won = futures_util::future::join_all(claims)
        .await
        .into_iter()
        .filter(|result| *result.as_ref().unwrap())
        .count();

    assert_eq!(won, 1, "two replicas both recovering one run is the thing this prevents");
}

/// Event indices are dense and unique even when appends race.
#[tokio::test]
async fn concurrent_appends_get_distinct_indices() {
    let store = store_or_skip!();
    let run = seeded_run(&store).await;

    let appends = (0..16).map(|index| {
        let store = store.clone();
        async move {
            let event = Event::generic(serde_json::json!({ "n": index }));
            store.append_event(run.run_id, &event).await
        }
    });

    let mut indices: Vec<u64> = futures_util::future::join_all(appends)
        .await
        .into_iter()
        .map(|result| result.unwrap())
        .collect();
    indices.sort_unstable();

    assert_eq!(indices, (0..16).collect::<Vec<u64>>(), "indices must be dense and unique");
    assert_eq!(store.events(run.run_id).await.unwrap().len(), 16);
}

/// An event too large for a `NOTIFY` payload still reaches subscribers.
///
/// This is the case the pointer exists for: the notification carries the log
/// index and the subscriber reads the row, so payload size stops mattering.
#[tokio::test]
async fn an_oversized_event_still_reaches_a_subscriber() {
    let store = store_or_skip!();
    let run = seeded_run(&store).await;

    let mut subscription = store.subscribe(run.run_id).await.unwrap();

    // Well past the 8000-byte NOTIFY cap.
    let big = "x".repeat(64 * 1024);
    let event = Event::MessagePart { part: MessagePart::text(&big) };
    let index = store.append_event(run.run_id, &event).await.unwrap();
    store.publish(run.run_id, Notification::event_at(index, event)).await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .expect("a subscriber must receive an event larger than the NOTIFY cap")
        .expect("the subscription must yield");

    assert_eq!(received.index(), Some(index));
    match received.event() {
        Some(Event::MessagePart { part }) => {
            assert_eq!(part.as_text().unwrap().len(), big.len())
        }
        other => panic!("expected the message part back, got {other:?}"),
    }
}

/// The same for a control signal, which has no log row to point at and is
/// parked instead.
#[tokio::test]
async fn an_oversized_resume_still_reaches_a_subscriber() {
    let store = store_or_skip!();
    let run = seeded_run(&store).await;

    let mut subscription = store.subscribe(run.run_id).await.unwrap();

    let big = "y".repeat(64 * 1024);
    let payload = AwaitResume::from(serde_json::json!({ "answer": big }));
    store.publish(run.run_id, Notification::Resume(payload)).await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .expect("an oversized resume must still be delivered")
        .expect("the subscription must yield");

    match received {
        Notification::Resume(resume) => {
            assert_eq!(resume.as_value()["answer"].as_str().unwrap().len(), big.len())
        }
        other => panic!("expected the resume payload back, got {other:?}"),
    }
}

/// Sweeping removes finished runs past the retention window, and leaves
/// everything else — including sessions, which outlive the runs that fed them.
#[tokio::test]
async fn sweep_removes_only_what_retention_allows() {
    let Some(url) = std::env::var("ACP_TEST_POSTGRES_URL").ok() else {
        eprintln!("skipping: set ACP_TEST_POSTGRES_URL to run the Postgres tests");
        return;
    };
    let config = PostgresStoreConfig {
        table_prefix: format!("acp_t{}", uuid::Uuid::new_v4().simple()),
        // Everything already written is instantly past it.
        retention: Some(Duration::ZERO),
        ..PostgresStoreConfig::default()
    };
    let store = PostgresStore::connect_with(&url, config).await.unwrap();

    let mut finished = Run::new(AgentName::new("probe").unwrap(), None);
    finished.status = RunStatus::Completed;
    store.put_run(&finished).await.unwrap();
    store.append_event(finished.run_id, &Event::generic(serde_json::json!({}))).await.unwrap();
    store
        .put_recovery_record(
            finished.run_id,
            Some(&RecoveryRecord { input: vec![Message::user("hi")], attempt: 1 }),
        )
        .await
        .unwrap();

    // Still running, so not eligible however old it is: sweeping a live run
    // would delete a run someone is watching.
    let running = seeded_run(&store).await;

    assert_eq!(store.sweep().await.unwrap(), 1);
    assert!(store.get_run(finished.run_id).await.unwrap().is_none());
    assert!(store.events(finished.run_id).await.unwrap().is_empty());
    assert!(store.recovery_record(finished.run_id).await.unwrap().is_none());
    assert!(store.get_run(running.run_id).await.unwrap().is_some(), "a live run must survive");
}

/// With no retention configured — the default — sweeping does nothing.
#[tokio::test]
async fn sweep_is_a_no_op_without_retention() {
    let store = store_or_skip!();

    let mut finished = Run::new(AgentName::new("probe").unwrap(), None);
    finished.status = RunStatus::Completed;
    store.put_run(&finished).await.unwrap();

    assert_eq!(store.sweep().await.unwrap(), 0);
    assert!(
        store.get_run(finished.run_id).await.unwrap().is_some(),
        "keeping everything is the default, and the reason to choose this backend"
    );
}
