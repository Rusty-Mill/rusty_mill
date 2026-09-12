//! `CrawlJob` and `CrawlURL` — a Rust port of `server/model/crawl.go`'s
//! persistent-crawl-job tables (backs the CLI's crawl-job subtree, §3.1).
//! Neither embeds Hister's `CommonFields` (no soft delete) in the Go
//! source, so neither does its Rust equivalent here. The job-lifecycle
//! helpers (`CreateCrawlJob`, `CreateNamedCrawlJobWithURLs`, ...) are
//! query-layer behavior, not schema, and are a follow-up increment — see
//! the crate root docs.

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
}
