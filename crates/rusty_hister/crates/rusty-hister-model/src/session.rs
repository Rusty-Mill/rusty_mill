//! `WebSession` — a Rust port of `server/model/session.go`. Stores browser
//! session data under a hash of the opaque cookie value; the raw session
//! identifier itself is never persisted. The lookup/update/expiry helpers
//! (`CreateWebSession`, `GetWebSession`, ...) are query-layer behavior, not
//! schema, and are a follow-up increment — see the crate root docs.
//!
//! The Go source's `last_seen_at` column (dropped by a historical
//! migration, superseded by rolling `expires_at`-based expiration) is
//! intentionally absent here: this crate targets the fresh-install schema,
//! which never had that column in the first place.

use rusty_db::prelude::*;

#[derive(Debug, Clone, PartialEq, Mapped)]
#[table(name = "web_sessions")]
pub struct WebSession {
    #[table(primary_key)]
    pub id: i64,
    /// SHA-256 (or equivalent) hash of the opaque cookie value; unique.
    pub token_hash: String,
    pub data: Vec<u8>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        "2024-01-01T00:00:00Z".parse().unwrap()
    }

    async fn migrated_engine() -> rusty_db::Engine {
        let engine = rusty_db::sqlite::SqliteDriver::engine("sqlite::memory:")
            .await
            .unwrap();
        engine
            .migrator()
            .up(crate::SQLITE_MIGRATIONS)
            .await
            .unwrap();
        engine
    }

    #[test]
    fn table_name() {
        assert_eq!(WebSession::TABLE_NAME, "web_sessions");
    }

    #[tokio::test]
    async fn round_trips_through_sqlite() {
        let engine = migrated_engine().await;
        let session = WebSession {
            id: 1,
            token_hash: "deadbeef".repeat(8),
            data: vec![1, 2, 3, 4],
            created_at: now(),
            updated_at: now(),
            expires_at: "2024-02-01T00:00:00Z".parse().unwrap(),
        };
        engine.execute(&session.insert()).await.unwrap();

        let fetched: WebSession = engine
            .fetch_one_as(&Select::from(&WebSession::table()))
            .await
            .unwrap();
        assert_eq!(fetched.token_hash, session.token_hash);
        assert_eq!(fetched.data, vec![1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn duplicate_token_hash_is_rejected() {
        let engine = migrated_engine().await;
        let session = WebSession {
            id: 1,
            token_hash: "same-hash".into(),
            data: vec![],
            created_at: now(),
            updated_at: now(),
            expires_at: now(),
        };
        engine.execute(&session.insert()).await.unwrap();

        let duplicate = WebSession { id: 2, ..session };
        assert!(engine.execute(&duplicate.insert()).await.is_err());
    }
}
