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
use rusty_acp::types::{
    AgentName, AwaitResume, Event, Message, MessagePart, Run, RunStatus, Session, SessionId,
};

/// Pooled connections per test store.
///
/// Every test in this file opens its own store and they run in parallel, so
/// this is multiplied by the number of tests. The default of 10 took the file
/// past a stock server's `max_connections` of 100 as soon as it grew past nine
/// tests, and the failure — "pool timed out while waiting for an open
/// connection" — reads like an unreachable database rather than a full one.
/// The concurrency tests still queue their work through this and still prove
/// what they claim.
const TEST_POOL: u32 = 3;

/// A store on a fresh table prefix, or `None` when Postgres is not configured.
async fn store() -> Option<PostgresStore> {
    let url = std::env::var("ACP_TEST_POSTGRES_URL").ok()?;
    let config = PostgresStoreConfig {
        table_prefix: format!("acp_t{}", uuid::Uuid::new_v4().simple()),
        max_connections: TEST_POOL,
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

/// A store whose retention window is already behind everything written to it.
async fn expiring_store() -> Option<PostgresStore> {
    let url = std::env::var("ACP_TEST_POSTGRES_URL").ok()?;
    let config = PostgresStoreConfig {
        table_prefix: format!("acp_t{}", uuid::Uuid::new_v4().simple()),
        // Everything already written is instantly past it.
        retention: Some(Duration::ZERO),
        max_connections: TEST_POOL,
        ..PostgresStoreConfig::default()
    };
    Some(PostgresStore::connect_with(&url, config).await.unwrap())
}

macro_rules! expiring_store_or_skip {
    () => {
        match expiring_store().await {
            Some(store) => store,
            None => {
                eprintln!("skipping: set ACP_TEST_POSTGRES_URL to run the Postgres tests");
                return;
            }
        }
    };
}

/// Sweeping removes finished runs past the retention window, and leaves live
/// ones alone.
#[tokio::test]
async fn sweep_removes_only_what_retention_allows() {
    let store = expiring_store_or_skip!();

    let mut finished = Run::new(AgentName::new("probe").unwrap(), None);
    finished.status = RunStatus::Completed;
    store.put_run(&finished).await.unwrap();
    store.append_event(finished.run_id, &Event::generic(serde_json::json!({}))).await.unwrap();
    store
        .put_recovery_record(
            finished.run_id,
            Some(&RecoveryRecord {
                input: vec![Message::user("hi")],
                attempt: 1,
                handed_off: false,
            }),
        )
        .await
        .unwrap();

    // Still running, so not eligible however old it is: sweeping a live run
    // would delete a run someone is watching.
    let running = seeded_run(&store).await;

    assert_eq!(store.sweep().await.unwrap().runs, 1);
    assert!(store.get_run(finished.run_id).await.unwrap().is_none());
    assert!(store.events(finished.run_id).await.unwrap().is_empty());
    assert!(store.recovery_record(finished.run_id).await.unwrap().is_none());
    assert!(store.get_run(running.run_id).await.unwrap().is_some(), "a live run must survive");
}

/// With no retention configured — the default — sweeping does nothing.
///
/// Sessions as well as runs. Adding session collection must not turn the
/// no-retention default into one that quietly deletes conversations.
#[tokio::test]
async fn sweep_is_a_no_op_without_retention() {
    let store = store_or_skip!();

    let mut finished = Run::new(AgentName::new("probe").unwrap(), None);
    finished.status = RunStatus::Completed;
    store.put_run(&finished).await.unwrap();

    let session_id = SessionId::new();
    store
        .append_session_messages(session_id, "http://acp.example", vec![Message::user("hi")])
        .await
        .unwrap();

    assert!(store.sweep().await.unwrap().is_empty());
    assert!(
        store.get_run(finished.run_id).await.unwrap().is_some(),
        "keeping everything is the default, and the reason to choose this backend"
    );
    assert!(store.get_session(session_id).await.unwrap().is_some());
}

/// A session past the window goes, and takes its history and state with it.
///
/// The state document is the half usually worth more, and holding it back while
/// reporting the session collected would be the bound that is not one — the
/// same trap #38 had to avoid in memory.
#[tokio::test]
async fn sweep_collects_a_session_with_its_history_and_state() {
    let store = expiring_store_or_skip!();

    let session_id = SessionId::new();
    store
        .append_session_messages(
            session_id,
            "http://acp.example",
            vec![Message::user("something memorable")],
        )
        .await
        .unwrap();
    store
        .put_session_state(session_id, "http://acp.example", serde_json::json!({ "big": "state" }))
        .await
        .unwrap();

    assert_eq!(store.sweep().await.unwrap().sessions, 1);
    assert!(store.get_session(session_id).await.unwrap().is_none(), "the record survived");
    assert!(
        store.get_session_state(session_id).await.unwrap().is_none(),
        "the state document outlived the session it belonged to"
    );
}

/// A conversation a run is still in the middle of is never collected, however
/// far past the window it is.
///
/// This is the line #38 drew in memory and the one a time-based rule can cross
/// without it: a run that has been going longer than the retention window would
/// otherwise have its session deleted underneath it, and its output would start
/// a fresh conversation with nothing raised.
#[tokio::test]
async fn a_session_with_a_run_in_flight_is_never_collected() {
    let store = expiring_store_or_skip!();

    let session_id = SessionId::new();
    store
        .append_session_messages(session_id, "http://acp.example", vec![Message::user("hi")])
        .await
        .unwrap();

    let mut live = Run::new(AgentName::new("probe").unwrap(), Some(session_id));
    live.status = RunStatus::InProgress;
    store.put_run(&live).await.unwrap();

    assert_eq!(store.sweep().await.unwrap().sessions, 0);
    assert!(
        store.get_session(session_id).await.unwrap().is_some(),
        "a conversation was collected out from under the run writing to it"
    );

    // Once that run reaches a terminal state it protects nothing, and the
    // session ages out like any other.
    live.status = RunStatus::Completed;
    store.put_run(&live).await.unwrap();
    assert_eq!(store.sweep().await.unwrap().sessions, 1);
}

/// Adopting a session counts as use, so the sweep leaves it alone.
///
/// `ensure_session` is the first thing a run does with its session and the last
/// signal before a possibly long gap until the output is appended. Without the
/// touch a quiet conversation could be collected inside that gap.
#[tokio::test]
async fn adopting_a_session_keeps_it() {
    let Some(url) = std::env::var("ACP_TEST_POSTGRES_URL").ok() else {
        eprintln!("skipping: set ACP_TEST_POSTGRES_URL to run the Postgres tests");
        return;
    };
    let config = PostgresStoreConfig {
        table_prefix: format!("acp_t{}", uuid::Uuid::new_v4().simple()),
        // Long enough that only the write times decide, not the clock.
        retention: Some(Duration::from_secs(3)),
        max_connections: TEST_POOL,
        ..PostgresStoreConfig::default()
    };
    let store = PostgresStore::connect_with(&url, config).await.unwrap();

    let adopted = SessionId::new();
    let idle = SessionId::new();
    for session_id in [adopted, idle] {
        store
            .append_session_messages(session_id, "http://acp.example", vec![Message::user("hi")])
            .await
            .unwrap();
    }

    // Past the window for both, then one of them is picked up by a run.
    tokio::time::sleep(Duration::from_secs(4)).await;
    store.ensure_session(Session::with_id(adopted)).await.unwrap();

    assert_eq!(store.sweep().await.unwrap().sessions, 1);
    assert!(
        store.get_session(adopted).await.unwrap().is_some(),
        "a session a run had just adopted was collected"
    );
    assert!(store.get_session(idle).await.unwrap().is_none());
}

/// Reading is deliberately *not* use, and that is a decision rather than an
/// oversight — so it is asserted, and a change that made reads touch would have
/// to come here and say so.
///
/// Making a read a write would put a row lock in front of every run loading its
/// own history. A conversation being read but never added to is one nobody is
/// continuing, so ageing it out is the right answer as well as the cheap one.
#[tokio::test]
async fn reading_a_session_does_not_keep_it() {
    let store = expiring_store_or_skip!();

    let session_id = SessionId::new();
    store
        .append_session_messages(session_id, "http://acp.example", vec![Message::user("hi")])
        .await
        .unwrap();

    store.get_session(session_id).await.unwrap();
    store.get_session_state(session_id).await.unwrap();

    assert_eq!(store.sweep().await.unwrap().sessions, 1);
    assert!(store.get_session(session_id).await.unwrap().is_none());
}
