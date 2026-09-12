//! `History`, `Link`, and `HistoryLink` — a Rust port of the tables backing
//! Hister's search-history tracking (query <-> URL associations), from
//! `server/model/history.go`. This is a distinct concept from the indexed
//! *document* store the extractor/indexer crates populate: `History` records
//! a user's past search *queries*, `Link` a URL that appeared in a result
//! set, and `HistoryLink` the join between them (plus the `count`/`pinned`
//! bookkeeping that back rows 1.16-1.18's history/pin features).
//!
//! The query helpers `history.go` builds on top of these tables (result
//! timelines, pin toggling, per-user URL counts) are query-layer behavior,
//! not schema, and are a follow-up increment — see the crate root docs.

use rusty_db::prelude::*;

/// A user's search query, tracked for history/autocomplete purposes.
#[derive(Debug, Clone, PartialEq, Mapped)]
#[table(name = "histories")]
pub struct History {
    #[table(primary_key)]
    pub id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[table(soft_delete)]
    pub deleted: bool,
    /// `0` for the single-user/no-auth deployment case (Go: `default:0`).
    #[table(default = "0")]
    pub user_id: i64,
    pub query: String,
}

/// A URL that appeared in a history entry's result set.
///
/// The Go source's `Links []*Link` many-to-many association on `History`
/// joins through this crate's [`HistoryLink`] table (`DB.SetupJoinTable`,
/// not GORM's auto-generated bare join table) — that relationship is
/// query-layer behavior, not represented as a field here.
#[derive(Debug, Clone, PartialEq, Mapped)]
#[table(name = "links")]
pub struct Link {
    #[table(primary_key)]
    pub id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[table(soft_delete)]
    pub deleted: bool,
    pub url: String,
    pub title: String,
}

/// The `History` <-> `Link` join, carrying the result-ranking bookkeeping
/// (`count`, `pinned`) that makes this more than a bare many-to-many table.
#[derive(Debug, Clone, PartialEq, Mapped)]
#[table(name = "history_links")]
pub struct HistoryLink {
    #[table(primary_key)]
    pub id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[table(soft_delete)]
    pub deleted: bool,
    pub history_id: i64,
    pub link_id: i64,
    /// Number of times this URL was selected from this query's results.
    #[table(default = "0")]
    pub count: i64,
    /// "Pin as priority result" (row 1.18's `pin` param).
    #[table(default = "false")]
    pub pinned: bool,
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
    fn table_names() {
        assert_eq!(History::TABLE_NAME, "histories");
        assert_eq!(Link::TABLE_NAME, "links");
        assert_eq!(HistoryLink::TABLE_NAME, "history_links");
    }

    #[tokio::test]
    async fn history_link_joins_a_history_and_a_link() {
        let engine = migrated_engine().await;

        let history = History {
            id: 1,
            created_at: now(),
            updated_at: now(),
            deleted: false,
            user_id: 0,
            query: "rust ownership".into(),
        };
        let link = Link {
            id: 1,
            created_at: now(),
            updated_at: now(),
            deleted: false,
            url: "https://doc.rust-lang.org/book/ch04-00-understanding-ownership.html".into(),
            title: "Understanding Ownership".into(),
        };
        engine.execute(&history.insert()).await.unwrap();
        engine.execute(&link.insert()).await.unwrap();

        let history_link = HistoryLink {
            id: 1,
            created_at: now(),
            updated_at: now(),
            deleted: false,
            history_id: history.id,
            link_id: link.id,
            count: 3,
            pinned: true,
        };
        engine.execute(&history_link.insert()).await.unwrap();

        let fetched: HistoryLink = engine
            .fetch_one_as(&Select::from(&HistoryLink::table()))
            .await
            .unwrap();
        assert_eq!(fetched.history_id, history.id);
        assert_eq!(fetched.link_id, link.id);
        assert_eq!(fetched.count, 3);
        assert!(fetched.pinned);
    }

    #[tokio::test]
    async fn duplicate_history_link_pair_is_rejected() {
        let engine = migrated_engine().await;
        let history = History {
            id: 1,
            created_at: now(),
            updated_at: now(),
            deleted: false,
            user_id: 0,
            query: "q".into(),
        };
        let link = Link {
            id: 1,
            created_at: now(),
            updated_at: now(),
            deleted: false,
            url: "https://example.com".into(),
            title: "".into(),
        };
        engine.execute(&history.insert()).await.unwrap();
        engine.execute(&link.insert()).await.unwrap();

        let first = HistoryLink {
            id: 1,
            created_at: now(),
            updated_at: now(),
            deleted: false,
            history_id: 1,
            link_id: 1,
            count: 1,
            pinned: false,
        };
        engine.execute(&first.insert()).await.unwrap();

        let duplicate = HistoryLink { id: 2, ..first };
        assert!(engine.execute(&duplicate.insert()).await.is_err());
    }

    #[tokio::test]
    async fn duplicate_user_query_pair_is_rejected() {
        let engine = migrated_engine().await;
        let history = History {
            id: 1,
            created_at: now(),
            updated_at: now(),
            deleted: false,
            user_id: 7,
            query: "same query".into(),
        };
        engine.execute(&history.insert()).await.unwrap();

        let duplicate = History { id: 2, ..history };
        assert!(engine.execute(&duplicate.insert()).await.is_err());
    }
}
