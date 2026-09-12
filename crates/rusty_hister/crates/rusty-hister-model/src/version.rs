//! `DocumentVersion` — a Rust port of `server/model/version.go`. Stores
//! unified text diffs of a document's HTML and plain-text fields, captured
//! each time the document is re-indexed and its URL matches a versioning
//! rule (backs row 1.10's version-diff endpoint). Either diff field may be
//! empty when the corresponding content was absent or unchanged.
//!
//! The diff-format/algorithm itself (capability inventory §11) is a
//! separate, not-yet-decided concern — this crate only stores whatever
//! diff text the caller already computed. `save` reuses `WebSession::create`'s
//! database-assigned-surrogate-key recipe (a raw `INSERT` omitting `id`,
//! then `RETURNING id`/`last_insert_rowid()` depending on the dialect).

use crate::placeholders;
use rusty_db::prelude::*;

#[derive(Debug, Clone, PartialEq, Mapped)]
#[table(name = "document_versions")]
pub struct DocumentVersion {
    #[table(primary_key)]
    pub id: i64,
    pub created_at: DateTime<Utc>,
    pub url: String,
    pub user_id: i64,
    pub html_diff: String,
    pub text_diff: String,
}

impl DocumentVersion {
    /// Creates a new version entry and returns its database-assigned `id`.
    pub async fn save(
        engine: &Engine,
        url: &str,
        user_id: i64,
        html_diff: &str,
        text_diff: &str,
    ) -> rusty_db::Result<i64> {
        let now = Utc::now();
        let dialect = engine.dialect();
        let p = placeholders(dialect, 5);
        let params: Vec<Value> = vec![
            now.into(),
            url.to_string().into(),
            user_id.into(),
            html_diff.to_string().into(),
            text_diff.to_string().into(),
        ];
        let mut conn = engine.connect().await?;

        if dialect.supports_returning() {
            let sql = format!(
                "INSERT INTO document_versions (created_at, url, user_id, html_diff, text_diff) \
                 VALUES ({}, {}, {}, {}, {}) RETURNING id",
                p[0], p[1], p[2], p[3], p[4]
            );
            let row = conn.fetch_one(&sql, &params).await?;
            row.get_by_name("id")
        } else {
            let sql = format!(
                "INSERT INTO document_versions (created_at, url, user_id, html_diff, text_diff) \
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

    /// Reassigns stored versions of a document to a new owner. A no-op
    /// when `from_user_id == to_user_id`.
    pub async fn move_versions(
        engine: &Engine,
        url: &str,
        from_user_id: i64,
        to_user_id: i64,
    ) -> rusty_db::Result<()> {
        if from_user_id == to_user_id {
            return Ok(());
        }
        let table = Self::table();
        engine
            .execute(
                &Update::table(&table)
                    .set("user_id", to_user_id)
                    .filter(table.col("url").eq(url))
                    .filter(table.col("user_id").eq(from_user_id)),
            )
            .await?;
        Ok(())
    }

    /// Number of stored versions for a URL and user.
    pub async fn count(engine: &Engine, url: &str, user_id: i64) -> rusty_db::Result<i64> {
        let table = Self::table();
        let row = engine
            .fetch_one(
                &Select::from(&table)
                    .columns([SelectExpr::from(Expr::count_all()).alias("count")])
                    .filter(table.col("url").eq(url))
                    .filter(table.col("user_id").eq(user_id)),
            )
            .await?;
        row.get_by_name("count")
    }

    /// All stored version diffs for a URL and user, newest first.
    pub async fn list(engine: &Engine, url: &str, user_id: i64) -> rusty_db::Result<Vec<Self>> {
        let table = Self::table();
        engine
            .fetch_all_as(
                &Select::from(&table)
                    .filter(table.col("url").eq(url))
                    .filter(table.col("user_id").eq(user_id))
                    .order_by(table.col("created_at").desc()),
            )
            .await
    }

    /// The version diffs needed to reconstruct the document state just
    /// before the version with the given id was applied: every version
    /// for the URL/user whose id is `>= version_id`, newest first, so the
    /// caller can apply their reverse diffs in sequence to walk the
    /// document back to that point in history.
    pub async fn list_until(
        engine: &Engine,
        url: &str,
        user_id: i64,
        version_id: i64,
    ) -> rusty_db::Result<Vec<Self>> {
        let table = Self::table();
        engine
            .fetch_all_as(
                &Select::from(&table)
                    .filter(table.col("url").eq(url))
                    .filter(table.col("user_id").eq(user_id))
                    .filter(table.col("id").gte(version_id))
                    .order_by(table.col("id").desc()),
            )
            .await
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
        assert_eq!(DocumentVersion::TABLE_NAME, "document_versions");
    }

    #[tokio::test]
    async fn round_trips_and_lists_newest_first() {
        let engine = migrated_engine().await;
        let older = DocumentVersion {
            id: 1,
            created_at: now(),
            url: "https://example.com".into(),
            user_id: 1,
            html_diff: "-old\n+new".into(),
            text_diff: "-old\n+new".into(),
        };
        let newer = DocumentVersion {
            id: 2,
            created_at: "2024-02-01T00:00:00Z".parse().unwrap(),
            ..older.clone()
        };
        engine.execute(&older.insert()).await.unwrap();
        engine.execute(&newer.insert()).await.unwrap();

        let table = DocumentVersion::table();
        let versions: Vec<DocumentVersion> = engine
            .fetch_all_as(&Select::from(&table).order_by(table.col("created_at").desc()))
            .await
            .unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].id, newer.id);
        assert_eq!(versions[1].id, older.id);
    }

    #[tokio::test]
    async fn save_assigns_an_id_and_is_listed() {
        let engine = migrated_engine().await;
        let id = DocumentVersion::save(&engine, "https://example.com", 1, "-a\n+b", "-a\n+b")
            .await
            .unwrap();
        assert!(id > 0);

        let versions = DocumentVersion::list(&engine, "https://example.com", 1)
            .await
            .unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].id, id);
        assert_eq!(versions[0].html_diff, "-a\n+b");
    }

    #[tokio::test]
    async fn list_only_returns_versions_for_the_matching_url_and_user() {
        let engine = migrated_engine().await;
        DocumentVersion::save(&engine, "https://example.com", 1, "a", "a")
            .await
            .unwrap();
        DocumentVersion::save(&engine, "https://example.com", 2, "b", "b")
            .await
            .unwrap();
        DocumentVersion::save(&engine, "https://other.com", 1, "c", "c")
            .await
            .unwrap();

        let versions = DocumentVersion::list(&engine, "https://example.com", 1)
            .await
            .unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].html_diff, "a");
    }

    #[tokio::test]
    async fn count_matches_the_number_of_stored_versions() {
        let engine = migrated_engine().await;
        assert_eq!(
            DocumentVersion::count(&engine, "https://example.com", 1)
                .await
                .unwrap(),
            0
        );

        DocumentVersion::save(&engine, "https://example.com", 1, "a", "a")
            .await
            .unwrap();
        DocumentVersion::save(&engine, "https://example.com", 1, "b", "b")
            .await
            .unwrap();

        assert_eq!(
            DocumentVersion::count(&engine, "https://example.com", 1)
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn move_versions_reassigns_owner() {
        let engine = migrated_engine().await;
        DocumentVersion::save(&engine, "https://example.com", 1, "a", "a")
            .await
            .unwrap();

        DocumentVersion::move_versions(&engine, "https://example.com", 1, 2)
            .await
            .unwrap();

        assert_eq!(
            DocumentVersion::count(&engine, "https://example.com", 1)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            DocumentVersion::count(&engine, "https://example.com", 2)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn move_versions_is_a_no_op_for_the_same_user() {
        let engine = migrated_engine().await;
        DocumentVersion::save(&engine, "https://example.com", 1, "a", "a")
            .await
            .unwrap();

        DocumentVersion::move_versions(&engine, "https://example.com", 1, 1)
            .await
            .unwrap();

        assert_eq!(
            DocumentVersion::count(&engine, "https://example.com", 1)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn list_until_returns_versions_from_the_given_id_onward_newest_first() {
        let engine = migrated_engine().await;
        let first = DocumentVersion::save(&engine, "https://example.com", 1, "a", "a")
            .await
            .unwrap();
        let second = DocumentVersion::save(&engine, "https://example.com", 1, "b", "b")
            .await
            .unwrap();
        let third = DocumentVersion::save(&engine, "https://example.com", 1, "c", "c")
            .await
            .unwrap();

        let versions = DocumentVersion::list_until(&engine, "https://example.com", 1, second)
            .await
            .unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].id, third);
        assert_eq!(versions[1].id, second);
        assert!(!versions.iter().any(|v| v.id == first));
    }
}
