//! `History`, `Link`, and `HistoryLink` — a Rust port of the tables backing
//! Hister's search-history tracking (query <-> URL associations), from
//! `server/model/history.go`. This is a distinct concept from the indexed
//! *document* store the extractor/indexer crates populate: `History` records
//! a user's past search *queries*, `Link` a URL that appeared in a result
//! set, and `HistoryLink` the join between them (plus the `count`/`pinned`
//! bookkeeping that back rows 1.16-1.18's history/pin features).
//!
//! `history.go`'s query layer is ported too:
//!
//! - `Link::get_or_create`/`History::get_or_create` (Go: `GetOrCreateLink`/
//!   `GetOrCreateHistory`).
//! - `HistoryLink::{delete_by_user_and_url, delete_by_user_query_and_url,
//!   set_pinned, record_selection, urls_by_query, latest_items, timestamps,
//!   suggest_query}` (Go: `DeleteHistoryURL`/`DeleteHistoryItem`/
//!   `SetHistoryPinned`/`UpdateHistory`/`GetURLsByQuery`/
//!   `GetLatestHistoryItems(Filtered(ByDate))`/
//!   `GetHistoryItemTimestampsFilteredByDate`/`GetQuerySuggestion`).
//!   `record_selection`, not `update`, since `#[derive(Mapped)]` already
//!   generates an `update()` instance method a same-named associated
//!   function would collide with (the same reasoning behind
//!   `WebSession::refresh`). `latest_items` collapses Go's
//!   `GetLatestHistoryItems`/`GetLatestHistoryItemsFiltered`/
//!   `GetLatestHistoryItemsFilteredByDate` into one function taking a
//!   [`HistoryItemsFilter`] — those three Go functions are pure
//!   argument-forwarding wrappers around the same query with different
//!   defaults (Go has no default parameters), not three different
//!   behaviors: every Go call site's arguments map onto a field here, with
//!   `Default` covering whatever the simpler wrappers omitted.
//!
//! **`deleted`/`#[table(soft_delete)]` is never actually set by anything in
//! this file.** Hister's `CommonFields.DeletedAt` (embedded by `History`,
//! `Link`, and `HistoryLink`) is a plain `*time.Time`, not GORM's own
//! `gorm.DeletedAt` sentinel type — so GORM never recognizes it as a
//! soft-delete column, and every `DB.Delete(...)` call in `history.go`
//! (`DeleteHistoryURL`/`DeleteHistoryItem`) is a genuine hard delete.
//! `delete_by_user_and_url`/`delete_by_user_query_and_url` reproduce that
//! with a real `DELETE`, not an `UPDATE ... SET deleted = true`: besides
//! matching Go, soft-deleting here would also break re-recording history
//! for the same URL afterward, since `history_links`' unique index on
//! `(history_id, link_id)` isn't scoped to active rows. `get_or_create`'s
//! lookups and the existing-row checks in `record_selection`/
//! `set_pinned`'s pin branch still filter via `T::not_deleted_filter()`
//! (matching Go's model-aware `.Model(&T{})` queries there), which costs
//! nothing and stays correct if a soft-delete path is ever added later —
//! it just never excludes anything today.

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

/// A URL recorded against a query's history, ranked pinned-first (backs
/// rows 1.16-1.18's history results). Go's `Text`/`DocID` fields
/// (`gorm:"-"`, never persisted) are populated by the indexer layer at
/// query time — out of scope for this model-layer struct.
#[derive(Debug, Clone, PartialEq)]
pub struct UrlCount {
    pub url: String,
    pub title: String,
    pub count: i64,
    pub pinned: bool,
}

/// One row of a user's history/search-query timeline (backs row 1.16's
/// history listing).
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryItem {
    pub id: i64,
    pub query: String,
    pub title: String,
    pub url: String,
    pub updated_at: DateTime<Utc>,
}

/// Filters for [`HistoryLink::latest_items`] — see this module's doc
/// comment for why Go's three `GetLatestHistoryItems*` wrappers collapse
/// into this one struct plus function.
#[derive(Debug, Clone, Default)]
pub struct HistoryItemsFilter {
    /// Keyset cursor: only items before this id (used when
    /// `last_updated_at` is `None`).
    pub last_id: i64,
    /// Keyset cursor: only items strictly older than this timestamp, with
    /// `last_id` breaking ties at the same timestamp. Takes priority over
    /// `last_id` alone, matching Go's own `if/else if`.
    pub last_updated_at: Option<DateTime<Utc>>,
    /// Case-insensitive substring match against the item's title or URL;
    /// empty matches everything. `%`/`_`/`!` in this string are matched
    /// literally, not as SQL wildcards (see [`escape_like_pattern`]).
    pub filter: String,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
}

impl Link {
    /// Finds a link by URL, creating it if missing. If found but `title`
    /// differs from the stored one (and is non-empty), updates the title
    /// in place first (Go: `GetOrCreateLink`).
    pub async fn get_or_create(engine: &Engine, url: &str, title: &str) -> rusty_db::Result<Self> {
        let table = Self::table();
        let existing: Option<Self> = engine
            .fetch_optional_as(
                &Select::from(&table)
                    .filter(table.col("url").eq(url))
                    .filter(Self::not_deleted_filter().expect("Link has a soft_delete column")),
            )
            .await?;

        if let Some(mut link) = existing {
            if !title.is_empty() && link.title != title {
                let now = Utc::now();
                engine
                    .execute(
                        &Update::table(&table)
                            .set("title", title)
                            .set("updated_at", now)
                            .filter(table.col("id").eq(link.id)),
                    )
                    .await?;
                link.title = title.to_string();
                link.updated_at = now;
            }
            return Ok(link);
        }

        let now = Utc::now();
        let dialect = engine.dialect();
        let p = crate::placeholders(dialect, 4);
        let params: Vec<Value> = vec![
            url.to_string().into(),
            title.to_string().into(),
            now.into(),
            now.into(),
        ];
        let mut conn = engine.connect().await?;
        let id: i64 = if dialect.supports_returning() {
            let sql = format!(
                "INSERT INTO links (url, title, created_at, updated_at) \
                 VALUES ({}, {}, {}, {}) RETURNING id",
                p[0], p[1], p[2], p[3]
            );
            conn.fetch_one(&sql, &params).await?.get_by_name("id")?
        } else {
            let sql = format!(
                "INSERT INTO links (url, title, created_at, updated_at) VALUES ({}, {}, {}, {})",
                p[0], p[1], p[2], p[3]
            );
            conn.execute(&sql, &params).await?;
            conn.fetch_one("SELECT last_insert_rowid() AS id", &[])
                .await?
                .get_by_name("id")?
        };

        Ok(Self {
            id,
            created_at: now,
            updated_at: now,
            deleted: false,
            url: url.to_string(),
            title: title.to_string(),
        })
    }
}

impl History {
    /// Finds `user_id`'s history record for `query`, creating it if
    /// missing (Go: `GetOrCreateHistory`).
    pub async fn get_or_create(
        engine: &Engine,
        user_id: i64,
        query: &str,
    ) -> rusty_db::Result<Self> {
        let table = Self::table();
        let existing: Option<Self> = engine
            .fetch_optional_as(
                &Select::from(&table)
                    .filter(table.col("user_id").eq(user_id))
                    .filter(table.col("query").eq(query))
                    .filter(Self::not_deleted_filter().expect("History has a soft_delete column")),
            )
            .await?;
        if let Some(history) = existing {
            return Ok(history);
        }

        let now = Utc::now();
        let dialect = engine.dialect();
        let p = crate::placeholders(dialect, 4);
        let params: Vec<Value> = vec![
            user_id.into(),
            query.to_string().into(),
            now.into(),
            now.into(),
        ];
        let mut conn = engine.connect().await?;
        let id: i64 = if dialect.supports_returning() {
            let sql = format!(
                "INSERT INTO histories (user_id, query, created_at, updated_at) \
                 VALUES ({}, {}, {}, {}) RETURNING id",
                p[0], p[1], p[2], p[3]
            );
            conn.fetch_one(&sql, &params).await?.get_by_name("id")?
        } else {
            let sql = format!(
                "INSERT INTO histories (user_id, query, created_at, updated_at) \
                 VALUES ({}, {}, {}, {})",
                p[0], p[1], p[2], p[3]
            );
            conn.execute(&sql, &params).await?;
            conn.fetch_one("SELECT last_insert_rowid() AS id", &[])
                .await?
                .get_by_name("id")?
        };

        Ok(Self {
            id,
            created_at: now,
            updated_at: now,
            deleted: false,
            user_id,
            query: query.to_string(),
        })
    }
}

impl HistoryLink {
    /// Inserts a fresh join row with a database-assigned id — shared by
    /// `record_selection`'s and `set_pinned`'s create branches. Named
    /// `insert_new`, not `insert`, since `#[derive(Mapped)]` already
    /// generates an `insert()` instance method a same-named associated
    /// function would collide with.
    async fn insert_new(
        engine: &Engine,
        history_id: i64,
        link_id: i64,
        count: i64,
        pinned: bool,
    ) -> rusty_db::Result<i64> {
        let now = Utc::now();
        let dialect = engine.dialect();
        let p = crate::placeholders(dialect, 6);
        let params: Vec<Value> = vec![
            history_id.into(),
            link_id.into(),
            count.into(),
            pinned.into(),
            now.into(),
            now.into(),
        ];
        let mut conn = engine.connect().await?;
        if dialect.supports_returning() {
            let sql = format!(
                "INSERT INTO history_links \
                    (history_id, link_id, count, pinned, created_at, updated_at) \
                 VALUES ({}, {}, {}, {}, {}, {}) RETURNING id",
                p[0], p[1], p[2], p[3], p[4], p[5]
            );
            conn.fetch_one(&sql, &params).await?.get_by_name("id")
        } else {
            let sql = format!(
                "INSERT INTO history_links \
                    (history_id, link_id, count, pinned, created_at, updated_at) \
                 VALUES ({}, {}, {}, {}, {}, {})",
                p[0], p[1], p[2], p[3], p[4], p[5]
            );
            conn.execute(&sql, &params).await?;
            conn.fetch_one("SELECT last_insert_rowid() AS id", &[])
                .await?
                .get_by_name("id")
        }
    }

    /// Deletes every history-link row associating `user_id`'s history
    /// entries with `url`, across every query (Go: `DeleteHistoryURL`).
    /// This is a genuine hard delete, matching `DB.Delete(&HistoryLink{},
    /// ...)`: Hister's `CommonFields.DeletedAt` is a plain `*time.Time`,
    /// not GORM's own `gorm.DeletedAt` sentinel type, so GORM never
    /// recognizes it as a soft-delete column and every `Delete` call
    /// across the Go codebase removes rows outright — the `deleted`
    /// column this crate's schema increment gave these tables (mirroring
    /// the *shape* of `CommonFields`) is accordingly never set by anything
    /// in this file. A real soft delete here would also break re-recording
    /// history for the same URL afterward, since `history_links`' unique
    /// index on `(history_id, link_id)` isn't scoped to active rows.
    pub async fn delete_by_user_and_url(
        engine: &Engine,
        user_id: i64,
        url: &str,
    ) -> rusty_db::Result<()> {
        let hl = Self::table();
        let histories = History::table();
        let links = Link::table();
        let matching = Select::from(&hl)
            .columns([hl.col("id")])
            .join(
                &histories,
                hl.col("history_id").eq_col(&histories.col("id")),
            )
            .join(&links, hl.col("link_id").eq_col(&links.col("id")))
            .filter(histories.col("user_id").eq(user_id))
            .filter(links.col("url").eq(url));
        engine
            .execute(&Delete::from(&hl).filter(hl.col("id").in_subquery(matching)))
            .await?;
        Ok(())
    }

    /// Deletes the history-link row for one specific (user, query, url)
    /// triple (Go: `DeleteHistoryItem`).
    pub async fn delete_by_user_query_and_url(
        engine: &Engine,
        user_id: i64,
        query: &str,
        url: &str,
    ) -> rusty_db::Result<()> {
        let hl = Self::table();
        let histories = History::table();
        let links = Link::table();
        let matching = Select::from(&hl)
            .columns([hl.col("id")])
            .join(
                &histories,
                hl.col("history_id").eq_col(&histories.col("id")),
            )
            .join(&links, hl.col("link_id").eq_col(&links.col("id")))
            .filter(histories.col("user_id").eq(user_id))
            .filter(histories.col("query").eq(query))
            .filter(links.col("url").eq(url));
        engine
            .execute(&Delete::from(&hl).filter(hl.col("id").in_subquery(matching)))
            .await?;
        Ok(())
    }

    /// Pins or unpins `url` within `query`'s history for `user_id` (Go:
    /// `SetHistoryPinned`). Unpinning only ever touches an existing row
    /// (Go's `UpdateColumn`, which — unlike `record_selection`'s `Save` —
    /// skips the `updated_at` bump); pinning creates the link/history/join
    /// rows as needed, the same way `record_selection` does.
    pub async fn set_pinned(
        engine: &Engine,
        user_id: i64,
        query: &str,
        url: &str,
        title: &str,
        pinned: bool,
    ) -> rusty_db::Result<()> {
        if query.is_empty() || url.is_empty() {
            return Err(rusty_db::Error::QueryBuilder("missing data".to_string()));
        }

        if !pinned {
            let hl = Self::table();
            let histories = History::table();
            let links = Link::table();
            let matching = Select::from(&hl)
                .columns([hl.col("id")])
                .join(
                    &histories,
                    hl.col("history_id").eq_col(&histories.col("id")),
                )
                .join(&links, hl.col("link_id").eq_col(&links.col("id")))
                .filter(histories.col("user_id").eq(user_id))
                .filter(histories.col("query").eq(query))
                .filter(links.col("url").eq(url));
            engine
                .execute(
                    &Update::table(&hl)
                        .set("pinned", false)
                        .filter(hl.col("id").in_subquery(matching)),
                )
                .await?;
            return Ok(());
        }

        let link = Link::get_or_create(engine, url, title).await?;
        let history = History::get_or_create(engine, user_id, query).await?;

        let table = Self::table();
        let existing: Option<Self> = engine
            .fetch_optional_as(
                &Select::from(&table)
                    .filter(table.col("history_id").eq(history.id))
                    .filter(table.col("link_id").eq(link.id))
                    .filter(
                        Self::not_deleted_filter().expect("HistoryLink has a soft_delete column"),
                    ),
            )
            .await?;

        match existing {
            Some(hl) => {
                engine
                    .execute(
                        &Update::table(&table)
                            .set("pinned", true)
                            .set("updated_at", Utc::now())
                            .filter(table.col("id").eq(hl.id)),
                    )
                    .await?;
            }
            None => {
                Self::insert_new(engine, history.id, link.id, 1, true).await?;
            }
        }
        Ok(())
    }

    /// Records that `url` (creating the link/history rows as needed) was
    /// selected from `query`'s results for `user_id`, incrementing its hit
    /// count (Go: `UpdateHistory`). Named `record_selection`, not
    /// `update`, since `#[derive(Mapped)]` already generates an `update()`
    /// instance method a same-named associated function would collide
    /// with.
    pub async fn record_selection(
        engine: &Engine,
        user_id: i64,
        query: &str,
        url: &str,
        title: &str,
    ) -> rusty_db::Result<()> {
        if query.is_empty() || url.is_empty() || title.is_empty() {
            return Err(rusty_db::Error::QueryBuilder("missing data".to_string()));
        }

        let link = Link::get_or_create(engine, url, title).await?;
        let history = History::get_or_create(engine, user_id, query).await?;

        let table = Self::table();
        let existing: Option<Self> = engine
            .fetch_optional_as(
                &Select::from(&table)
                    .filter(table.col("history_id").eq(history.id))
                    .filter(table.col("link_id").eq(link.id))
                    .filter(
                        Self::not_deleted_filter().expect("HistoryLink has a soft_delete column"),
                    ),
            )
            .await?;

        match existing {
            Some(hl) => {
                engine
                    .execute(
                        &Update::table(&table)
                            .set("count", hl.count + 1)
                            .set("updated_at", Utc::now())
                            .filter(table.col("id").eq(hl.id)),
                    )
                    .await?;
            }
            None => {
                Self::insert_new(engine, history.id, link.id, 1, false).await?;
            }
        }
        Ok(())
    }

    /// Returns up to 20 URLs recorded for `query`, ranked pinned-first,
    /// then by hit count, then by recency (Go: `GetURLsByQuery`).
    pub async fn urls_by_query(
        engine: &Engine,
        user_id: i64,
        query: &str,
    ) -> rusty_db::Result<Vec<UrlCount>> {
        let hl = Self::table();
        let links = Link::table();
        let histories = History::table();
        let rows = engine
            .fetch_all(
                &Select::from(&hl)
                    .columns([
                        SelectExpr::from(links.col("url")).alias("url"),
                        SelectExpr::from(links.col("title")).alias("title"),
                        SelectExpr::from(hl.col("count")).alias("count"),
                        SelectExpr::from(hl.col("pinned")).alias("pinned"),
                    ])
                    .join(&links, hl.col("link_id").eq_col(&links.col("id")))
                    .join(
                        &histories,
                        hl.col("history_id").eq_col(&histories.col("id")),
                    )
                    .filter(histories.col("user_id").eq(user_id))
                    .filter(histories.col("query").eq(query))
                    .order_by((hl.col("pinned"), false))
                    .order_by((hl.col("count"), false))
                    .order_by((hl.col("updated_at"), false))
                    .limit(20),
            )
            .await?;

        rows.into_iter()
            .map(|row| {
                Ok(UrlCount {
                    url: row.get_by_name("url")?,
                    title: row.get_by_name("title")?,
                    count: row.get_by_name("count")?,
                    pinned: row.get_by_name("pinned")?,
                })
            })
            .collect()
    }

    /// Returns the most recent history items for `user_id`, most-recent
    /// first with a stable `(updated_at, id)` keyset cursor for
    /// pagination, optionally restricted to a date range and/or a
    /// case-insensitive title/URL substring filter — see this module's
    /// doc comment and [`HistoryItemsFilter`] for how Go's three
    /// `GetLatestHistoryItems*` wrappers collapse into this one function.
    pub async fn latest_items(
        engine: &Engine,
        user_id: i64,
        limit: i64,
        filter: &HistoryItemsFilter,
    ) -> rusty_db::Result<Vec<HistoryItem>> {
        let hl = HistoryLink::table();
        let links = Link::table();
        let histories = History::table();
        let mut query = filtered_history_items(user_id, &filter.filter)
            .columns([
                SelectExpr::from(hl.col("id")).alias("id"),
                SelectExpr::from(links.col("url")).alias("url"),
                SelectExpr::from(links.col("title")).alias("title"),
                SelectExpr::from(histories.col("query")).alias("query"),
                SelectExpr::from(hl.col("updated_at")).alias("updated_at"),
            ])
            .order_by((hl.col("updated_at"), false))
            .order_by((hl.col("id"), false));

        if let Some(date_from) = filter.date_from {
            query = query.filter(hl.col("updated_at").gte(date_from));
        }
        if let Some(date_to) = filter.date_to {
            query = query.filter(hl.col("updated_at").lt(date_to));
        }
        if let Some(last_updated_at) = filter.last_updated_at {
            query = query.filter(
                hl.col("updated_at").lt(last_updated_at).or(hl
                    .col("updated_at")
                    .eq(last_updated_at)
                    .and(hl.col("id").lt(filter.last_id))),
            );
        } else if filter.last_id > 0 {
            query = query.filter(hl.col("id").lt(filter.last_id));
        }

        let rows = engine.fetch_all(&query.limit(limit)).await?;
        rows.into_iter()
            .map(|row| {
                Ok(HistoryItem {
                    id: row.get_by_name("id")?,
                    query: row.get_by_name("query")?,
                    title: row.get_by_name("title")?,
                    url: row.get_by_name("url")?,
                    updated_at: row.get_by_name("updated_at")?,
                })
            })
            .collect()
    }

    /// Returns the `updated_at` timestamps of history items matching
    /// `filter`/the given date range — the raw per-item timestamps the
    /// timeline endpoint (row 1.17, `server/timeline`) buckets into
    /// hierarchical date counts; that bucketing itself is server-layer
    /// behavior, out of scope for this crate (Go:
    /// `GetHistoryItemTimestampsFilteredByDate`).
    pub async fn timestamps(
        engine: &Engine,
        user_id: i64,
        filter: &str,
        date_from: Option<DateTime<Utc>>,
        date_to: Option<DateTime<Utc>>,
    ) -> rusty_db::Result<Vec<DateTime<Utc>>> {
        let hl = HistoryLink::table();
        let mut query = filtered_history_items(user_id, filter)
            .columns([SelectExpr::from(hl.col("updated_at")).alias("updated_at")]);
        if let Some(date_from) = date_from {
            query = query.filter(hl.col("updated_at").gte(date_from));
        }
        if let Some(date_to) = date_to {
            query = query.filter(hl.col("updated_at").lt(date_to));
        }

        let rows = engine.fetch_all(&query).await?;
        rows.into_iter()
            .map(|row| row.get_by_name("updated_at"))
            .collect()
    }

    /// Suggests the most-used query starting with `q` (case-insensitive
    /// prefix match — `%`/`_` wildcards inside `q` itself are not escaped,
    /// matching Go's own unescaped `LOWER(query) LIKE ?` here) for
    /// `user_id`, ranked by hit count (Go: `GetQuerySuggestion`, which
    /// returns `""` on no match; `None` is the idiomatic equivalent).
    pub async fn suggest_query(
        engine: &Engine,
        user_id: i64,
        q: &str,
    ) -> rusty_db::Result<Option<String>> {
        let hl = Self::table();
        let histories = History::table();
        let pattern = format!("{}%", q.to_lowercase());
        let row = engine
            .fetch_optional(
                &Select::from(&hl)
                    .columns([SelectExpr::from(histories.col("query")).alias("query")])
                    .join(
                        &histories,
                        hl.col("history_id").eq_col(&histories.col("id")),
                    )
                    .filter(histories.col("user_id").eq(user_id))
                    .filter(Expr::col(histories.col("query")).lower().like(pattern))
                    .order_by((hl.col("count"), false))
                    .limit(1),
            )
            .await?;
        row.map(|r| r.get_by_name("query")).transpose()
    }
}

/// The base joined-and-filtered query every `HistoryLink::{latest_items,
/// timestamps}` call builds on (Go: the private `filteredHistoryItems`),
/// scoped to `user_id`'s history and — when `filter` is non-blank — a
/// case-insensitive substring match against the item's title or URL.
fn filtered_history_items(user_id: i64, filter: &str) -> Select {
    let hl = HistoryLink::table();
    let links = Link::table();
    let histories = History::table();
    let mut query = Select::from(&hl)
        .join(&links, hl.col("link_id").eq_col(&links.col("id")))
        .join(
            &histories,
            hl.col("history_id").eq_col(&histories.col("id")),
        )
        .filter(histories.col("user_id").eq(user_id));

    let trimmed = filter.trim();
    if !trimmed.is_empty() {
        let pattern = format!("%{}%", escape_like_pattern(trimmed));
        query = query.filter(Expr::text(
            "(LOWER(links.title) LIKE ? ESCAPE '!' OR LOWER(links.url) LIKE ? ESCAPE '!')",
            [pattern.clone().into(), pattern.into()],
        ));
    }
    query
}

/// Escapes `!`, `%`, and `_` for a `LIKE ... ESCAPE '!'` pattern, matching
/// Go's `strings.NewReplacer("!", "!!", "%", "!%", "_", "!_")` — a single
/// simultaneous pass over a fixed, non-overlapping character set, so
/// scanning char-by-char reproduces it exactly (none of the replacement
/// outputs contain a character that itself needs replacing).
fn escape_like_pattern(filter: &str) -> String {
    let mut escaped = String::with_capacity(filter.len());
    for c in filter.to_lowercase().chars() {
        match c {
            '!' => escaped.push_str("!!"),
            '%' => escaped.push_str("!%"),
            '_' => escaped.push_str("!_"),
            other => escaped.push(other),
        }
    }
    escaped
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

    #[tokio::test]
    async fn link_get_or_create_creates_a_new_link() {
        let engine = migrated_engine().await;
        let link = Link::get_or_create(&engine, "https://example.com", "Example")
            .await
            .unwrap();
        assert_eq!(link.url, "https://example.com");
        assert_eq!(link.title, "Example");
        assert!(link.id > 0);
    }

    #[tokio::test]
    async fn link_get_or_create_returns_the_existing_link_by_url() {
        let engine = migrated_engine().await;
        let first = Link::get_or_create(&engine, "https://example.com", "Example")
            .await
            .unwrap();
        let second = Link::get_or_create(&engine, "https://example.com", "Example")
            .await
            .unwrap();
        assert_eq!(first.id, second.id);
    }

    #[tokio::test]
    async fn link_get_or_create_updates_the_title_when_it_changes() {
        let engine = migrated_engine().await;
        let first = Link::get_or_create(&engine, "https://example.com", "Old Title")
            .await
            .unwrap();
        let updated = Link::get_or_create(&engine, "https://example.com", "New Title")
            .await
            .unwrap();
        assert_eq!(updated.id, first.id);
        assert_eq!(updated.title, "New Title");
    }

    #[tokio::test]
    async fn link_get_or_create_keeps_the_title_when_the_new_one_is_empty() {
        let engine = migrated_engine().await;
        let first = Link::get_or_create(&engine, "https://example.com", "Old Title")
            .await
            .unwrap();
        let unchanged = Link::get_or_create(&engine, "https://example.com", "")
            .await
            .unwrap();
        assert_eq!(unchanged.id, first.id);
        assert_eq!(unchanged.title, "Old Title");
    }

    #[tokio::test]
    async fn history_get_or_create_creates_and_reuses_a_history_row() {
        let engine = migrated_engine().await;
        let first = History::get_or_create(&engine, 1, "rust ownership")
            .await
            .unwrap();
        let second = History::get_or_create(&engine, 1, "rust ownership")
            .await
            .unwrap();
        assert_eq!(first.id, second.id);

        let other_user = History::get_or_create(&engine, 2, "rust ownership")
            .await
            .unwrap();
        assert_ne!(first.id, other_user.id);
    }

    #[tokio::test]
    async fn record_selection_creates_a_history_link_with_count_one() {
        let engine = migrated_engine().await;
        HistoryLink::record_selection(&engine, 1, "go", "https://example.com/go", "Go")
            .await
            .unwrap();

        let urls = HistoryLink::urls_by_query(&engine, 1, "go").await.unwrap();
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].url, "https://example.com/go");
        assert_eq!(urls[0].count, 1);
        assert!(!urls[0].pinned);
    }

    #[tokio::test]
    async fn record_selection_increments_the_count_on_repeat_selection() {
        let engine = migrated_engine().await;
        for _ in 0..3 {
            HistoryLink::record_selection(&engine, 1, "go", "https://example.com/go", "Go")
                .await
                .unwrap();
        }

        let urls = HistoryLink::urls_by_query(&engine, 1, "go").await.unwrap();
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].count, 3);
    }

    #[tokio::test]
    async fn record_selection_rejects_missing_data() {
        let engine = migrated_engine().await;
        assert!(
            HistoryLink::record_selection(&engine, 1, "", "https://example.com", "T")
                .await
                .is_err()
        );
        assert!(HistoryLink::record_selection(&engine, 1, "q", "", "T")
            .await
            .is_err());
        assert!(
            HistoryLink::record_selection(&engine, 1, "q", "https://example.com", "")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn set_pinned_creates_a_pinned_history_link_when_none_exists() {
        let engine = migrated_engine().await;
        HistoryLink::set_pinned(&engine, 1, "go", "https://example.com/go", "Go", true)
            .await
            .unwrap();

        let urls = HistoryLink::urls_by_query(&engine, 1, "go").await.unwrap();
        assert_eq!(urls.len(), 1);
        assert!(urls[0].pinned);
        assert_eq!(urls[0].count, 1);
    }

    #[tokio::test]
    async fn set_pinned_pins_an_existing_unpinned_history_link_without_resetting_count() {
        let engine = migrated_engine().await;
        for _ in 0..2 {
            HistoryLink::record_selection(&engine, 1, "go", "https://example.com/go", "Go")
                .await
                .unwrap();
        }

        HistoryLink::set_pinned(&engine, 1, "go", "https://example.com/go", "Go", true)
            .await
            .unwrap();

        let urls = HistoryLink::urls_by_query(&engine, 1, "go").await.unwrap();
        assert_eq!(urls.len(), 1);
        assert!(urls[0].pinned);
        assert_eq!(urls[0].count, 2);
    }

    #[tokio::test]
    async fn set_pinned_false_unpins_without_creating_a_row() {
        let engine = migrated_engine().await;
        HistoryLink::set_pinned(&engine, 1, "go", "https://example.com/go", "Go", false)
            .await
            .unwrap();
        let urls = HistoryLink::urls_by_query(&engine, 1, "go").await.unwrap();
        assert!(urls.is_empty());
    }

    #[tokio::test]
    async fn set_pinned_false_unpins_a_pinned_row() {
        let engine = migrated_engine().await;
        HistoryLink::set_pinned(&engine, 1, "go", "https://example.com/go", "Go", true)
            .await
            .unwrap();
        HistoryLink::set_pinned(&engine, 1, "go", "https://example.com/go", "Go", false)
            .await
            .unwrap();

        let urls = HistoryLink::urls_by_query(&engine, 1, "go").await.unwrap();
        assert_eq!(urls.len(), 1);
        assert!(!urls[0].pinned);
    }

    #[tokio::test]
    async fn set_pinned_rejects_missing_data() {
        let engine = migrated_engine().await;
        assert!(
            HistoryLink::set_pinned(&engine, 1, "", "https://example.com", "T", true)
                .await
                .is_err()
        );
        assert!(HistoryLink::set_pinned(&engine, 1, "q", "", "T", true)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn delete_by_user_and_url_removes_the_url_across_every_query() {
        let engine = migrated_engine().await;
        HistoryLink::record_selection(&engine, 1, "go", "https://example.com/go", "Go")
            .await
            .unwrap();
        HistoryLink::record_selection(&engine, 1, "golang", "https://example.com/go", "Go")
            .await
            .unwrap();

        HistoryLink::delete_by_user_and_url(&engine, 1, "https://example.com/go")
            .await
            .unwrap();

        assert!(HistoryLink::urls_by_query(&engine, 1, "go")
            .await
            .unwrap()
            .is_empty());
        assert!(HistoryLink::urls_by_query(&engine, 1, "golang")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn delete_by_user_query_and_url_removes_only_the_matching_row() {
        let engine = migrated_engine().await;
        HistoryLink::record_selection(&engine, 1, "go", "https://example.com/go", "Go")
            .await
            .unwrap();
        HistoryLink::record_selection(&engine, 1, "golang", "https://example.com/go", "Go")
            .await
            .unwrap();

        HistoryLink::delete_by_user_query_and_url(&engine, 1, "go", "https://example.com/go")
            .await
            .unwrap();

        assert!(HistoryLink::urls_by_query(&engine, 1, "go")
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            HistoryLink::urls_by_query(&engine, 1, "golang")
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn urls_by_query_ranks_pinned_first_then_by_count() {
        let engine = migrated_engine().await;
        HistoryLink::record_selection(&engine, 1, "rust", "https://example.com/a", "A")
            .await
            .unwrap();
        for _ in 0..5 {
            HistoryLink::record_selection(&engine, 1, "rust", "https://example.com/b", "B")
                .await
                .unwrap();
        }
        HistoryLink::set_pinned(&engine, 1, "rust", "https://example.com/c", "C", true)
            .await
            .unwrap();

        let urls = HistoryLink::urls_by_query(&engine, 1, "rust")
            .await
            .unwrap();
        assert_eq!(urls.len(), 3);
        assert_eq!(urls[0].url, "https://example.com/c");
        assert_eq!(urls[1].url, "https://example.com/b");
        assert_eq!(urls[2].url, "https://example.com/a");
    }

    #[tokio::test]
    async fn latest_items_returns_newest_first() {
        let engine = migrated_engine().await;
        HistoryLink::record_selection(&engine, 0, "go", "https://example.com/go", "Golang")
            .await
            .unwrap();
        HistoryLink::record_selection(&engine, 0, "rust", "https://example.com/rust", "Rust")
            .await
            .unwrap();

        let items = HistoryLink::latest_items(&engine, 0, 100, &HistoryItemsFilter::default())
            .await
            .unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].url, "https://example.com/rust");
        assert_eq!(items[1].url, "https://example.com/go");
    }

    #[tokio::test]
    async fn latest_items_filters_by_title_or_url_case_insensitively() {
        let engine = migrated_engine().await;
        HistoryLink::record_selection(
            &engine,
            0,
            "go",
            "https://example.com/go",
            "Golang Test Guide",
        )
        .await
        .unwrap();
        HistoryLink::record_selection(
            &engine,
            0,
            "docs",
            "https://docs.example.com/rust",
            "Rust Guide",
        )
        .await
        .unwrap();

        let by_title = HistoryLink::latest_items(
            &engine,
            0,
            100,
            &HistoryItemsFilter {
                filter: "GOLANG TEST".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(by_title.len(), 1);
        assert_eq!(by_title[0].url, "https://example.com/go");

        let by_url = HistoryLink::latest_items(
            &engine,
            0,
            100,
            &HistoryItemsFilter {
                filter: "DOCS.EXAMPLE.COM".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(by_url.len(), 1);
        assert_eq!(by_url[0].url, "https://docs.example.com/rust");
    }

    #[tokio::test]
    async fn latest_items_treats_sql_wildcards_in_the_filter_literally() {
        let engine = migrated_engine().await;
        HistoryLink::record_selection(
            &engine,
            0,
            "coverage",
            "https://example.com/coverage",
            "100% coverage",
        )
        .await
        .unwrap();
        HistoryLink::record_selection(&engine, 0, "other", "https://example.com/other", "Other")
            .await
            .unwrap();

        let items = HistoryLink::latest_items(
            &engine,
            0,
            100,
            &HistoryItemsFilter {
                filter: "%".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].url, "https://example.com/coverage");
    }

    #[tokio::test]
    async fn latest_items_paginates_with_a_stable_updated_at_id_cursor() {
        let engine = migrated_engine().await;
        HistoryLink::record_selection(&engine, 0, "newer", "https://example.com/newer", "Newer")
            .await
            .unwrap();
        HistoryLink::record_selection(&engine, 0, "older", "https://example.com/older", "Older")
            .await
            .unwrap();

        let first_page = HistoryLink::latest_items(&engine, 0, 1, &HistoryItemsFilter::default())
            .await
            .unwrap();
        assert_eq!(first_page.len(), 1);
        assert_eq!(first_page[0].url, "https://example.com/older");

        let second_page = HistoryLink::latest_items(
            &engine,
            0,
            1,
            &HistoryItemsFilter {
                last_id: first_page[0].id,
                last_updated_at: Some(first_page[0].updated_at),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(second_page.len(), 1);
        assert_eq!(second_page[0].url, "https://example.com/newer");
    }

    #[tokio::test]
    async fn latest_items_filters_by_date_range() {
        let engine = migrated_engine().await;
        HistoryLink::record_selection(&engine, 0, "go", "https://example.com/go", "Go")
            .await
            .unwrap();

        let in_range = HistoryLink::latest_items(
            &engine,
            0,
            100,
            &HistoryItemsFilter {
                date_from: Some("2000-01-01T00:00:00Z".parse().unwrap()),
                date_to: Some("2100-01-01T00:00:00Z".parse().unwrap()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(in_range.len(), 1);

        let out_of_range = HistoryLink::latest_items(
            &engine,
            0,
            100,
            &HistoryItemsFilter {
                date_from: Some("2000-01-01T00:00:00Z".parse().unwrap()),
                date_to: Some("2001-01-01T00:00:00Z".parse().unwrap()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(out_of_range.is_empty());
    }

    #[tokio::test]
    async fn timestamps_returns_the_updated_at_of_matching_items() {
        let engine = migrated_engine().await;
        HistoryLink::record_selection(&engine, 0, "go", "https://example.com/go", "Go")
            .await
            .unwrap();

        let timestamps = HistoryLink::timestamps(&engine, 0, "", None, None)
            .await
            .unwrap();
        assert_eq!(timestamps.len(), 1);
    }

    #[tokio::test]
    async fn suggest_query_returns_the_most_used_query_matching_the_prefix() {
        let engine = migrated_engine().await;
        for _ in 0..3 {
            HistoryLink::record_selection(&engine, 0, "rust ownership", "https://a", "A")
                .await
                .unwrap();
        }
        HistoryLink::record_selection(&engine, 0, "rust macros", "https://b", "B")
            .await
            .unwrap();

        let suggestion = HistoryLink::suggest_query(&engine, 0, "RUST")
            .await
            .unwrap();
        assert_eq!(suggestion, Some("rust ownership".to_string()));
    }

    #[tokio::test]
    async fn suggest_query_returns_none_when_nothing_matches() {
        let engine = migrated_engine().await;
        HistoryLink::record_selection(&engine, 0, "rust ownership", "https://a", "A")
            .await
            .unwrap();

        let suggestion = HistoryLink::suggest_query(&engine, 0, "python")
            .await
            .unwrap();
        assert_eq!(suggestion, None);
    }
}
