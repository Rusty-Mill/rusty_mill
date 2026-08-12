//! A [`SearchBackend`] implementation backed by SQLite's
//! [FTS5](https://www.sqlite.org/fts5.html) virtual table module - genuinely
//! embedded like [`rusty-search-tantivy`](https://docs.rs/rusty-search-tantivy),
//! but via SQL rather than an inverted-index library. Bundles its own SQLite
//! (via [`rusty_sqlite`](https://github.com/baileyrd/rusty_sqlite), a thin
//! wrapper over `rusqlite`'s `bundled` feature that also applies sane
//! connection pragmas - WAL journaling, foreign key enforcement, a busy
//! timeout - this crate didn't set on its own before adopting it), so no
//! system SQLite install is required.
//!
//! Use [`SqliteFts5Backend::in_memory`] for an ephemeral, process-local
//! database, or [`SqliteFts5Backend::on_disk`] to persist each index as its
//! own `<dir>/<name>.sqlite3` file.
//!
//! Each index is backed by two SQL objects sharing `rowid` values within one
//! SQLite connection: a `content` table with one real, typed column per
//! schema field (so `Query::Term`/`Query::Range` and [`Sort::Field`] all
//! translate to ordinary SQL - no fast-field opt-in needed, unlike
//! `rusty-search-tantivy`), and, when the schema has any `Text` fields, an
//! `idx_fts` FTS5 virtual table shadowing them for full-text search and
//! `bm25()` relevance scoring.
//!
//! ## Known limitations
//!
//! - At most one `Query::Match` clause is supported per search - see
//!   [`query_map::compile`] and ADR-0008 for why. `must_not` wrapping a bare
//!   `Query::MatchAll`/`Query::Match` (which `rusty-search-meilisearch`/
//!   `rusty-search-algolia` reject) works fine here, and every other
//!   `Query::Term`/`Query::Range`/`Query::Bool` combination is supported
//!   without restriction.
//! - `Query::Bool`'s `should` clauses are dropped (contribute neither
//!   filtering nor scoring) whenever they share a `Bool` node with a
//!   non-empty `must`/`filter`, matching `Query`'s documented "should is
//!   only required when must is empty" semantics literally rather than
//!   also threading them into `bm25` scoring.
//! - One SQLite connection per index, guarded by a `std::sync::Mutex` - all
//!   reads and writes through this backend serialize on it. There's no
//!   support for a second process (or a second `SqliteFts5Backend`
//!   instance) attaching to the same on-disk file concurrently.
//! - `SqliteFts5Backend::on_disk` does not reopen indices that already
//!   exist on disk from a previous process - `create_index` always creates
//!   a fresh `.sqlite3` file and errors if one already exists, matching
//!   `rusty-search-tantivy::TantivyBackend::on_disk`'s same documented
//!   limitation.
//! - No vector/hybrid search yet ([`SearchBackend::supports_vector_search`]
//!   returns `false`); `SearchRequest::vector` is rejected. See ADR-0008.

mod convert;
mod query_map;
mod schema_map;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use rusty_sqlite::rusqlite::params;
use tokio::sync::RwLock;

use rusty_search_core::{
    Document, Hit, Result, Schema as CoreSchema, SearchBackend, SearchError, SearchRequest,
    SearchResults,
};

use schema_map::{quote_ident, FieldMeta};

struct IndexHandle {
    conn: StdMutex<rusty_sqlite::Connection>,
    fields: HashMap<String, FieldMeta>,
    /// Every schema field, in declared order - the column order `content`
    /// `INSERT`s bind against.
    field_order: Vec<String>,
    /// The subset of `field_order` with an `idx_fts` column, in the same
    /// relative order the FTS5 table's columns were declared in.
    fts_field_order: Vec<String>,
    has_fts: bool,
}

/// A SQLite/FTS5-backed [`SearchBackend`]. Cheaply cloneable - clones share
/// the same underlying indices via an `Arc`.
#[derive(Clone)]
pub struct SqliteFts5Backend {
    data_dir: Option<PathBuf>,
    indices: Arc<RwLock<HashMap<String, Arc<IndexHandle>>>>,
}

impl SqliteFts5Backend {
    /// Creates a backend whose indices live entirely in memory and vanish
    /// when the backend is dropped.
    pub fn in_memory() -> Self {
        Self {
            data_dir: None,
            indices: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Creates a backend that persists each index as its own
    /// `dir/<index name>.sqlite3` file.
    pub fn on_disk(dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: Some(dir.into()),
            indices: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for SqliteFts5Backend {
    fn default() -> Self {
        Self::in_memory()
    }
}

fn backend_err(e: rusty_sqlite::rusqlite::Error) -> SearchError {
    SearchError::Backend(anyhow::Error::new(e))
}

fn rusty_sqlite_err(e: rusty_sqlite::Error) -> SearchError {
    SearchError::Backend(anyhow::Error::new(e))
}

#[async_trait]
impl SearchBackend for SqliteFts5Backend {
    async fn create_index(&self, name: &str, schema: CoreSchema) -> Result<()> {
        let mut indices = self.indices.write().await;
        if indices.contains_key(name) {
            return Err(SearchError::IndexAlreadyExists(name.to_string()));
        }

        let conn = match &self.data_dir {
            None => rusty_sqlite::Connection::open_in_memory().map_err(rusty_sqlite_err)?,
            Some(dir) => {
                let path = dir.join(format!("{name}.sqlite3"));
                if path.exists() {
                    return Err(SearchError::IndexAlreadyExists(name.to_string()));
                }
                std::fs::create_dir_all(dir).map_err(|e| SearchError::Backend(e.into()))?;
                rusty_sqlite::Connection::open(&path).map_err(rusty_sqlite_err)?
            }
        };

        let fields = schema_map::create_tables(conn.as_raw(), &schema)?;
        let field_order: Vec<String> = schema.fields.iter().map(|f| f.name.clone()).collect();
        let fts_field_order: Vec<String> = field_order
            .iter()
            .filter(|name| fields[name.as_str()].fts_indexed)
            .cloned()
            .collect();
        let has_fts = !fts_field_order.is_empty();

        indices.insert(
            name.to_string(),
            Arc::new(IndexHandle {
                conn: StdMutex::new(conn),
                fields,
                field_order,
                fts_field_order,
                has_fts,
            }),
        );
        Ok(())
    }

    async fn delete_index(&self, name: &str) -> Result<()> {
        let mut indices = self.indices.write().await;
        indices
            .remove(name)
            .ok_or_else(|| SearchError::IndexNotFound(name.to_string()))?;
        if let Some(dir) = &self.data_dir {
            let _ = std::fs::remove_file(dir.join(format!("{name}.sqlite3")));
        }
        Ok(())
    }

    async fn index_exists(&self, name: &str) -> Result<bool> {
        Ok(self.indices.read().await.contains_key(name))
    }

    async fn index_batch(&self, index: &str, documents: Vec<Document>) -> Result<()> {
        let handle = self.handle(index).await?;
        let mut conn = handle.conn.lock().expect("connection mutex poisoned");
        let tx = conn.as_raw_mut().transaction().map_err(backend_err)?;

        let content_columns: String = handle
            .field_order
            .iter()
            .map(|f| quote_ident(f))
            .collect::<Vec<_>>()
            .join(", ");
        let content_placeholders: String = std::iter::repeat("?")
            .take(handle.field_order.len())
            .collect::<Vec<_>>()
            .join(", ");
        let insert_content_sql = format!(
            "INSERT INTO content (_id{}{}) VALUES (?{}{})",
            if handle.field_order.is_empty() {
                ""
            } else {
                ", "
            },
            content_columns,
            if handle.field_order.is_empty() {
                ""
            } else {
                ", "
            },
            content_placeholders,
        );

        let fts_columns: String = handle
            .fts_field_order
            .iter()
            .map(|f| quote_ident(f))
            .collect::<Vec<_>>()
            .join(", ");
        let fts_placeholders: String = std::iter::repeat("?")
            .take(handle.fts_field_order.len())
            .collect::<Vec<_>>()
            .join(", ");
        let insert_fts_sql = format!(
            "INSERT INTO idx_fts (rowid{}{}) VALUES (?{}{})",
            if handle.fts_field_order.is_empty() {
                ""
            } else {
                ", "
            },
            fts_columns,
            if handle.fts_field_order.is_empty() {
                ""
            } else {
                ", "
            },
            fts_placeholders,
        );

        for document in documents {
            let prepared = convert::document_to_row(&handle.fields, &handle.field_order, document);

            // Insert-or-replace: clear out any previous row for this id
            // (and its FTS5 shadow row) before inserting fresh ones.
            if handle.has_fts {
                tx.execute(
                    "DELETE FROM idx_fts WHERE rowid = (SELECT rowid FROM content WHERE _id = ?1)",
                    params![prepared.id],
                )
                .map_err(backend_err)?;
            }
            tx.execute("DELETE FROM content WHERE _id = ?1", params![prepared.id])
                .map_err(backend_err)?;

            let content_params = std::iter::once(rusty_sqlite::rusqlite::types::Value::Text(
                prepared.id.clone(),
            ))
            .chain(prepared.content_values.iter().cloned());
            tx.execute(
                &insert_content_sql,
                rusty_sqlite::rusqlite::params_from_iter(content_params),
            )
            .map_err(backend_err)?;

            if handle.has_fts {
                let rowid = tx.last_insert_rowid();
                let fts_params =
                    std::iter::once(rusty_sqlite::rusqlite::types::Value::Integer(rowid)).chain(
                        prepared
                            .fts_values
                            .iter()
                            .cloned()
                            .map(rusty_sqlite::rusqlite::types::Value::Text),
                    );
                tx.execute(
                    &insert_fts_sql,
                    rusty_sqlite::rusqlite::params_from_iter(fts_params),
                )
                .map_err(backend_err)?;
            }
        }

        tx.commit().map_err(backend_err)?;
        Ok(())
    }

    async fn delete(&self, index: &str, id: &str) -> Result<()> {
        let handle = self.handle(index).await?;
        let conn = handle.conn.lock().expect("connection mutex poisoned");
        if handle.has_fts {
            conn.as_raw()
                .execute(
                    "DELETE FROM idx_fts WHERE rowid = (SELECT rowid FROM content WHERE _id = ?1)",
                    params![id],
                )
                .map_err(backend_err)?;
        }
        conn.as_raw()
            .execute("DELETE FROM content WHERE _id = ?1", params![id])
            .map_err(backend_err)?;
        Ok(())
    }

    async fn search(&self, index: &str, request: SearchRequest) -> Result<SearchResults> {
        if request.vector.is_some() {
            return Err(SearchError::InvalidQuery(
                "vector/hybrid search is not supported by rusty-search-sqlite-fts5 yet".into(),
            ));
        }
        let handle = self.handle(index).await?;
        let compiled = query_map::compile(&request, &handle.fields)?;
        let conn = handle.conn.lock().expect("connection mutex poisoned");

        let total: usize = conn
            .as_raw()
            .query_row(
                &compiled.count_sql,
                rusty_sqlite::rusqlite::params_from_iter(compiled.count_params.iter().cloned()),
                |row| row.get::<_, i64>(0),
            )
            .map_err(backend_err)? as usize;

        let mut stmt = conn
            .as_raw()
            .prepare(&compiled.select_sql)
            .map_err(backend_err)?;
        let rows = stmt
            .query_map(
                rusty_sqlite::rusqlite::params_from_iter(compiled.select_params.iter().cloned()),
                |row| {
                    let score: f64 = row.get("score")?;
                    let document = convert::row_to_document(row, &handle.fields);
                    Ok((score as f32, document))
                },
            )
            .map_err(backend_err)?;

        let mut hits = Vec::new();
        for row in rows {
            let (score, document) = row.map_err(backend_err)?;
            let id = document.id.clone().unwrap_or_default();
            hits.push(Hit {
                id,
                score,
                document,
            });
        }

        Ok(SearchResults { hits, total })
    }

    async fn commit(&self, index: &str) -> Result<()> {
        // Every write already ran (and committed) its own transaction by
        // the time index_batch/delete returned, so there's nothing left to
        // flush - just confirm the index exists, matching every other
        // backend's error behavior for a missing index.
        self.handle(index).await?;
        Ok(())
    }
}

impl SqliteFts5Backend {
    async fn handle(&self, index: &str) -> Result<Arc<IndexHandle>> {
        self.indices
            .read()
            .await
            .get(index)
            .cloned()
            .ok_or_else(|| SearchError::IndexNotFound(index.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_search_core::{FieldOptions, Query, Sort, SortOrder, VectorQuery};

    fn articles_schema() -> CoreSchema {
        CoreSchema::builder()
            .text("title")
            .keyword("status")
            .i64_field_with("views", FieldOptions::new().fast(true))
            .build()
    }

    async fn seeded_backend() -> SqliteFts5Backend {
        let backend = SqliteFts5Backend::in_memory();
        backend
            .create_index("articles", articles_schema())
            .await
            .unwrap();
        backend
            .index_batch(
                "articles",
                vec![
                    Document::new()
                        .with_id("1")
                        .set("title", "Rust async search")
                        .set("status", "published")
                        .set("views", 100),
                    Document::new()
                        .with_id("2")
                        .set("title", "Async Rust patterns")
                        .set("status", "draft")
                        .set("views", 10),
                    Document::new()
                        .with_id("3")
                        .set("title", "Cooking with cast iron")
                        .set("status", "published")
                        .set("views", 50),
                ],
            )
            .await
            .unwrap();
        backend.commit("articles").await.unwrap();
        backend
    }

    #[tokio::test]
    async fn create_index_rejects_duplicates() {
        let backend = SqliteFts5Backend::in_memory();
        backend.create_index("a", articles_schema()).await.unwrap();
        let err = backend
            .create_index("a", articles_schema())
            .await
            .unwrap_err();
        assert!(matches!(err, SearchError::IndexAlreadyExists(name) if name == "a"));
    }

    #[tokio::test]
    async fn operations_on_missing_index_error() {
        let backend = SqliteFts5Backend::in_memory();
        let err = backend
            .search("missing", Query::match_all().into())
            .await
            .unwrap_err();
        assert!(matches!(err, SearchError::IndexNotFound(name) if name == "missing"));
    }

    #[tokio::test]
    async fn full_text_match_finds_relevant_documents() {
        let backend = seeded_backend().await;
        let results = backend
            .search("articles", Query::match_query("title", "async rust").into())
            .await
            .unwrap();
        assert_eq!(results.total, 2);
        let ids: std::collections::HashSet<_> = results.hits.iter().map(|h| h.id.clone()).collect();
        assert!(ids.contains("1"));
        assert!(ids.contains("2"));
        // bm25-derived scores should actually rank matches, not just tie at 0.
        assert!(results.hits.iter().any(|h| h.score != 0.0));
    }

    #[tokio::test]
    async fn term_and_bool_queries_filter_exactly() {
        let backend = seeded_backend().await;
        let results = backend
            .search(
                "articles",
                Query::match_query("title", "async")
                    .and(Query::term("status", "published"))
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(results.total, 1);
        assert_eq!(results.hits[0].id, "1");
    }

    #[tokio::test]
    async fn range_query_filters_numerically() {
        let backend = seeded_backend().await;
        let results = backend
            .search(
                "articles",
                Query::range("views", Some(60.into()), None).into(),
            )
            .await
            .unwrap();
        assert_eq!(results.total, 1);
        assert_eq!(results.hits[0].id, "1");
    }

    #[tokio::test]
    async fn native_sort_orders_by_any_field_type() {
        let backend = seeded_backend().await;
        // Unlike rusty-search-tantivy, sorting doesn't need `fast: true` or
        // fall back to an in-memory cap - every column is real SQL.
        let results = backend
            .search(
                "articles",
                SearchRequest::new(Query::match_all())
                    .sort(Sort::field("status", SortOrder::Asc))
                    .limit(10),
            )
            .await
            .unwrap();
        let statuses: Vec<_> = results
            .hits
            .iter()
            .map(|h| {
                h.document
                    .get("status")
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        let mut sorted = statuses.clone();
        sorted.sort();
        assert_eq!(statuses, sorted);
    }

    #[tokio::test]
    async fn sort_by_views_ascending() {
        let backend = seeded_backend().await;
        let results = backend
            .search(
                "articles",
                SearchRequest::new(Query::term("status", "published"))
                    .sort(Sort::field("views", SortOrder::Asc)),
            )
            .await
            .unwrap();
        let ids: Vec<_> = results.hits.iter().map(|h| h.id.clone()).collect();
        assert_eq!(ids, vec!["3", "1"]); // views: 50 then 100
    }

    #[tokio::test]
    async fn must_not_wrapping_bare_match_works() {
        // rusty-search-meilisearch/rusty-search-algolia reject this shape
        // outright (ADR-0003); plain SQL `NOT (...)` handles it natively.
        let backend = seeded_backend().await;
        let results = backend
            .search("articles", Query::match_query("title", "rust").not().into())
            .await
            .unwrap();
        assert_eq!(results.total, 1);
        assert_eq!(results.hits[0].id, "3");
    }

    #[tokio::test]
    async fn second_match_clause_is_rejected() {
        let backend = seeded_backend().await;
        let err = backend
            .search(
                "articles",
                Query::match_query("title", "rust")
                    .and(Query::match_query("title", "async"))
                    .into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, SearchError::InvalidQuery(_)));
    }

    #[tokio::test]
    async fn delete_removes_document_from_results() {
        let backend = seeded_backend().await;
        backend.delete("articles", "1").await.unwrap();
        backend.commit("articles").await.unwrap();
        let results = backend
            .search("articles", Query::match_all().into())
            .await
            .unwrap();
        assert_eq!(results.total, 2);
        assert!(results.hits.iter().all(|h| h.id != "1"));
    }

    #[tokio::test]
    async fn reindexing_same_id_replaces_document() {
        let backend = seeded_backend().await;
        backend
            .index(
                "articles",
                Document::new()
                    .with_id("1")
                    .set("title", "Completely different")
                    .set("status", "archived")
                    .set("views", 5),
            )
            .await
            .unwrap();
        backend.commit("articles").await.unwrap();
        let results = backend
            .search("articles", Query::term("status", "archived").into())
            .await
            .unwrap();
        assert_eq!(results.total, 1);
        assert_eq!(results.hits[0].id, "1");

        // The old title shouldn't still be full-text searchable either -
        // the FTS5 shadow row must have been replaced too.
        let stale = backend
            .search("articles", Query::match_query("title", "rust").into())
            .await
            .unwrap();
        assert!(stale.hits.iter().all(|h| h.id != "1"));
    }

    #[tokio::test]
    async fn index_without_id_gets_one_assigned() {
        let backend = SqliteFts5Backend::in_memory();
        backend.create_index("a", articles_schema()).await.unwrap();
        backend
            .index("a", Document::new().set("title", "no id"))
            .await
            .unwrap();
        let results = backend
            .search("a", Query::match_all().into())
            .await
            .unwrap();
        assert_eq!(results.total, 1);
        assert!(!results.hits[0].id.is_empty());
    }

    #[tokio::test]
    async fn vector_search_is_rejected() {
        let backend = seeded_backend().await;
        assert!(!backend.supports_vector_search());
        let request = SearchRequest::new(Query::match_all()).vector(VectorQuery::new(
            "embedding",
            vec![0.1, 0.2],
            5,
        ));
        let err = backend.search("articles", request).await.unwrap_err();
        assert!(matches!(err, SearchError::InvalidQuery(_)));
    }

    #[tokio::test]
    async fn on_disk_backend_persists_within_process() {
        let dir = tempfile::tempdir().unwrap();
        let backend = SqliteFts5Backend::on_disk(dir.path());
        backend
            .create_index("articles", articles_schema())
            .await
            .unwrap();
        backend
            .index(
                "articles",
                Document::new().with_id("1").set("title", "hello disk"),
            )
            .await
            .unwrap();
        backend.commit("articles").await.unwrap();

        let results = backend
            .search("articles", Query::match_query("title", "hello").into())
            .await
            .unwrap();
        assert_eq!(results.total, 1);
    }

    #[tokio::test]
    async fn on_disk_backend_rejects_reopening_existing_index() {
        let dir = tempfile::tempdir().unwrap();
        let backend = SqliteFts5Backend::on_disk(dir.path());
        backend
            .create_index("articles", articles_schema())
            .await
            .unwrap();
        drop(backend);

        let reopened = SqliteFts5Backend::on_disk(dir.path());
        let err = reopened
            .create_index("articles", articles_schema())
            .await
            .unwrap_err();
        assert!(matches!(err, SearchError::IndexAlreadyExists(_)));
    }
}
