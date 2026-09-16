//! `CrawlJob` and `CrawlURL` — a Rust port of `server/model/crawl.go`'s
//! persistent-crawl-job tables (backs the CLI's crawl-job subtree, §3.1).
//! Neither embeds Hister's `CommonFields` (no soft delete) in the Go
//! source, so neither does its Rust equivalent here.
//!
//! `CrawlJob`'s own lifecycle (`generate_id`/`create`/`create_with_urls`/
//! `get`/`update_status`/`list`/`delete`) is ported below. `generate_id`
//! uses the existing first-party `rusty_rand` crate for its OS-backed
//! CSPRNG bytes rather than adding the external `rand` crate (sovereignty
//! check: `rusty_rand` already exists in this workspace precisely to
//! avoid re-adding that dependency). `create_with_urls`'s job-id-collision
//! retry loop and its per-URL dedup both need `ON CONFLICT DO NOTHING`,
//! which `rusty_db`'s query builder doesn't support, so both drop to raw
//! SQL inside a `Transaction` — the same pattern `embedding.rs` uses for
//! its `CASE`-based updates, extended here to a multi-statement
//! transaction via `Engine::begin()`/`Transaction::execute`/`commit`.
//!
//! `CrawlURL`'s own queue mechanics are ported too:
//! `insert_if_not_exists`/`bulk_insert`/`mark_done_and_enqueue_links`/
//! `insert_done`/`next_pending`/`update_status`/`mark_failed`/
//! `reset_in_progress`/`count_by_status`/`count`/`list_failed`/`list`/
//! `job_stats`. Go's `insertCrawlURLs` private helper (shared by
//! `CreateNamedCrawlJobWithURLs` and `BulkInsertCrawlURLs`) becomes this
//! file's own private `insert_crawl_urls`, used by `CrawlJob::create_with_urls`
//! and `CrawlURL::bulk_insert` alike. `ForEachFailedCrawlURL(WithMessage)`/
//! `ForEachCrawlURL(ByStatus)` become `list_failed`/`list` returning a
//! `Vec<Self>` rather than taking a row-streaming callback — a deliberate
//! simplification (this crate has no other streaming-query precedent, and
//! nothing yet calls these at a scale where collecting matters), not a
//! silently dropped capability: every row Go's callback saw is still
//! reachable, just batched instead of streamed.

use crate::placeholders;
use rusty_db::{prelude::*, Dialect};

/// `CrawlJob.status` values (Go: untyped string constants
/// `CrawlJobRunning`/`CrawlJobCompleted`/`CrawlJobInterrupted`). A closed
/// enum here makes the otherwise-implicit "only these three values are
/// valid" invariant checkable by the type system instead of by convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, MappedEnum)]
pub enum CrawlJobStatus {
    Running,
    Completed,
    Interrupted,
}

/// `CrawlURL.status` values (Go: untyped string constants
/// `CrawlURLPending`/`CrawlURLInProgress`/`CrawlURLDone`/`CrawlURLFailed`/
/// `CrawlURLSkipped`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, MappedEnum)]
pub enum CrawlUrlStatus {
    #[default]
    Pending,
    InProgress,
    Done,
    Failed,
    Skipped,
}

/// The configuration and status of a persistent crawl job. `id` is a short
/// random hex string (Go: `GenerateCrawlJobID`), not an auto-incrementing
/// integer.
#[derive(Debug, Clone, PartialEq, Mapped)]
#[table(name = "crawl_jobs")]
pub struct CrawlJob {
    #[table(primary_key)]
    pub id: String,
    pub start_url: String,
    /// JSON-encoded `ValidatorRules` (Go stores this as `type:text`
    /// containing JSON; using `Json` here gets the same on-disk shape with
    /// a structured Rust type instead of a raw string).
    pub validator_rules: Json,
    pub label: String,
    pub status: CrawlJobStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl CrawlJob {
    /// Generates a random 8-character hex string suitable as a job id.
    pub fn generate_id() -> Result<String, rusty_rand::Error> {
        let bytes = rusty_rand::bytes(4)?;
        Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }

    /// Inserts a new crawl job record.
    pub async fn create(
        engine: &Engine,
        id: &str,
        start_url: &str,
        validator_rules: Json,
        label: &str,
    ) -> rusty_db::Result<()> {
        let now = Utc::now();
        let job = Self {
            id: id.to_string(),
            start_url: start_url.to_string(),
            validator_rules,
            label: label.to_string(),
            status: CrawlJobStatus::Running,
            created_at: now,
            updated_at: now,
        };
        engine.execute(&job.insert()).await?;
        Ok(())
    }

    /// Atomically creates a crawl job and its initial URL queue. `base_id`
    /// is used when available, followed by `-2`, `-3`, and so on when
    /// another job already uses that id.
    pub async fn create_with_urls(
        engine: &Engine,
        base_id: &str,
        start_url: &str,
        validator_rules: Json,
        label: &str,
        urls: &[String],
    ) -> rusty_db::Result<String> {
        if urls.is_empty() {
            return Err(rusty_db::Error::QueryBuilder(
                "crawl job requires at least one URL".to_string(),
            ));
        }

        let now = Utc::now();
        let dialect = engine.dialect();
        let mut tx = engine.begin().await?;

        let job_id = 'retry: {
            let mut suffix = 1;
            loop {
                let id = if suffix == 1 {
                    base_id.to_string()
                } else {
                    format!("{base_id}-{suffix}")
                };

                let p = placeholders(dialect, 6);
                let sql = format!(
                    "INSERT INTO crawl_jobs \
                        (id, start_url, validator_rules, label, status, created_at, updated_at) \
                     VALUES ({}, {}, {}, {}, 'running', {}, {}) \
                     ON CONFLICT (id) DO NOTHING",
                    p[0], p[1], p[2], p[3], p[4], p[5]
                );
                let params: Vec<Value> = vec![
                    id.clone().into(),
                    start_url.to_string().into(),
                    validator_rules.clone().into(),
                    label.to_string().into(),
                    now.into(),
                    now.into(),
                ];
                let inserted = tx.execute(&sql, &params).await?;
                if inserted == 1 {
                    break 'retry id;
                }
                suffix += 1;
            }
        };

        insert_crawl_urls(&mut tx, dialect, &job_id, urls, 0, now).await?;

        tx.commit().await?;
        Ok(job_id)
    }

    /// Looks up a job by id.
    pub async fn get(engine: &Engine, id: &str) -> rusty_db::Result<Option<Self>> {
        let table = Self::table();
        engine
            .fetch_optional_as(&Select::from(&table).filter(table.col("id").eq(id)))
            .await
    }

    /// Updates a job's status.
    pub async fn update_status(
        engine: &Engine,
        id: &str,
        status: CrawlJobStatus,
    ) -> rusty_db::Result<()> {
        let table = Self::table();
        engine
            .execute(
                &Update::table(&table)
                    .set("status", status)
                    .set("updated_at", Utc::now())
                    .filter(table.col("id").eq(id)),
            )
            .await?;
        Ok(())
    }

    /// All crawl jobs, newest first.
    pub async fn list(engine: &Engine) -> rusty_db::Result<Vec<Self>> {
        let table = Self::table();
        engine
            .fetch_all_as(&Select::from(&table).order_by(table.col("created_at").desc()))
            .await
    }

    /// Removes a job and all its associated URL rows.
    pub async fn delete(engine: &Engine, id: &str) -> rusty_db::Result<()> {
        let urls_table = CrawlURL::table();
        engine
            .execute(&Delete::from(&urls_table).filter(urls_table.col("job_id").eq(id)))
            .await?;
        let table = Self::table();
        engine
            .execute(&Delete::from(&table).filter(table.col("id").eq(id)))
            .await?;
        Ok(())
    }
}

/// A single URL discovered during a crawl job.
#[derive(Debug, Clone, PartialEq, Mapped)]
#[table(name = "crawl_urls")]
pub struct CrawlURL {
    #[table(primary_key)]
    pub id: i64,
    pub job_id: String,
    pub url: String,
    pub depth: i64,
    #[table(default = "'pending'")]
    pub status: CrawlUrlStatus,
    pub error: String,
    pub error_code: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Inserts all `urls` for `job_id` at `depth`, silently skipping any
/// already present (the unique index on `(job_id, url)` is what "already
/// present" means). Shared by `CrawlJob::create_with_urls` and
/// `CrawlURL::bulk_insert` — Go's `insertCrawlURLs` private helper, shared
/// the same way by `CreateNamedCrawlJobWithURLs` and `BulkInsertCrawlURLs`.
async fn insert_crawl_urls(
    tx: &mut Transaction,
    dialect: &dyn Dialect,
    job_id: &str,
    urls: &[String],
    depth: i64,
    now: DateTime<Utc>,
) -> rusty_db::Result<()> {
    for url in urls {
        let p = placeholders(dialect, 5);
        let sql = format!(
            "INSERT INTO crawl_urls \
                (job_id, url, depth, status, error, error_code, created_at, updated_at) \
             VALUES ({}, {}, {}, 'pending', '', 0, {}, {}) \
             ON CONFLICT (job_id, url) DO NOTHING",
            p[0], p[1], p[2], p[3], p[4]
        );
        let params: Vec<Value> = vec![
            job_id.to_string().into(),
            url.clone().into(),
            depth.into(),
            now.into(),
            now.into(),
        ];
        tx.execute(&sql, &params).await?;
    }
    Ok(())
}

/// Aggregate URL counts per status for a crawl job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CrawlJobStats {
    pub pending: i64,
    pub in_progress: i64,
    pub done: i64,
    pub failed: i64,
    pub skipped: i64,
}

impl CrawlURL {
    /// Adds a URL to the job's queue only when it has not been seen
    /// before.
    pub async fn insert_if_not_exists(
        engine: &Engine,
        job_id: &str,
        url: &str,
        depth: i64,
    ) -> rusty_db::Result<()> {
        Self::bulk_insert(
            engine,
            job_id,
            std::slice::from_ref(&url.to_string()),
            depth,
        )
        .await
    }

    /// Inserts all `urls` for the given job in a single transaction,
    /// silently skipping any that are already present.
    pub async fn bulk_insert(
        engine: &Engine,
        job_id: &str,
        urls: &[String],
        depth: i64,
    ) -> rusty_db::Result<()> {
        if urls.is_empty() {
            return Ok(());
        }
        let now = Utc::now();
        let dialect = engine.dialect();
        let mut tx = engine.begin().await?;
        insert_crawl_urls(&mut tx, dialect, job_id, urls, depth, now).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Marks a crawl URL as done and inserts all discovered child URLs in
    /// a single transaction.
    pub async fn mark_done_and_enqueue_links(
        engine: &Engine,
        id: i64,
        job_id: &str,
        links: &[String],
        depth: i64,
    ) -> rusty_db::Result<()> {
        let now = Utc::now();
        let dialect = engine.dialect();
        let mut tx = engine.begin().await?;

        let table = Self::table();
        tx.execute_query(
            &Update::table(&table)
                .set("status", CrawlUrlStatus::Done)
                .set("error", "")
                .set("error_code", 0_i64)
                .set("updated_at", now)
                .filter(table.col("id").eq(id)),
            dialect,
        )
        .await?;

        if !links.is_empty() {
            insert_crawl_urls(&mut tx, dialect, job_id, links, depth, now).await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Marks a URL done if already present (e.g. a redirect target
    /// fetched indirectly), otherwise inserts it directly in the done
    /// state.
    pub async fn insert_done(
        engine: &Engine,
        job_id: &str,
        url: &str,
        depth: i64,
    ) -> rusty_db::Result<()> {
        let now = Utc::now();
        let table = Self::table();
        let updated = engine
            .execute(
                &Update::table(&table)
                    .set("status", CrawlUrlStatus::Done)
                    .set("error", "")
                    .set("error_code", 0_i64)
                    .set("updated_at", now)
                    .filter(table.col("job_id").eq(job_id))
                    .filter(table.col("url").eq(url)),
            )
            .await?;
        if updated == 0 {
            let dialect = engine.dialect();
            let mut conn = engine.connect().await?;
            let p = placeholders(dialect, 5);
            let sql = format!(
                "INSERT INTO crawl_urls \
                    (job_id, url, depth, status, error, error_code, created_at, updated_at) \
                 VALUES ({}, {}, {}, 'done', '', 0, {}, {})",
                p[0], p[1], p[2], p[3], p[4]
            );
            let params: Vec<Value> = vec![
                job_id.to_string().into(),
                url.to_string().into(),
                depth.into(),
                now.into(),
                now.into(),
            ];
            conn.execute(&sql, &params).await?;
        }
        Ok(())
    }

    /// The oldest pending URL for the job, or `None` when none remain.
    pub async fn next_pending(engine: &Engine, job_id: &str) -> rusty_db::Result<Option<Self>> {
        let table = Self::table();
        engine
            .fetch_optional_as(
                &Select::from(&table)
                    .filter(table.col("job_id").eq(job_id))
                    .filter(table.col("status").eq("pending"))
                    .order_by(table.col("id").asc())
                    .limit(1),
            )
            .await
    }

    /// Sets the status and optional error message on a URL row.
    pub async fn update_status(
        engine: &Engine,
        id: i64,
        status: CrawlUrlStatus,
        err_msg: &str,
    ) -> rusty_db::Result<()> {
        let table = Self::table();
        engine
            .execute(
                &Update::table(&table)
                    .set("status", status)
                    .set("error", err_msg)
                    .set("error_code", 0_i64)
                    .set("updated_at", Utc::now())
                    .filter(table.col("id").eq(id)),
            )
            .await?;
        Ok(())
    }

    /// Marks the URL for a job as failed with an error code and message.
    pub async fn mark_failed(
        engine: &Engine,
        job_id: &str,
        url: &str,
        err_code: i64,
        err_msg: &str,
    ) -> rusty_db::Result<()> {
        let table = Self::table();
        engine
            .execute(
                &Update::table(&table)
                    .set("status", CrawlUrlStatus::Failed)
                    .set("error", err_msg)
                    .set("error_code", err_code)
                    .set("updated_at", Utc::now())
                    .filter(table.col("job_id").eq(job_id))
                    .filter(table.col("url").eq(url)),
            )
            .await?;
        Ok(())
    }

    /// Moves all `in_progress` URLs for a job back to `pending` so they
    /// are retried after a crash or interruption.
    pub async fn reset_in_progress(engine: &Engine, job_id: &str) -> rusty_db::Result<()> {
        let table = Self::table();
        engine
            .execute(
                &Update::table(&table)
                    .set("status", CrawlUrlStatus::Pending)
                    .set("updated_at", Utc::now())
                    .filter(table.col("job_id").eq(job_id))
                    .filter(table.col("status").eq("in_progress")),
            )
            .await?;
        Ok(())
    }

    /// Number of URLs with the given status for a job.
    pub async fn count_by_status(
        engine: &Engine,
        job_id: &str,
        status: CrawlUrlStatus,
    ) -> rusty_db::Result<i64> {
        let table = Self::table();
        let row = engine
            .fetch_one(
                &Select::from(&table)
                    .columns([SelectExpr::from(Expr::count_all()).alias("count")])
                    .filter(table.col("job_id").eq(job_id))
                    .filter(table.col("status").eq(status)),
            )
            .await?;
        row.get_by_name("count")
    }

    /// Number of URL rows tracked for a job.
    pub async fn count(engine: &Engine, job_id: &str) -> rusty_db::Result<i64> {
        let table = Self::table();
        let row = engine
            .fetch_one(
                &Select::from(&table)
                    .columns([SelectExpr::from(Expr::count_all()).alias("count")])
                    .filter(table.col("job_id").eq(job_id)),
            )
            .await?;
        row.get_by_name("count")
    }

    /// Every failed URL for a job, oldest first.
    pub async fn list_failed(engine: &Engine, job_id: &str) -> rusty_db::Result<Vec<Self>> {
        let table = Self::table();
        engine
            .fetch_all_as(
                &Select::from(&table)
                    .filter(table.col("job_id").eq(job_id))
                    .filter(table.col("status").eq("failed"))
                    .order_by(table.col("id").asc()),
            )
            .await
    }

    /// Every URL tracked for a job, oldest first, optionally filtered to
    /// one status.
    pub async fn list(
        engine: &Engine,
        job_id: &str,
        status: Option<CrawlUrlStatus>,
    ) -> rusty_db::Result<Vec<Self>> {
        let table = Self::table();
        let mut query = Select::from(&table).filter(table.col("job_id").eq(job_id));
        if let Some(status) = status {
            query = query.filter(table.col("status").eq(status));
        }
        engine
            .fetch_all_as(&query.order_by(table.col("id").asc()))
            .await
    }

    /// URL counts per status for a job.
    pub async fn job_stats(engine: &Engine, job_id: &str) -> rusty_db::Result<CrawlJobStats> {
        let table = Self::table();
        let rows = engine
            .fetch_all(
                &Select::from(&table)
                    .columns([
                        SelectExpr::from(table.col("status")),
                        SelectExpr::from(Expr::count_all()).alias("count"),
                    ])
                    .filter(table.col("job_id").eq(job_id))
                    .group_by([table.col("status")]),
            )
            .await?;

        let mut stats = CrawlJobStats::default();
        for row in rows {
            let status: CrawlUrlStatus = row.get_by_name("status")?;
            let count: i64 = row.get_by_name("count")?;
            match status {
                CrawlUrlStatus::Pending => stats.pending = count,
                CrawlUrlStatus::InProgress => stats.in_progress = count,
                CrawlUrlStatus::Done => stats.done = count,
                CrawlUrlStatus::Failed => stats.failed = count,
                CrawlUrlStatus::Skipped => stats.skipped = count,
            }
        }
        Ok(stats)
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
    fn table_names() {
        assert_eq!(CrawlJob::TABLE_NAME, "crawl_jobs");
        assert_eq!(CrawlURL::TABLE_NAME, "crawl_urls");
    }

    #[test]
    fn crawl_url_status_defaults_to_pending() {
        assert_eq!(CrawlUrlStatus::default(), CrawlUrlStatus::Pending);
    }

    #[tokio::test]
    async fn crawl_job_and_its_urls_round_trip() {
        let engine = migrated_engine().await;

        let job = CrawlJob {
            id: "abcd1234".into(),
            start_url: "https://example.com".into(),
            validator_rules: serde_json::json!({"allow": ["example.com"]}),
            label: "example crawl".into(),
            status: CrawlJobStatus::Running,
            created_at: now(),
            updated_at: now(),
        };
        engine.execute(&job.insert()).await.unwrap();

        let url = CrawlURL {
            id: 1,
            job_id: job.id.clone(),
            url: "https://example.com/page".into(),
            depth: 1,
            status: CrawlUrlStatus::Pending,
            error: String::new(),
            error_code: 0,
            created_at: now(),
            updated_at: now(),
        };
        engine.execute(&url.insert()).await.unwrap();

        let fetched_job: CrawlJob = engine
            .fetch_one_as(&Select::from(&CrawlJob::table()))
            .await
            .unwrap();
        assert_eq!(fetched_job.status, CrawlJobStatus::Running);

        let fetched_url: CrawlURL = engine
            .fetch_one_as(&Select::from(&CrawlURL::table()))
            .await
            .unwrap();
        assert_eq!(fetched_url.job_id, job.id);
        assert_eq!(fetched_url.status, CrawlUrlStatus::Pending);
    }

    #[tokio::test]
    async fn duplicate_job_url_pair_is_rejected() {
        let engine = migrated_engine().await;
        let job = CrawlJob {
            id: "job1".into(),
            start_url: "https://example.com".into(),
            validator_rules: serde_json::json!({}),
            label: "".into(),
            status: CrawlJobStatus::Running,
            created_at: now(),
            updated_at: now(),
        };
        engine.execute(&job.insert()).await.unwrap();

        let first = CrawlURL {
            id: 1,
            job_id: job.id.clone(),
            url: "https://example.com/a".into(),
            depth: 0,
            status: CrawlUrlStatus::Pending,
            error: String::new(),
            error_code: 0,
            created_at: now(),
            updated_at: now(),
        };
        engine.execute(&first.insert()).await.unwrap();

        let duplicate = CrawlURL { id: 2, ..first };
        assert!(engine.execute(&duplicate.insert()).await.is_err());
    }

    #[test]
    fn generate_id_returns_distinct_eight_char_hex_strings() {
        let a = CrawlJob::generate_id().unwrap();
        let b = CrawlJob::generate_id().unwrap();
        assert_eq!(a.len(), 8);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn create_inserts_a_running_job() {
        let engine = migrated_engine().await;
        CrawlJob::create(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "label",
        )
        .await
        .unwrap();

        let job = CrawlJob::get(&engine, "job1").await.unwrap().unwrap();
        assert_eq!(job.status, CrawlJobStatus::Running);
        assert_eq!(job.start_url, "https://example.com");
    }

    #[tokio::test]
    async fn get_returns_none_for_an_unknown_id() {
        let engine = migrated_engine().await;
        assert_eq!(CrawlJob::get(&engine, "no-such-job").await.unwrap(), None);
    }

    #[tokio::test]
    async fn update_status_changes_the_stored_status() {
        let engine = migrated_engine().await;
        CrawlJob::create(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
        )
        .await
        .unwrap();

        CrawlJob::update_status(&engine, "job1", CrawlJobStatus::Completed)
            .await
            .unwrap();

        let job = CrawlJob::get(&engine, "job1").await.unwrap().unwrap();
        assert_eq!(job.status, CrawlJobStatus::Completed);
    }

    #[tokio::test]
    async fn list_returns_jobs_newest_first() {
        let engine = migrated_engine().await;
        CrawlJob::create(&engine, "older", "https://a.com", serde_json::json!({}), "")
            .await
            .unwrap();
        // SQLite's CURRENT_TIMESTAMP-free `now()` calls here could tie, so
        // insert the jobs directly with distinct timestamps instead.
        let table = CrawlJob::table();
        engine
            .execute(
                &Update::table(&table)
                    .set(
                        "created_at",
                        "2024-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap(),
                    )
                    .filter(table.col("id").eq("older")),
            )
            .await
            .unwrap();
        CrawlJob::create(&engine, "newer", "https://b.com", serde_json::json!({}), "")
            .await
            .unwrap();
        engine
            .execute(
                &Update::table(&table)
                    .set(
                        "created_at",
                        "2024-02-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap(),
                    )
                    .filter(table.col("id").eq("newer")),
            )
            .await
            .unwrap();

        let jobs = CrawlJob::list(&engine).await.unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].id, "newer");
        assert_eq!(jobs[1].id, "older");
    }

    #[tokio::test]
    async fn delete_removes_the_job_and_its_urls() {
        let engine = migrated_engine().await;
        CrawlJob::create_with_urls(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
            &["https://example.com/a".to_string()],
        )
        .await
        .unwrap();

        CrawlJob::delete(&engine, "job1").await.unwrap();

        assert_eq!(CrawlJob::get(&engine, "job1").await.unwrap(), None);
        let urls_table = CrawlURL::table();
        let remaining: Vec<CrawlURL> = engine
            .fetch_all_as(&Select::from(&urls_table))
            .await
            .unwrap();
        assert!(remaining.is_empty());
    }

    #[tokio::test]
    async fn create_with_urls_rejects_an_empty_url_list() {
        let engine = migrated_engine().await;
        let result = CrawlJob::create_with_urls(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
            &[],
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn create_with_urls_inserts_the_job_and_its_initial_queue() {
        let engine = migrated_engine().await;
        let urls = vec![
            "https://example.com/a".to_string(),
            "https://example.com/b".to_string(),
        ];
        let job_id = CrawlJob::create_with_urls(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "label",
            &urls,
        )
        .await
        .unwrap();
        assert_eq!(job_id, "job1");

        let job = CrawlJob::get(&engine, "job1").await.unwrap().unwrap();
        assert_eq!(job.status, CrawlJobStatus::Running);

        let urls_table = CrawlURL::table();
        let queued: Vec<CrawlURL> = engine
            .fetch_all_as(&Select::from(&urls_table))
            .await
            .unwrap();
        assert_eq!(queued.len(), 2);
        assert!(queued.iter().all(|u| u.job_id == "job1"));
        assert!(queued.iter().all(|u| u.status == CrawlUrlStatus::Pending));
    }

    #[tokio::test]
    async fn create_with_urls_retries_with_a_suffix_on_id_collision() {
        let engine = migrated_engine().await;
        CrawlJob::create(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
        )
        .await
        .unwrap();

        let job_id = CrawlJob::create_with_urls(
            &engine,
            "job1",
            "https://other.com",
            serde_json::json!({}),
            "",
            &["https://other.com/a".to_string()],
        )
        .await
        .unwrap();
        assert_eq!(job_id, "job1-2");

        let original = CrawlJob::get(&engine, "job1").await.unwrap().unwrap();
        assert_eq!(original.start_url, "https://example.com");
        let renamed = CrawlJob::get(&engine, "job1-2").await.unwrap().unwrap();
        assert_eq!(renamed.start_url, "https://other.com");
    }

    #[tokio::test]
    async fn insert_if_not_exists_dedups_against_an_existing_url() {
        let engine = migrated_engine().await;
        CrawlJob::create(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
        )
        .await
        .unwrap();

        CrawlURL::insert_if_not_exists(&engine, "job1", "https://example.com/a", 0)
            .await
            .unwrap();
        CrawlURL::insert_if_not_exists(&engine, "job1", "https://example.com/a", 0)
            .await
            .unwrap();

        assert_eq!(CrawlURL::count(&engine, "job1").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn bulk_insert_skips_urls_already_present() {
        let engine = migrated_engine().await;
        CrawlJob::create(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
        )
        .await
        .unwrap();
        CrawlURL::insert_if_not_exists(&engine, "job1", "https://example.com/a", 0)
            .await
            .unwrap();

        CrawlURL::bulk_insert(
            &engine,
            "job1",
            &[
                "https://example.com/a".to_string(),
                "https://example.com/b".to_string(),
            ],
            0,
        )
        .await
        .unwrap();

        assert_eq!(CrawlURL::count(&engine, "job1").await.unwrap(), 2);
    }

    #[tokio::test]
    async fn bulk_insert_is_a_no_op_for_an_empty_list() {
        let engine = migrated_engine().await;
        CrawlJob::create(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
        )
        .await
        .unwrap();
        CrawlURL::bulk_insert(&engine, "job1", &[], 0)
            .await
            .unwrap();
        assert_eq!(CrawlURL::count(&engine, "job1").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn mark_done_and_enqueue_links_updates_the_url_and_queues_children() {
        let engine = migrated_engine().await;
        let job_id = CrawlJob::create_with_urls(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
            &["https://example.com".to_string()],
        )
        .await
        .unwrap();
        let parent = CrawlURL::next_pending(&engine, &job_id)
            .await
            .unwrap()
            .unwrap();

        CrawlURL::mark_done_and_enqueue_links(
            &engine,
            parent.id,
            &job_id,
            &["https://example.com/child".to_string()],
            1,
        )
        .await
        .unwrap();

        let all = CrawlURL::list(&engine, &job_id, None).await.unwrap();
        assert_eq!(all.len(), 2);
        let done = all.iter().find(|u| u.id == parent.id).unwrap();
        assert_eq!(done.status, CrawlUrlStatus::Done);
        let child = all.iter().find(|u| u.url.ends_with("/child")).unwrap();
        assert_eq!(child.status, CrawlUrlStatus::Pending);
        assert_eq!(child.depth, 1);
    }

    #[tokio::test]
    async fn insert_done_updates_an_existing_row() {
        let engine = migrated_engine().await;
        CrawlJob::create(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
        )
        .await
        .unwrap();
        CrawlURL::insert_if_not_exists(&engine, "job1", "https://example.com/a", 0)
            .await
            .unwrap();

        CrawlURL::insert_done(&engine, "job1", "https://example.com/a", 0)
            .await
            .unwrap();

        assert_eq!(CrawlURL::count(&engine, "job1").await.unwrap(), 1);
        let all = CrawlURL::list(&engine, "job1", None).await.unwrap();
        assert_eq!(all[0].status, CrawlUrlStatus::Done);
    }

    #[tokio::test]
    async fn insert_done_inserts_when_no_row_exists() {
        let engine = migrated_engine().await;
        CrawlJob::create(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
        )
        .await
        .unwrap();

        CrawlURL::insert_done(&engine, "job1", "https://example.com/a", 2)
            .await
            .unwrap();

        let all = CrawlURL::list(&engine, "job1", None).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].status, CrawlUrlStatus::Done);
        assert_eq!(all[0].depth, 2);
    }

    #[tokio::test]
    async fn next_pending_returns_the_oldest_pending_url() {
        let engine = migrated_engine().await;
        let job_id = CrawlJob::create_with_urls(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
            &[
                "https://example.com/a".to_string(),
                "https://example.com/b".to_string(),
            ],
        )
        .await
        .unwrap();

        let next = CrawlURL::next_pending(&engine, &job_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(next.url, "https://example.com/a");
    }

    #[tokio::test]
    async fn next_pending_returns_none_when_queue_is_empty() {
        let engine = migrated_engine().await;
        CrawlJob::create(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
        )
        .await
        .unwrap();
        assert_eq!(CrawlURL::next_pending(&engine, "job1").await.unwrap(), None);
    }

    #[tokio::test]
    async fn update_status_sets_status_and_error() {
        let engine = migrated_engine().await;
        let job_id = CrawlJob::create_with_urls(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
            &["https://example.com/a".to_string()],
        )
        .await
        .unwrap();
        let url = CrawlURL::next_pending(&engine, &job_id)
            .await
            .unwrap()
            .unwrap();

        CrawlURL::update_status(&engine, url.id, CrawlUrlStatus::Failed, "boom")
            .await
            .unwrap();

        let all = CrawlURL::list(&engine, &job_id, None).await.unwrap();
        assert_eq!(all[0].status, CrawlUrlStatus::Failed);
        assert_eq!(all[0].error, "boom");
    }

    #[tokio::test]
    async fn mark_failed_sets_status_error_code_and_message() {
        let engine = migrated_engine().await;
        CrawlJob::create(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
        )
        .await
        .unwrap();
        CrawlURL::insert_if_not_exists(&engine, "job1", "https://example.com/a", 0)
            .await
            .unwrap();

        CrawlURL::mark_failed(&engine, "job1", "https://example.com/a", 404, "not found")
            .await
            .unwrap();

        let all = CrawlURL::list(&engine, "job1", None).await.unwrap();
        assert_eq!(all[0].status, CrawlUrlStatus::Failed);
        assert_eq!(all[0].error_code, 404);
        assert_eq!(all[0].error, "not found");
    }

    #[tokio::test]
    async fn reset_in_progress_moves_only_in_progress_urls_back_to_pending() {
        let engine = migrated_engine().await;
        let job_id = CrawlJob::create_with_urls(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
            &[
                "https://example.com/a".to_string(),
                "https://example.com/b".to_string(),
            ],
        )
        .await
        .unwrap();
        let all = CrawlURL::list(&engine, &job_id, None).await.unwrap();
        CrawlURL::update_status(&engine, all[0].id, CrawlUrlStatus::InProgress, "")
            .await
            .unwrap();
        CrawlURL::update_status(&engine, all[1].id, CrawlUrlStatus::Done, "")
            .await
            .unwrap();

        CrawlURL::reset_in_progress(&engine, &job_id).await.unwrap();

        let after = CrawlURL::list(&engine, &job_id, None).await.unwrap();
        let reset = after.iter().find(|u| u.id == all[0].id).unwrap();
        assert_eq!(reset.status, CrawlUrlStatus::Pending);
        let untouched = after.iter().find(|u| u.id == all[1].id).unwrap();
        assert_eq!(untouched.status, CrawlUrlStatus::Done);
    }

    #[tokio::test]
    async fn count_by_status_and_count_reflect_the_queue() {
        let engine = migrated_engine().await;
        let job_id = CrawlJob::create_with_urls(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
            &[
                "https://example.com/a".to_string(),
                "https://example.com/b".to_string(),
            ],
        )
        .await
        .unwrap();
        let all = CrawlURL::list(&engine, &job_id, None).await.unwrap();
        CrawlURL::update_status(&engine, all[0].id, CrawlUrlStatus::Done, "")
            .await
            .unwrap();

        assert_eq!(CrawlURL::count(&engine, &job_id).await.unwrap(), 2);
        assert_eq!(
            CrawlURL::count_by_status(&engine, &job_id, CrawlUrlStatus::Done)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            CrawlURL::count_by_status(&engine, &job_id, CrawlUrlStatus::Pending)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn list_failed_returns_only_failed_urls() {
        let engine = migrated_engine().await;
        let job_id = CrawlJob::create_with_urls(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
            &[
                "https://example.com/a".to_string(),
                "https://example.com/b".to_string(),
            ],
        )
        .await
        .unwrap();
        let all = CrawlURL::list(&engine, &job_id, None).await.unwrap();
        CrawlURL::mark_failed(&engine, &job_id, &all[0].url, 500, "boom")
            .await
            .unwrap();

        let failed = CrawlURL::list_failed(&engine, &job_id).await.unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].url, all[0].url);
    }

    #[tokio::test]
    async fn list_filters_by_status_when_given() {
        let engine = migrated_engine().await;
        let job_id = CrawlJob::create_with_urls(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
            &[
                "https://example.com/a".to_string(),
                "https://example.com/b".to_string(),
            ],
        )
        .await
        .unwrap();
        let all = CrawlURL::list(&engine, &job_id, None).await.unwrap();
        CrawlURL::update_status(&engine, all[0].id, CrawlUrlStatus::Done, "")
            .await
            .unwrap();

        let pending_only = CrawlURL::list(&engine, &job_id, Some(CrawlUrlStatus::Pending))
            .await
            .unwrap();
        assert_eq!(pending_only.len(), 1);
        assert_eq!(pending_only[0].id, all[1].id);
    }

    #[tokio::test]
    async fn job_stats_aggregates_counts_per_status() {
        let engine = migrated_engine().await;
        let job_id = CrawlJob::create_with_urls(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
            &[
                "https://example.com/a".to_string(),
                "https://example.com/b".to_string(),
                "https://example.com/c".to_string(),
            ],
        )
        .await
        .unwrap();
        let all = CrawlURL::list(&engine, &job_id, None).await.unwrap();
        CrawlURL::update_status(&engine, all[0].id, CrawlUrlStatus::Done, "")
            .await
            .unwrap();
        CrawlURL::mark_failed(&engine, &job_id, &all[1].url, 500, "boom")
            .await
            .unwrap();

        let stats = CrawlURL::job_stats(&engine, &job_id).await.unwrap();
        assert_eq!(stats.pending, 1);
        assert_eq!(stats.done, 1);
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.in_progress, 0);
        assert_eq!(stats.skipped, 0);
    }

    #[tokio::test]
    async fn job_stats_is_all_zero_for_a_job_with_no_urls() {
        let engine = migrated_engine().await;
        CrawlJob::create(
            &engine,
            "job1",
            "https://example.com",
            serde_json::json!({}),
            "",
        )
        .await
        .unwrap();

        let stats = CrawlURL::job_stats(&engine, "job1").await.unwrap();
        assert_eq!(stats, CrawlJobStats::default());
    }
}
