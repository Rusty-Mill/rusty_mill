//! The three shipped backends, run against the public conformance suite.
//!
//! This pays back inward as much as outward. Before the testkit existed the
//! store-level invariants — dense indices under concurrent append, exactly one
//! lease claim, expiry enforced on read — were checked in
//! `tests/postgres_store.rs`, which is to say **for Postgres only**, because
//! that is when they were written. `InMemoryStore` and `RedisStore` were
//! covered end-to-end through the server and not at the trait.
//!
//! Running the same suite against all three means they are held to the contract
//! a third-party backend is held to, rather than to whatever each happens to do.

#![cfg(all(feature = "store-testkit", feature = "server"))]

use std::sync::Arc;

use rusty_acp::server::store::{testkit, InMemoryStore, Store};

#[tokio::test]
async fn the_in_memory_store_satisfies_the_contract() {
    let report =
        testkit::verify(|| async { Arc::new(InMemoryStore::default()) as Arc<dyn Store> }).await;
    assert!(report.is_ok(), "{report}");
    assert!(!report.passed.is_empty(), "the suite ran no checks");
}

#[cfg(feature = "redis-store")]
#[tokio::test]
async fn the_redis_store_satisfies_the_contract() {
    use rusty_acp::server::store::{RedisStore, RedisStoreConfig};

    let Ok(url) = std::env::var("ACP_TEST_REDIS_URL") else {
        eprintln!("skipping: set ACP_TEST_REDIS_URL to run the Redis conformance suite");
        return;
    };

    // A fresh key prefix per check, which is what the factory is for: one
    // check's runs and leases must not be visible to the next.
    let report = testkit::verify(|| {
        let url = url.clone();
        async move {
            let config = RedisStoreConfig {
                key_prefix: format!("acpkit{}", uuid::Uuid::new_v4().simple()),
                ..RedisStoreConfig::default()
            };
            Arc::new(
                RedisStore::connect_with(&url, config)
                    .await
                    .expect("ACP_TEST_REDIS_URL is set but Redis is unreachable"),
            ) as Arc<dyn Store>
        }
    })
    .await;

    assert!(report.is_ok(), "{report}");
}

#[cfg(feature = "postgres-store")]
#[tokio::test]
async fn the_postgres_store_satisfies_the_contract() {
    use rusty_acp::server::store::{PostgresStore, PostgresStoreConfig};

    let Ok(url) = std::env::var("ACP_TEST_POSTGRES_URL") else {
        eprintln!("skipping: set ACP_TEST_POSTGRES_URL to run the Postgres conformance suite");
        return;
    };

    let report = testkit::verify(|| {
        let url = url.clone();
        async move {
            let config = PostgresStoreConfig {
                table_prefix: format!("acp_k{}", uuid::Uuid::new_v4().simple()),
                // One connection per check; the suite runs them in sequence.
                max_connections: 4,
                ..PostgresStoreConfig::default()
            };
            Arc::new(
                PostgresStore::connect_with(&url, config)
                    .await
                    .expect("ACP_TEST_POSTGRES_URL is set but Postgres is unreachable"),
            ) as Arc<dyn Store>
        }
    })
    .await;

    assert!(report.is_ok(), "{report}");
}

/// A backend that breaks one invariant is caught, and the report names which.
///
/// Without this the suite could pass everything by asserting nothing, and a
/// backend author would get a clean bill from a kit that does not look. The
/// broken store hands out indices by reading the length and adding one — the
/// obvious implementation, and the one that is wrong only under contention.
#[tokio::test]
async fn a_broken_backend_is_caught() {
    use rusty_acp::server::store::{
        Notification, NotificationStream, RecoveryRecord, SessionRecord, StoreResult,
    };
    use rusty_acp::types::{Event, Message, Run, RunId, Session, SessionId};
    use std::time::Duration;

    /// An [`InMemoryStore`] whose event indices are computed the naive way.
    #[derive(Debug, Default)]
    struct RacyIndices {
        inner: InMemoryStore,
    }

    #[async_trait::async_trait]
    impl Store for RacyIndices {
        async fn append_event(&self, run_id: RunId, event: &Event) -> StoreResult<u64> {
            // Read, then write: two concurrent appends see the same length and
            // are told the same index.
            let index = self.inner.events(run_id).await?.len() as u64;
            tokio::task::yield_now().await;
            self.inner.append_event(run_id, event).await?;
            Ok(index)
        }

        async fn put_run(&self, run: &Run) -> StoreResult<()> {
            self.inner.put_run(run).await
        }
        async fn get_run(&self, run_id: RunId) -> StoreResult<Option<Run>> {
            self.inner.get_run(run_id).await
        }
        async fn events(&self, run_id: RunId) -> StoreResult<Vec<Event>> {
            self.inner.events(run_id).await
        }
        async fn events_from(&self, run_id: RunId, from: u64) -> StoreResult<Vec<Event>> {
            self.inner.events_from(run_id, from).await
        }
        async fn earliest_event(&self, run_id: RunId) -> StoreResult<u64> {
            self.inner.earliest_event(run_id).await
        }
        async fn publish(&self, run_id: RunId, notification: Notification) -> StoreResult<()> {
            self.inner.publish(run_id, notification).await
        }
        async fn subscribe(&self, run_id: RunId) -> StoreResult<NotificationStream> {
            self.inner.subscribe(run_id).await
        }
        async fn get_session(&self, session_id: SessionId) -> StoreResult<Option<SessionRecord>> {
            self.inner.get_session(session_id).await
        }
        async fn ensure_session(&self, session: Session) -> StoreResult<SessionRecord> {
            self.inner.ensure_session(session).await
        }
        async fn append_session_messages(
            &self,
            session_id: SessionId,
            base_url: &str,
            messages: Vec<Message>,
        ) -> StoreResult<()> {
            self.inner.append_session_messages(session_id, base_url, messages).await
        }
        async fn get_session_state(
            &self,
            session_id: SessionId,
        ) -> StoreResult<Option<serde_json::Value>> {
            self.inner.get_session_state(session_id).await
        }
        async fn put_session_state(
            &self,
            session_id: SessionId,
            base_url: &str,
            state: serde_json::Value,
        ) -> StoreResult<()> {
            self.inner.put_session_state(session_id, base_url, state).await
        }
        async fn renew_lease(&self, run_id: RunId, owner: &str, ttl: Duration) -> StoreResult<()> {
            self.inner.renew_lease(run_id, owner, ttl).await
        }
        async fn lease_owner(&self, run_id: RunId) -> StoreResult<Option<String>> {
            self.inner.lease_owner(run_id).await
        }
        async fn try_claim_lease(
            &self,
            run_id: RunId,
            owner: &str,
            ttl: Duration,
        ) -> StoreResult<bool> {
            self.inner.try_claim_lease(run_id, owner, ttl).await
        }
        async fn recovery_record(&self, run_id: RunId) -> StoreResult<Option<RecoveryRecord>> {
            self.inner.recovery_record(run_id).await
        }
        async fn put_recovery_record(
            &self,
            run_id: RunId,
            record: Option<&RecoveryRecord>,
        ) -> StoreResult<()> {
            self.inner.put_recovery_record(run_id, record).await
        }
        async fn release_lease(&self, run_id: RunId) -> StoreResult<()> {
            self.inner.release_lease(run_id).await
        }
    }

    let report =
        testkit::verify(|| async { Arc::new(RacyIndices::default()) as Arc<dyn Store> }).await;

    assert!(!report.is_ok(), "the suite passed a backend with racy indices");
    assert!(
        report.failed.iter().any(|(name, _)| name.contains("indices")),
        "the report did not name the broken invariant: {report}"
    );
    // And it kept going rather than stopping at the first failure, which is
    // what makes the report worth reading.
    assert!(!report.passed.is_empty(), "one failure aborted the whole suite");
}
