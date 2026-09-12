//! `DocumentVersion` — a Rust port of `server/model/version.go`. Stores
//! unified text diffs of a document's HTML and plain-text fields, captured
//! each time the document is re-indexed and its URL matches a versioning
//! rule (backs row 1.10's version-diff endpoint). Either diff field may be
//! empty when the corresponding content was absent or unchanged.
//!
//! The diff-format/algorithm itself (capability inventory §11) and the
//! query helpers (`SaveDocumentVersion`, `GetDocumentVersionsUntil`, ...)
//! are query-layer behavior, not schema, and are a follow-up increment —
//! see the crate root docs and `docs/PROJECT-STATUS.md`'s open items.

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
}
