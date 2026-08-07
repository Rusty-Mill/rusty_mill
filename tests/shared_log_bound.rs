//! Bounding a run's event log on the shared backends.
//!
//! #60 bounded `InMemoryStore` only, so the growth it fixed was untouched on
//! the two backends chosen for scale. A TTL and a retention window bound how
//! *long* a log is kept, not how *much*, and a single streaming run can exhaust
//! an instance well inside either.
//!
//! The interesting half is Redis, which used to derive an event's index from
//! what `RPUSH` returned — the list length *was* the index. Trimming the front
//! would have restarted it and handed two different events the same
//! `Last-Event-ID`, which in memory was a test failure and here would have been
//! a resuming client silently skipping or repeating, with nothing failing. So
//! the index moved to a counter of its own, and that is what these check first.
//!
//! Skipped unless the backend is configured; when it *is* configured, an
//! unreachable one fails rather than skipping.

// Gated on having a shared backend at all: with neither, every helper below is
// dead code, and `-D warnings` is right to say so.
#![cfg(all(feature = "server", any(feature = "redis-store", feature = "postgres-store")))]

use std::time::Duration;

use rusty_acp::server::store::Store;
use rusty_acp::types::{AgentName, Event, MessagePart, Run, RunId};

/// Small enough that a handful of events crosses it.
const LIMIT: usize = 16 * 1024;

/// A part of roughly `bytes` bytes, so a test can count in whole events.
fn sized_event(bytes: usize) -> Event {
    Event::MessagePart { part: MessagePart::text("x".repeat(bytes)) }
}

/// An event carrying its index, so a misaligned read is visible rather than
/// merely short.
fn numbered_event(index: u64) -> Event {
    Event::MessagePart { part: MessagePart::text(format!("{index:0>2048}")) }
}

fn numbered_value(event: &Event) -> u64 {
    let Event::MessagePart { part } = event else { panic!("expected a message part") };
    part.content.as_deref().unwrap().trim_start_matches('0').parse().unwrap_or(0)
}

async fn seeded_run(store: &dyn Store) -> RunId {
    let run = Run::new(AgentName::new("probe").unwrap(), None);
    store.put_run(&run).await.unwrap();
    run.run_id
}

/// The four claims, run against whichever backend is handed in.
///
/// One function rather than four per backend, because the whole point is that
/// the backends agree — a copy per backend is how they stop agreeing.
async fn check_contract(store: &dyn Store) {
    let run_id = seeded_run(store).await;

    // Indices keep counting past a trim. Asserted first: everything below is
    // meaningless if two events can share an index.
    let mut indices = Vec::new();
    for index in 0..40u64 {
        indices.push(store.append_event(run_id, &numbered_event(index)).await.unwrap());
    }
    assert_eq!(indices, (0..40).collect::<Vec<u64>>(), "indices restarted or repeated over a trim");

    // Something was actually dropped, or the rest proves nothing.
    let earliest = store.earliest_event(run_id).await.unwrap();
    assert!(earliest > 0, "the log was not trimmed, so this proves nothing");

    // The log is inside the bound.
    let retained = store.events(run_id).await.unwrap();
    let held: usize = retained.iter().map(Event::approximate_size).sum();
    assert!(held <= LIMIT, "retained {held} bytes against a {LIMIT} limit");
    assert!(retained.len() < 40, "the log kept everything it was given");

    // Reading from a retained index lands on the right event, not merely on
    // some event. Seeking by the absolute index would return real events from
    // the wrong position, which is worse than returning none.
    let from = earliest + 1;
    let tail = store.events_from(run_id, from).await.unwrap();
    assert_eq!(
        numbered_value(&tail[0]),
        from,
        "events_from returned the wrong position in the log"
    );

    // And a run nothing was dropped from reports no loss, so the 410 that
    // rides on this does not fire on a log that is whole.
    let untouched = seeded_run(store).await;
    store.append_event(untouched, &sized_event(64)).await.unwrap();
    assert_eq!(store.earliest_event(untouched).await.unwrap(), 0);
}

/// The newest event is kept even when it alone exceeds the whole limit, so a
/// live tail keeps working for an agent emitting one oversized artifact.
async fn check_newest_is_kept(store: &dyn Store) {
    let run_id = seeded_run(store).await;
    for _ in 0..4 {
        store.append_event(run_id, &sized_event(2048)).await.unwrap();
    }
    store.append_event(run_id, &sized_event(64 * 1024)).await.unwrap();

    let retained = store.events(run_id).await.unwrap();
    assert_eq!(retained.len(), 1, "an oversized event should displace the rest and survive");
}

#[cfg(feature = "redis-store")]
mod redis {
    use super::*;
    use rusty_acp::server::store::{RedisStore, RedisStoreConfig};

    async fn store() -> Option<RedisStore> {
        let url = std::env::var("ACP_TEST_REDIS_URL").ok()?;
        let config = RedisStoreConfig {
            key_prefix: format!("acptest{}", uuid::Uuid::new_v4().simple()),
            ttl: Some(Duration::from_secs(60)),
            max_run_event_bytes: LIMIT,
        };
        Some(
            RedisStore::connect_with(&url, config)
                .await
                .expect("ACP_TEST_REDIS_URL is set but Redis is unreachable"),
        )
    }

    macro_rules! store_or_skip {
        () => {
            match store().await {
                Some(store) => store,
                None => {
                    eprintln!("skipping: set ACP_TEST_REDIS_URL to run the Redis tests");
                    return;
                }
            }
        };
    }

    #[tokio::test]
    async fn satisfies_the_log_bound_contract() {
        check_contract(&store_or_skip!()).await;
    }

    #[tokio::test]
    async fn keeps_the_newest_event() {
        check_newest_is_kept(&store_or_skip!()).await;
    }

    /// Indices stay dense and unique when appends race a trim.
    ///
    /// `concurrent_appends_get_distinct_indices` checks this against a list
    /// nothing is removing; this runs the same claim against one being trimmed
    /// underneath it, which is the case the counter had to be introduced for.
    #[tokio::test]
    async fn concurrent_appends_survive_a_trim() {
        let store = store_or_skip!();
        let run_id = seeded_run(&store).await;

        let appends = (0..16).map(|_| {
            let store = store.clone();
            async move { store.append_event(run_id, &sized_event(2048)).await }
        });

        let mut indices: Vec<u64> = futures_util::future::join_all(appends)
            .await
            .into_iter()
            .map(|result| result.unwrap())
            .collect();
        indices.sort_unstable();

        assert_eq!(indices, (0..16).collect::<Vec<u64>>(), "indices must be dense and unique");
        assert!(store.earliest_event(run_id).await.unwrap() > 0, "nothing was trimmed");
    }
}

#[cfg(feature = "postgres-store")]
mod postgres {
    use super::*;
    use rusty_acp::server::store::{PostgresStore, PostgresStoreConfig};

    async fn store() -> Option<PostgresStore> {
        let url = std::env::var("ACP_TEST_POSTGRES_URL").ok()?;
        let config = PostgresStoreConfig {
            table_prefix: format!("acp_t{}", uuid::Uuid::new_v4().simple()),
            retention: None,
            max_run_event_bytes: LIMIT,
            max_connections: 3,
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

    #[tokio::test]
    async fn satisfies_the_log_bound_contract() {
        check_contract(&store_or_skip!()).await;
    }

    #[tokio::test]
    async fn keeps_the_newest_event() {
        check_newest_is_kept(&store_or_skip!()).await;
    }
}
