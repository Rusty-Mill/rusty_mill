//! `WebSession` — a Rust port of `server/model/session.go`. Stores browser
//! session data under a hash of the opaque cookie value; the raw session
//! identifier itself is never persisted.
//!
//! The Go source's `last_seen_at` column (dropped by a historical
//! migration, superseded by rolling `expires_at`-based expiration) is
//! intentionally absent here: this crate targets the fresh-install schema,
//! which never had that column in the first place.
//!
//! `create` is the first place this crate needs a database-assigned
//! surrogate key: `rusty_db`'s `#[derive(Mapped)]` `insert()` always
//! includes the primary-key field's current value (there's no "leave this
//! to the database" marker), so it can't be used as-is for an
//! autoincrementing `i64` column — inserting a placeholder like `0` would
//! either become the literal row id (SQLite, which has no complaint about
//! an explicit rowid) or be rejected outright (Postgres's
//! `GENERATED ALWAYS AS IDENTITY`, which refuses an explicit value without
//! `OVERRIDING SYSTEM VALUE`). `create` instead drops to a raw `INSERT`
//! that omits the `id` column entirely, then recovers the generated value
//! dialect-appropriately: `RETURNING id` where `Dialect::supports_returning()`
//! is true (Postgres), `SELECT last_insert_rowid()` otherwise (SQLite —
//! this crate's dialect model reports `false` here even though modern
//! SQLite itself supports `RETURNING`). The same recipe applies to every
//! other autoincrementing model in this crate (`User`, `Link`, `History`,
//! `HistoryLink`, `DocumentVersion`) once their own `create`-equivalents
//! are ported.

use crate::placeholders;
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

impl WebSession {
    /// Creates a new session and returns its database-assigned `id`.
    pub async fn create(
        engine: &Engine,
        token_hash: &str,
        data: &[u8],
        expires_at: DateTime<Utc>,
    ) -> rusty_db::Result<i64> {
        let now = Utc::now();
        let dialect = engine.dialect();
        let p = placeholders(dialect, 5);
        let params: Vec<Value> = vec![
            token_hash.to_string().into(),
            data.to_vec().into(),
            now.into(),
            now.into(),
            expires_at.into(),
        ];
        let mut conn = engine.connect().await?;

        if dialect.supports_returning() {
            let sql = format!(
                "INSERT INTO web_sessions (token_hash, data, created_at, updated_at, expires_at) \
                 VALUES ({}, {}, {}, {}, {}) RETURNING id",
                p[0], p[1], p[2], p[3], p[4]
            );
            let row = conn.fetch_one(&sql, &params).await?;
            row.get_by_name("id")
        } else {
            let sql = format!(
                "INSERT INTO web_sessions (token_hash, data, created_at, updated_at, expires_at) \
                 VALUES ({}, {}, {}, {}, {})",
                p[0], p[1], p[2], p[3], p[4]
            );
            conn.execute(&sql, &params).await?;
            let row = conn
                .fetch_one("SELECT last_insert_rowid() AS id", &[])
                .await?;
            row.get_by_name("id")
        }
    }

    /// Looks up a session by its token hash. `None` when no session (or no
    /// longer a live one) matches — Go's `GetWebSession` signals the same
    /// case with a dedicated `ErrWebSessionNotFound` sentinel error;
    /// `Option` is the idiomatic Rust equivalent for a lookup that can
    /// legitimately find nothing.
    pub async fn get(engine: &Engine, token_hash: &str) -> rusty_db::Result<Option<Self>> {
        let table = Self::table();
        engine
            .fetch_optional_as(
                &Select::from(&table)
                    .filter(table.col("token_hash").eq(token_hash))
                    .filter(table.col("expires_at").gt(Utc::now())),
            )
            .await
    }

    /// Updates a session's data and expiry. Returns `false` when no session
    /// matched `token_hash` (Go returns `ErrWebSessionNotFound` for this).
    ///
    /// Named `refresh`, not `update` (Go: `UpdateWebSession`) — `update` is
    /// already the name `#[derive(Mapped)]` generates for the instance
    /// method that turns a whole `WebSession` value into an `UPDATE`
    /// statement (`self.update()`), and the two would collide.
    pub async fn refresh(
        engine: &Engine,
        token_hash: &str,
        data: &[u8],
        expires_at: DateTime<Utc>,
    ) -> rusty_db::Result<bool> {
        let table = Self::table();
        let affected = engine
            .execute(
                &Update::table(&table)
                    .set("data", data.to_vec())
                    .set("expires_at", expires_at)
                    .filter(table.col("token_hash").eq(token_hash)),
            )
            .await?;
        Ok(affected == 1)
    }

    /// Removes a session. A no-op if `token_hash` doesn't match any row.
    pub async fn delete(engine: &Engine, token_hash: &str) -> rusty_db::Result<()> {
        let table = Self::table();
        engine
            .execute(&Delete::from(&table).filter(table.col("token_hash").eq(token_hash)))
            .await?;
        Ok(())
    }
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

    #[tokio::test]
    async fn create_assigns_an_id_and_is_retrievable_by_token_hash() {
        let engine = migrated_engine().await;
        let expires_at: DateTime<Utc> = "2030-02-01T00:00:00Z".parse().unwrap();

        let id = WebSession::create(&engine, "token-hash-1", b"session-data", expires_at)
            .await
            .unwrap();
        assert!(id > 0);

        let fetched = WebSession::get(&engine, "token-hash-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.id, id);
        assert_eq!(fetched.data, b"session-data");
        assert_eq!(fetched.expires_at, expires_at);
    }

    #[tokio::test]
    async fn create_assigns_distinct_ids_to_separate_sessions() {
        let engine = migrated_engine().await;
        let expires_at = now();

        let first = WebSession::create(&engine, "hash-a", b"a", expires_at)
            .await
            .unwrap();
        let second = WebSession::create(&engine, "hash-b", b"b", expires_at)
            .await
            .unwrap();
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn get_returns_none_for_an_unknown_token_hash() {
        let engine = migrated_engine().await;
        assert_eq!(
            WebSession::get(&engine, "no-such-hash").await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn get_returns_none_for_an_expired_session() {
        let engine = migrated_engine().await;
        let expired_at: DateTime<Utc> = "2024-01-01T00:00:00Z".parse().unwrap();
        WebSession::create(&engine, "token-hash-1", b"session-data", expired_at)
            .await
            .unwrap();

        assert_eq!(
            WebSession::get(&engine, "token-hash-1").await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn refresh_changes_data_and_expiry_for_a_known_session() {
        let engine = migrated_engine().await;
        WebSession::create(&engine, "token-hash-1", b"old-data", now())
            .await
            .unwrap();

        let new_expires_at: DateTime<Utc> = "2030-01-01T00:00:00Z".parse().unwrap();
        let updated = WebSession::refresh(&engine, "token-hash-1", b"new-data", new_expires_at)
            .await
            .unwrap();
        assert!(updated);

        let fetched = WebSession::get(&engine, "token-hash-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.data, b"new-data");
        assert_eq!(fetched.expires_at, new_expires_at);
    }

    #[tokio::test]
    async fn refresh_reports_false_for_an_unknown_token_hash() {
        let engine = migrated_engine().await;
        let updated = WebSession::refresh(&engine, "no-such-hash", b"data", now())
            .await
            .unwrap();
        assert!(!updated);
    }

    #[tokio::test]
    async fn delete_removes_a_session() {
        let engine = migrated_engine().await;
        WebSession::create(&engine, "token-hash-1", b"data", now())
            .await
            .unwrap();

        WebSession::delete(&engine, "token-hash-1").await.unwrap();

        assert_eq!(
            WebSession::get(&engine, "token-hash-1").await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn delete_is_a_no_op_for_an_unknown_token_hash() {
        let engine = migrated_engine().await;
        WebSession::delete(&engine, "no-such-hash").await.unwrap();
    }
}
