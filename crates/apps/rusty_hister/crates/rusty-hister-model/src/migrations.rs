//! Fresh-install schema migrations for the nine tables in [`crate`].
//!
//! **Scope note (see the crate root docs and `docs/PROJECT-STATUS.md`'s
//! "Open items"):** this is a *fresh-install-only* schema — the shape the
//! database has today, after all of Hister's own historical migrations have
//! already run. It intentionally does **not** reproduce Hister's three
//! historical Go migrations (the `history_links.pinned` backfill, the
//! `web_sessions.last_seen_at` column drop, and the mixed-offset-to-UTC
//! timestamp rewrite) — those are one-time data-shape transitions for a
//! database that predates them, not part of the schema's current shape.
//! Whether `rusty_hister` ever needs to *open* a pre-existing
//! Hister-Go-created database file (in which case those three transitions,
//! and the legacy `indexer_versions` read path, would need equivalent
//! handling) is a still-open, unresolved decision — same status as the
//! already-flagged `indexer_versions` item.
//!
//! `Database`, Hister's own singleton-row schema-version tracker, has no
//! Rust equivalent here: `rusty_db`'s [`Migrator`] already tracks applied
//! migrations in its own bookkeeping table, so re-implementing the same
//! behavior a second time would be a duplicated, not a ported, capability.
//!
//! Migrations are plain SQL and are not portable across dialects (per
//! `rusty_db`'s own migration docs), so this module ships one array per
//! supported backend (SQLite and Postgres, per capability inventory §7.1 —
//! MySQL is not part of Hister's dual-backend story and is not provided
//! here). Pick whichever array matches the `Engine` in use:
//!
//! ```ignore
//! let engine = rusty_db::sqlite::SqliteDriver::engine("sqlite://app.db?mode=rwc").await?;
//! engine.migrator().up(rusty_hister_model::SQLITE_MIGRATIONS).await?;
//! ```

use rusty_db::prelude::*;

/// Fresh-install schema for the SQLite backend.
pub const SQLITE_MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "create_hister_schema",
    up: &[
        "CREATE TABLE users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            deleted BOOLEAN NOT NULL DEFAULT 0,
            username TEXT NOT NULL,
            password TEXT NOT NULL,
            token TEXT NOT NULL,
            is_admin BOOLEAN NOT NULL DEFAULT 0,
            rules_json TEXT NOT NULL DEFAULT '{}',
            oauth_id TEXT
        )",
        "CREATE UNIQUE INDEX idx_users_username ON users (username)",
        "CREATE INDEX idx_users_oauth_id ON users (oauth_id)",
        "CREATE TABLE links (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            deleted BOOLEAN NOT NULL DEFAULT 0,
            url TEXT NOT NULL,
            title TEXT NOT NULL
        )",
        "CREATE UNIQUE INDEX idx_links_url ON links (url)",
        "CREATE TABLE histories (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            deleted BOOLEAN NOT NULL DEFAULT 0,
            user_id INTEGER NOT NULL DEFAULT 0,
            query TEXT NOT NULL
        )",
        "CREATE UNIQUE INDEX useridqueryidx ON histories (user_id, query)",
        "CREATE TABLE history_links (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            deleted BOOLEAN NOT NULL DEFAULT 0,
            history_id INTEGER NOT NULL REFERENCES histories(id),
            link_id INTEGER NOT NULL REFERENCES links(id),
            count INTEGER NOT NULL DEFAULT 0,
            pinned BOOLEAN NOT NULL DEFAULT 0
        )",
        "CREATE UNIQUE INDEX historylinkuidx ON history_links (history_id, link_id)",
        "CREATE TABLE crawl_jobs (
            id TEXT PRIMARY KEY,
            start_url TEXT NOT NULL,
            validator_rules TEXT NOT NULL,
            label TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
        "CREATE TABLE crawl_urls (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id TEXT NOT NULL REFERENCES crawl_jobs(id),
            url TEXT NOT NULL,
            depth INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            error TEXT NOT NULL,
            error_code INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
        "CREATE UNIQUE INDEX idx_crawl_job_url ON crawl_urls (job_id, url)",
        "CREATE TABLE web_sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            token_hash TEXT NOT NULL,
            data BLOB NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            expires_at TEXT NOT NULL
        )",
        "CREATE UNIQUE INDEX idx_web_sessions_token_hash ON web_sessions (token_hash)",
        "CREATE INDEX idx_web_sessions_expires_at ON web_sessions (expires_at)",
        "CREATE TABLE document_versions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at TEXT NOT NULL,
            url TEXT NOT NULL,
            user_id INTEGER NOT NULL,
            html_diff TEXT NOT NULL,
            text_diff TEXT NOT NULL
        )",
        "CREATE INDEX idx_version_url_user ON document_versions (created_at, url, user_id)",
        "CREATE TABLE embedding_jobs (
            doc_id TEXT PRIMARY KEY,
            status TEXT NOT NULL DEFAULT 'pending',
            dirty BOOLEAN NOT NULL DEFAULT 0,
            attempts INTEGER NOT NULL DEFAULT 0,
            available_at TEXT NOT NULL,
            last_error TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
        "CREATE INDEX idx_embedding_jobs_status ON embedding_jobs (status)",
        "CREATE INDEX idx_embedding_jobs_available_at ON embedding_jobs (available_at)",
    ],
    down: &[
        "DROP TABLE history_links",
        "DROP TABLE crawl_urls",
        "DROP TABLE embedding_jobs",
        "DROP TABLE document_versions",
        "DROP TABLE web_sessions",
        "DROP TABLE crawl_jobs",
        "DROP TABLE histories",
        "DROP TABLE links",
        "DROP TABLE users",
    ],
}];

/// Fresh-install schema for the Postgres backend. Same shape as
/// [`SQLITE_MIGRATIONS`], translated to Postgres's own column syntax
/// (`BIGINT GENERATED ALWAYS AS IDENTITY` for autoincrementing primary
/// keys, `TIMESTAMPTZ` for UTC instants, `JSONB` for the two JSON-shaped
/// columns, `BYTEA` for the session's opaque blob).
pub const POSTGRES_MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "create_hister_schema",
    up: &[
        "CREATE TABLE users (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            created_at TIMESTAMPTZ NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL,
            deleted BOOLEAN NOT NULL DEFAULT FALSE,
            username TEXT NOT NULL,
            password TEXT NOT NULL,
            token TEXT NOT NULL,
            is_admin BOOLEAN NOT NULL DEFAULT FALSE,
            rules_json JSONB NOT NULL DEFAULT '{}'::jsonb,
            oauth_id TEXT
        )",
        "CREATE UNIQUE INDEX idx_users_username ON users (username)",
        "CREATE INDEX idx_users_oauth_id ON users (oauth_id)",
        "CREATE TABLE links (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            created_at TIMESTAMPTZ NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL,
            deleted BOOLEAN NOT NULL DEFAULT FALSE,
            url TEXT NOT NULL,
            title TEXT NOT NULL
        )",
        "CREATE UNIQUE INDEX idx_links_url ON links (url)",
        "CREATE TABLE histories (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            created_at TIMESTAMPTZ NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL,
            deleted BOOLEAN NOT NULL DEFAULT FALSE,
            user_id BIGINT NOT NULL DEFAULT 0,
            query TEXT NOT NULL
        )",
        "CREATE UNIQUE INDEX useridqueryidx ON histories (user_id, query)",
        "CREATE TABLE history_links (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            created_at TIMESTAMPTZ NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL,
            deleted BOOLEAN NOT NULL DEFAULT FALSE,
            history_id BIGINT NOT NULL REFERENCES histories(id),
            link_id BIGINT NOT NULL REFERENCES links(id),
            count BIGINT NOT NULL DEFAULT 0,
            pinned BOOLEAN NOT NULL DEFAULT FALSE
        )",
        "CREATE UNIQUE INDEX historylinkuidx ON history_links (history_id, link_id)",
        "CREATE TABLE crawl_jobs (
            id TEXT PRIMARY KEY,
            start_url TEXT NOT NULL,
            validator_rules JSONB NOT NULL,
            label TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL
        )",
        "CREATE TABLE crawl_urls (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            job_id TEXT NOT NULL REFERENCES crawl_jobs(id),
            url TEXT NOT NULL,
            depth BIGINT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            error TEXT NOT NULL,
            error_code BIGINT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL
        )",
        "CREATE UNIQUE INDEX idx_crawl_job_url ON crawl_urls (job_id, url)",
        "CREATE TABLE web_sessions (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            token_hash TEXT NOT NULL,
            data BYTEA NOT NULL,
            created_at TIMESTAMPTZ NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL,
            expires_at TIMESTAMPTZ NOT NULL
        )",
        "CREATE UNIQUE INDEX idx_web_sessions_token_hash ON web_sessions (token_hash)",
        "CREATE INDEX idx_web_sessions_expires_at ON web_sessions (expires_at)",
        "CREATE TABLE document_versions (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            created_at TIMESTAMPTZ NOT NULL,
            url TEXT NOT NULL,
            user_id BIGINT NOT NULL,
            html_diff TEXT NOT NULL,
            text_diff TEXT NOT NULL
        )",
        "CREATE INDEX idx_version_url_user ON document_versions (created_at, url, user_id)",
        "CREATE TABLE embedding_jobs (
            doc_id TEXT PRIMARY KEY,
            status TEXT NOT NULL DEFAULT 'pending',
            dirty BOOLEAN NOT NULL DEFAULT FALSE,
            attempts BIGINT NOT NULL DEFAULT 0,
            available_at TIMESTAMPTZ NOT NULL,
            last_error TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL
        )",
        "CREATE INDEX idx_embedding_jobs_status ON embedding_jobs (status)",
        "CREATE INDEX idx_embedding_jobs_available_at ON embedding_jobs (available_at)",
    ],
    down: &[
        "DROP TABLE history_links",
        "DROP TABLE crawl_urls",
        "DROP TABLE embedding_jobs",
        "DROP TABLE document_versions",
        "DROP TABLE web_sessions",
        "DROP TABLE crawl_jobs",
        "DROP TABLE histories",
        "DROP TABLE links",
        "DROP TABLE users",
    ],
}];

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sqlite_migrations_apply_and_are_tracked() {
        let engine = rusty_db::sqlite::SqliteDriver::engine("sqlite::memory:")
            .await
            .unwrap();
        let migrator = engine.migrator();

        let applied = migrator.up(SQLITE_MIGRATIONS).await.unwrap();
        assert_eq!(applied, vec![1]);

        let status = migrator.status(SQLITE_MIGRATIONS).await.unwrap();
        assert_eq!(status.len(), 1);
        assert!(status[0].1, "migration should be recorded as applied");

        // Applying again is a no-op: the bookkeeping table already has it.
        let applied_again = migrator.up(SQLITE_MIGRATIONS).await.unwrap();
        assert!(applied_again.is_empty());
    }

    #[tokio::test]
    async fn sqlite_migrations_create_every_table() {
        let engine = rusty_db::sqlite::SqliteDriver::engine("sqlite::memory:")
            .await
            .unwrap();
        engine.migrator().up(SQLITE_MIGRATIONS).await.unwrap();

        let tables = engine.list_tables().await.unwrap();
        for expected in [
            "users",
            "links",
            "histories",
            "history_links",
            "crawl_jobs",
            "crawl_urls",
            "web_sessions",
            "document_versions",
            "embedding_jobs",
        ] {
            assert!(
                tables.iter().any(|t| t == expected),
                "expected table `{expected}` to exist, got {tables:?}"
            );
        }
    }

    #[tokio::test]
    async fn sqlite_migrations_are_reversible() {
        let engine = rusty_db::sqlite::SqliteDriver::engine("sqlite::memory:")
            .await
            .unwrap();
        let migrator = engine.migrator();
        migrator.up(SQLITE_MIGRATIONS).await.unwrap();

        migrator.down(SQLITE_MIGRATIONS).await.unwrap();

        let tables = engine.list_tables().await.unwrap();
        assert!(!tables.iter().any(|t| t == "users"));
    }

    #[test]
    fn sqlite_and_postgres_migrations_declare_the_same_tables() {
        let count_creates = |up: &[&str]| up.iter().filter(|s| s.contains("CREATE TABLE")).count();
        assert_eq!(
            count_creates(SQLITE_MIGRATIONS[0].up),
            count_creates(POSTGRES_MIGRATIONS[0].up)
        );
        assert_eq!(SQLITE_MIGRATIONS[0].down.len(), 9);
        assert_eq!(POSTGRES_MIGRATIONS[0].down.len(), 9);
    }
}
