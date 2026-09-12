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
//! `CrawlURL`'s own queue mechanics (`BulkInsertCrawlURLs`,
//! `MarkDoneAndEnqueueLinks`, `NextPendingCrawlURL`, per-URL status
//! updates, the `ForEach*` streaming iterators, `GetCrawlJobStats`) are a
//! separate, not-yet-started increment — this file's schema-only comment
//! already covers `CrawlURL`, but its query layer doesn't ride along with
//! `CrawlJob`'s.

use crate::placeholders;
use rusty_db::prelude::*;

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

        for url in urls {
            let p = placeholders(dialect, 4);
            let sql = format!(
                "INSERT INTO crawl_urls \
                    (job_id, url, depth, status, error, error_code, created_at, updated_at) \
                 VALUES ({}, {}, 0, 'pending', '', 0, {}, {}) \
                 ON CONFLICT (job_id, url) DO NOTHING",
                p[0], p[1], p[2], p[3]
            );
            let params: Vec<Value> = vec![
                job_id.clone().into(),
                url.clone().into(),
                now.into(),
                now.into(),
            ];
            tx.execute(&sql, &params).await?;
        }

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
}
