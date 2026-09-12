//! `EmbeddingJob` — a Rust port of `server/model/embedding.go`. A durable,
//! deduplicated request to embed the latest indexed version of a document
//! (backs §5.9's embedding queue). Completed jobs are deleted; failed jobs
//! are retained with their last error until new document contents enqueue
//! them again; a `dirty` job indicates the document changed while a worker
//! was processing it.
//!
//! The queue's state-machine operations are ported below as `EmbeddingJob`
//! associated functions. Three of them (`enqueue`'s upsert, `retry`'s and
//! `release`'s conditional `CASE` updates) need SQL the portable query
//! builder can't express — `rusty_db::Update::set` only ever assigns a
//! plain `Value`, never an `Expr` — so those three drop to
//! `Engine::connect()`/raw `Connection::execute`, built dialect-portably by
//! rendering placeholders through `Engine::dialect().placeholder(..)`
//! rather than hardcoding `?`/`$N`. The rest (`claim_next`'s claim-loop,
//! `complete`, `fail`, `reset_in_progress`, `in_progress_exists`,
//! `delete`) are plain enough to use `Select`/`Update`/`Delete` directly.

use crate::placeholders;
use rusty_db::prelude::*;

/// `EmbeddingJob.status` values (Go: untyped string constants
/// `EmbeddingJobPending`/`EmbeddingJobInProgress`/`EmbeddingJobFailed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, MappedEnum)]
pub enum EmbeddingJobStatus {
    #[default]
    Pending,
    InProgress,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Mapped)]
#[table(name = "embedding_jobs")]
pub struct EmbeddingJob {
    #[table(primary_key)]
    pub doc_id: String,
    #[table(default = "'pending'")]
    pub status: EmbeddingJobStatus,
    #[table(default = "false")]
    pub dirty: bool,
    #[table(default = "0")]
    pub attempts: i64,
    pub available_at: DateTime<Utc>,
    pub last_error: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl EmbeddingJob {
    /// Adds a document to the embedding work set. A pending job already
    /// represents the latest document stored in the index, so re-enqueuing
    /// it is a no-op; enqueuing an active (`InProgress`) job marks it dirty
    /// so it is processed again after the current attempt finishes;
    /// enqueuing a failed job resets its failure state and attempt count.
    pub async fn enqueue(engine: &Engine, doc_id: &str) -> rusty_db::Result<()> {
        let now = Utc::now();
        let p = placeholders(engine.dialect(), 6);
        let sql = format!(
            "INSERT INTO embedding_jobs \
                (doc_id, status, dirty, attempts, available_at, last_error, created_at, updated_at) \
             VALUES ({}, 'pending', false, 0, {}, '', {}, {}) \
             ON CONFLICT (doc_id) DO UPDATE SET \
                status = CASE WHEN embedding_jobs.status = 'in_progress' \
                    THEN embedding_jobs.status ELSE 'pending' END, \
                dirty = CASE WHEN embedding_jobs.status = 'in_progress' \
                    THEN true ELSE embedding_jobs.dirty END, \
                attempts = CASE WHEN embedding_jobs.status = 'in_progress' \
                    THEN embedding_jobs.attempts ELSE 0 END, \
                available_at = CASE WHEN embedding_jobs.status = 'in_progress' \
                    THEN embedding_jobs.available_at ELSE {} END, \
                last_error = CASE WHEN embedding_jobs.status = 'in_progress' \
                    THEN embedding_jobs.last_error ELSE '' END, \
                updated_at = {}",
            p[0], p[1], p[2], p[3], p[4], p[5]
        );
        let params: Vec<Value> = vec![
            doc_id.to_string().into(),
            now.into(),
            now.into(),
            now.into(),
            now.into(),
            now.into(),
        ];
        let mut conn = engine.connect().await?;
        conn.execute(&sql, &params).await?;
        Ok(())
    }

    /// Atomically claims the oldest available pending job (`available_at`
    /// due, ties broken by `created_at`), retrying the select-then-claim
    /// pair when another worker races ahead of it. Returns `None` when no
    /// job is currently available.
    pub async fn claim_next(engine: &Engine) -> rusty_db::Result<Option<Self>> {
        let table = Self::table();
        loop {
            let now = Utc::now();
            let candidate: Option<Self> = engine
                .fetch_optional_as(
                    &Select::from(&table)
                        .filter(table.col("status").eq("pending"))
                        .filter(table.col("available_at").lte(now))
                        .order_by(table.col("available_at").asc())
                        .order_by(table.col("created_at").asc())
                        .limit(1),
                )
                .await?;
            let Some(candidate) = candidate else {
                return Ok(None);
            };

            let affected = engine
                .execute(
                    &Update::table(&table)
                        .set("status", "in_progress")
                        .set("dirty", false)
                        .set("attempts", candidate.attempts + 1)
                        .set("updated_at", now)
                        .filter(table.col("doc_id").eq(candidate.doc_id.clone()))
                        .filter(table.col("status").eq("pending")),
                )
                .await?;
            if affected == 1 {
                return Ok(Some(Self {
                    status: EmbeddingJobStatus::InProgress,
                    dirty: false,
                    attempts: candidate.attempts + 1,
                    updated_at: now,
                    ..candidate
                }));
            }
        }
    }

    /// Deletes a completed job unless it became dirty while active — a
    /// dirty job returns to pending instead, and this returns `true`
    /// (the caller should retry) rather than `false` (genuinely done).
    pub async fn complete(engine: &Engine, doc_id: &str) -> rusty_db::Result<bool> {
        let table = Self::table();
        let deleted = engine
            .execute(
                &Delete::from(&table)
                    .filter(table.col("doc_id").eq(doc_id))
                    .filter(table.col("status").eq("in_progress"))
                    .filter(table.col("dirty").eq(false)),
            )
            .await?;
        if deleted == 1 {
            return Ok(false);
        }

        let now = Utc::now();
        let retried = engine
            .execute(
                &Update::table(&table)
                    .set("status", "pending")
                    .set("dirty", false)
                    .set("attempts", 0_i64)
                    .set("available_at", now)
                    .set("last_error", "")
                    .set("updated_at", now)
                    .filter(table.col("doc_id").eq(doc_id))
                    .filter(table.col("status").eq("in_progress"))
                    .filter(table.col("dirty").eq(true)),
            )
            .await?;
        Ok(retried == 1)
    }

    /// Returns an active job to pending. A dirty job retries immediately
    /// (its latest indexed contents haven't been attempted yet), so its
    /// attempt count/error/availability reset instead of using `retry_at`.
    pub async fn retry(
        engine: &Engine,
        doc_id: &str,
        retry_at: DateTime<Utc>,
        last_error: &str,
    ) -> rusty_db::Result<()> {
        let now = Utc::now();
        let p = placeholders(engine.dialect(), 8);
        let sql = format!(
            "UPDATE embedding_jobs SET \
                status = 'pending', \
                dirty = false, \
                attempts = CASE WHEN dirty = {} THEN 0 ELSE attempts END, \
                available_at = CASE WHEN dirty = {} THEN {} ELSE {} END, \
                last_error = CASE WHEN dirty = {} THEN '' ELSE {} END, \
                updated_at = {} \
             WHERE doc_id = {} AND status = 'in_progress'",
            p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7]
        );
        let params: Vec<Value> = vec![
            true.into(),
            true.into(),
            now.into(),
            retry_at.into(),
            true.into(),
            last_error.to_string().into(),
            now.into(),
            doc_id.to_string().into(),
        ];
        let mut conn = engine.connect().await?;
        conn.execute(&sql, &params).await?;
        Ok(())
    }

    /// Moves an active job to the failed state. If the document changed
    /// while the job was active, its latest contents return to pending
    /// instead (attempt count reset) and this returns `true`.
    pub async fn fail(engine: &Engine, doc_id: &str, last_error: &str) -> rusty_db::Result<bool> {
        let table = Self::table();
        let now = Utc::now();
        let failed = engine
            .execute(
                &Update::table(&table)
                    .set("status", "failed")
                    .set("last_error", last_error)
                    .set("updated_at", now)
                    .filter(table.col("doc_id").eq(doc_id))
                    .filter(table.col("status").eq("in_progress"))
                    .filter(table.col("dirty").eq(false)),
            )
            .await?;
        if failed == 1 {
            return Ok(false);
        }

        let retried = engine
            .execute(
                &Update::table(&table)
                    .set("status", "pending")
                    .set("dirty", false)
                    .set("attempts", 0_i64)
                    .set("available_at", now)
                    .set("last_error", "")
                    .set("updated_at", now)
                    .filter(table.col("doc_id").eq(doc_id))
                    .filter(table.col("status").eq("in_progress"))
                    .filter(table.col("dirty").eq(true)),
            )
            .await?;
        Ok(retried == 1)
    }

    /// Returns a claimed job to pending without counting the interrupted
    /// attempt. Used during queue shutdown and cancellation.
    pub async fn release(engine: &Engine, doc_id: &str) -> rusty_db::Result<()> {
        let now = Utc::now();
        let p = placeholders(engine.dialect(), 3);
        let sql = format!(
            "UPDATE embedding_jobs SET \
                status = 'pending', \
                dirty = false, \
                attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END, \
                available_at = {}, \
                updated_at = {} \
             WHERE doc_id = {} AND status = 'in_progress'",
            p[0], p[1], p[2]
        );
        let params: Vec<Value> = vec![now.into(), now.into(), doc_id.to_string().into()];
        let mut conn = engine.connect().await?;
        conn.execute(&sql, &params).await?;
        Ok(())
    }

    /// Reports whether a worker still owns `doc_id`.
    pub async fn in_progress_exists(engine: &Engine, doc_id: &str) -> rusty_db::Result<bool> {
        let table = Self::table();
        let found: Option<Self> = engine
            .fetch_optional_as(
                &Select::from(&table)
                    .filter(table.col("doc_id").eq(doc_id))
                    .filter(table.col("status").eq("in_progress"))
                    .limit(1),
            )
            .await?;
        Ok(found.is_some())
    }

    /// Removes pending or active work for a deleted document.
    pub async fn delete(engine: &Engine, doc_id: &str) -> rusty_db::Result<()> {
        let table = Self::table();
        engine
            .execute(&Delete::from(&table).filter(table.col("doc_id").eq(doc_id)))
            .await?;
        Ok(())
    }

    /// Recovers jobs interrupted by process shutdown, returning them to
    /// pending. Returns the number of jobs recovered.
    pub async fn reset_in_progress(engine: &Engine) -> rusty_db::Result<u64> {
        let table = Self::table();
        let now = Utc::now();
        engine
            .execute(
                &Update::table(&table)
                    .set("status", "pending")
                    .set("dirty", false)
                    .set("available_at", now)
                    .set("updated_at", now)
                    .filter(table.col("status").eq("in_progress")),
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
        assert_eq!(EmbeddingJob::TABLE_NAME, "embedding_jobs");
    }

    #[test]
    fn status_defaults_to_pending() {
        assert_eq!(EmbeddingJobStatus::default(), EmbeddingJobStatus::Pending);
    }

    #[tokio::test]
    async fn round_trips_through_sqlite() {
        let engine = migrated_engine().await;
        let job = EmbeddingJob {
            doc_id: "doc-1".into(),
            status: EmbeddingJobStatus::Pending,
            dirty: false,
            attempts: 0,
            available_at: now(),
            last_error: String::new(),
            created_at: now(),
            updated_at: now(),
        };
        engine.execute(&job.insert()).await.unwrap();

        let fetched: EmbeddingJob = engine
            .fetch_one_as(&Select::from(&EmbeddingJob::table()))
            .await
            .unwrap();
        assert_eq!(fetched.doc_id, "doc-1");
        assert_eq!(fetched.status, EmbeddingJobStatus::Pending);
    }

    #[tokio::test]
    async fn primary_key_is_doc_id_not_a_surrogate() {
        let engine = migrated_engine().await;
        let job = EmbeddingJob {
            doc_id: "doc-1".into(),
            status: EmbeddingJobStatus::Failed,
            dirty: true,
            attempts: 4,
            available_at: now(),
            last_error: "boom".into(),
            created_at: now(),
            updated_at: now(),
        };
        engine.execute(&job.insert()).await.unwrap();

        let mut session = engine.session();
        let fetched = session
            .get::<EmbeddingJob>("doc-1".to_string())
            .await
            .unwrap()
            .unwrap();
        let fetched = fetched.borrow();
        assert_eq!(fetched.attempts, 4);
        assert!(fetched.dirty);
    }

    #[tokio::test]
    async fn enqueue_creates_a_pending_job() {
        let engine = migrated_engine().await;
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap();

        let fetched: EmbeddingJob = engine
            .fetch_one_as(&Select::from(&EmbeddingJob::table()))
            .await
            .unwrap();
        assert_eq!(fetched.status, EmbeddingJobStatus::Pending);
        assert!(!fetched.dirty);
        assert_eq!(fetched.attempts, 0);
    }

    #[tokio::test]
    async fn enqueue_is_idempotent_for_an_already_pending_job() {
        let engine = migrated_engine().await;
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap();
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap();

        let all: Vec<EmbeddingJob> = engine
            .fetch_all_as(&Select::from(&EmbeddingJob::table()))
            .await
            .unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn enqueue_marks_an_in_progress_job_dirty_instead_of_resetting_it() {
        let engine = migrated_engine().await;
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap();
        let claimed = EmbeddingJob::claim_next(&engine).await.unwrap().unwrap();
        assert_eq!(claimed.status, EmbeddingJobStatus::InProgress);

        // The document changed again while the worker is still processing it.
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap();

        let fetched: EmbeddingJob = engine
            .fetch_one_as(&Select::from(&EmbeddingJob::table()))
            .await
            .unwrap();
        assert_eq!(fetched.status, EmbeddingJobStatus::InProgress);
        assert!(
            fetched.dirty,
            "re-enqueuing an active job should mark it dirty"
        );
        assert_eq!(
            fetched.attempts, claimed.attempts,
            "attempt count is untouched"
        );
    }

    #[tokio::test]
    async fn enqueue_resets_a_failed_job() {
        let engine = migrated_engine().await;
        let failed = EmbeddingJob {
            doc_id: "doc-1".into(),
            status: EmbeddingJobStatus::Failed,
            dirty: false,
            attempts: 3,
            available_at: now(),
            last_error: "boom".into(),
            created_at: now(),
            updated_at: now(),
        };
        engine.execute(&failed.insert()).await.unwrap();

        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap();

        let fetched: EmbeddingJob = engine
            .fetch_one_as(&Select::from(&EmbeddingJob::table()))
            .await
            .unwrap();
        assert_eq!(fetched.status, EmbeddingJobStatus::Pending);
        assert_eq!(fetched.attempts, 0);
        assert_eq!(fetched.last_error, "");
    }

    #[tokio::test]
    async fn claim_next_returns_none_when_queue_is_empty() {
        let engine = migrated_engine().await;
        assert_eq!(EmbeddingJob::claim_next(&engine).await.unwrap(), None);
    }

    #[tokio::test]
    async fn claim_next_picks_the_oldest_available_at_first() {
        let engine = migrated_engine().await;
        let older = EmbeddingJob {
            doc_id: "older".into(),
            status: EmbeddingJobStatus::Pending,
            dirty: false,
            attempts: 0,
            available_at: "2024-01-01T00:00:00Z".parse().unwrap(),
            last_error: String::new(),
            created_at: now(),
            updated_at: now(),
        };
        let newer = EmbeddingJob {
            doc_id: "newer".into(),
            available_at: "2024-02-01T00:00:00Z".parse().unwrap(),
            ..older.clone()
        };
        engine.execute(&newer.insert()).await.unwrap();
        engine.execute(&older.insert()).await.unwrap();

        let claimed = EmbeddingJob::claim_next(&engine).await.unwrap().unwrap();
        assert_eq!(claimed.doc_id, "older");
        assert_eq!(claimed.status, EmbeddingJobStatus::InProgress);
        assert_eq!(claimed.attempts, 1);

        // The claimed job is no longer pending, so the next claim skips it.
        let second = EmbeddingJob::claim_next(&engine).await.unwrap().unwrap();
        assert_eq!(second.doc_id, "newer");
    }

    #[tokio::test]
    async fn claim_next_skips_a_job_not_yet_available() {
        let engine = migrated_engine().await;
        let not_yet = EmbeddingJob {
            doc_id: "future".into(),
            status: EmbeddingJobStatus::Pending,
            dirty: false,
            attempts: 0,
            available_at: "2999-01-01T00:00:00Z".parse().unwrap(),
            last_error: String::new(),
            created_at: now(),
            updated_at: now(),
        };
        engine.execute(&not_yet.insert()).await.unwrap();

        assert_eq!(EmbeddingJob::claim_next(&engine).await.unwrap(), None);
    }

    #[tokio::test]
    async fn complete_deletes_a_clean_job() {
        let engine = migrated_engine().await;
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap();
        EmbeddingJob::claim_next(&engine).await.unwrap();

        let retry = EmbeddingJob::complete(&engine, "doc-1").await.unwrap();
        assert!(!retry);

        let remaining: Vec<EmbeddingJob> = engine
            .fetch_all_as(&Select::from(&EmbeddingJob::table()))
            .await
            .unwrap();
        assert!(remaining.is_empty());
    }

    #[tokio::test]
    async fn complete_returns_a_dirty_job_to_pending_instead_of_deleting_it() {
        let engine = migrated_engine().await;
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap();
        EmbeddingJob::claim_next(&engine).await.unwrap();
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap(); // marks it dirty

        let retry = EmbeddingJob::complete(&engine, "doc-1").await.unwrap();
        assert!(retry, "a dirty job should ask the caller to retry");

        let fetched: EmbeddingJob = engine
            .fetch_one_as(&Select::from(&EmbeddingJob::table()))
            .await
            .unwrap();
        assert_eq!(fetched.status, EmbeddingJobStatus::Pending);
        assert!(!fetched.dirty);
        assert_eq!(fetched.attempts, 0);
    }

    #[tokio::test]
    async fn retry_reschedules_a_clean_job_for_later() {
        let engine = migrated_engine().await;
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap();
        EmbeddingJob::claim_next(&engine).await.unwrap();

        let retry_at: DateTime<Utc> = "2030-01-01T00:00:00Z".parse().unwrap();
        EmbeddingJob::retry(&engine, "doc-1", retry_at, "temporary failure")
            .await
            .unwrap();

        let fetched: EmbeddingJob = engine
            .fetch_one_as(&Select::from(&EmbeddingJob::table()))
            .await
            .unwrap();
        assert_eq!(fetched.status, EmbeddingJobStatus::Pending);
        assert_eq!(fetched.available_at, retry_at);
        assert_eq!(fetched.last_error, "temporary failure");
    }

    #[tokio::test]
    async fn retry_retries_immediately_when_the_job_is_dirty() {
        let engine = migrated_engine().await;
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap();
        EmbeddingJob::claim_next(&engine).await.unwrap();
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap(); // marks it dirty

        let retry_at: DateTime<Utc> = "2030-01-01T00:00:00Z".parse().unwrap();
        EmbeddingJob::retry(&engine, "doc-1", retry_at, "temporary failure")
            .await
            .unwrap();

        let fetched: EmbeddingJob = engine
            .fetch_one_as(&Select::from(&EmbeddingJob::table()))
            .await
            .unwrap();
        assert_eq!(fetched.status, EmbeddingJobStatus::Pending);
        assert_ne!(
            fetched.available_at, retry_at,
            "a dirty job retries immediately, not at retry_at"
        );
        assert_eq!(fetched.attempts, 0);
        assert_eq!(fetched.last_error, "");
    }

    #[tokio::test]
    async fn fail_moves_a_clean_job_to_failed() {
        let engine = migrated_engine().await;
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap();
        EmbeddingJob::claim_next(&engine).await.unwrap();

        let retry = EmbeddingJob::fail(&engine, "doc-1", "boom").await.unwrap();
        assert!(!retry);

        let fetched: EmbeddingJob = engine
            .fetch_one_as(&Select::from(&EmbeddingJob::table()))
            .await
            .unwrap();
        assert_eq!(fetched.status, EmbeddingJobStatus::Failed);
        assert_eq!(fetched.last_error, "boom");
    }

    #[tokio::test]
    async fn fail_returns_a_dirty_job_to_pending_instead_of_failing_it() {
        let engine = migrated_engine().await;
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap();
        EmbeddingJob::claim_next(&engine).await.unwrap();
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap(); // marks it dirty

        let retry = EmbeddingJob::fail(&engine, "doc-1", "boom").await.unwrap();
        assert!(retry);

        let fetched: EmbeddingJob = engine
            .fetch_one_as(&Select::from(&EmbeddingJob::table()))
            .await
            .unwrap();
        assert_eq!(fetched.status, EmbeddingJobStatus::Pending);
    }

    #[tokio::test]
    async fn release_returns_a_claimed_job_to_pending_and_decrements_attempts() {
        let engine = migrated_engine().await;
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap();
        EmbeddingJob::claim_next(&engine).await.unwrap();

        EmbeddingJob::release(&engine, "doc-1").await.unwrap();

        let fetched: EmbeddingJob = engine
            .fetch_one_as(&Select::from(&EmbeddingJob::table()))
            .await
            .unwrap();
        assert_eq!(fetched.status, EmbeddingJobStatus::Pending);
        assert_eq!(
            fetched.attempts, 0,
            "the interrupted attempt is not counted"
        );
    }

    #[tokio::test]
    async fn release_never_lets_attempts_go_negative() {
        let engine = migrated_engine().await;
        let job = EmbeddingJob {
            doc_id: "doc-1".into(),
            status: EmbeddingJobStatus::InProgress,
            dirty: false,
            attempts: 0,
            available_at: now(),
            last_error: String::new(),
            created_at: now(),
            updated_at: now(),
        };
        engine.execute(&job.insert()).await.unwrap();

        EmbeddingJob::release(&engine, "doc-1").await.unwrap();

        let fetched: EmbeddingJob = engine
            .fetch_one_as(&Select::from(&EmbeddingJob::table()))
            .await
            .unwrap();
        assert_eq!(fetched.attempts, 0);
    }

    #[tokio::test]
    async fn in_progress_exists_reflects_current_state() {
        let engine = migrated_engine().await;
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap();
        assert!(!EmbeddingJob::in_progress_exists(&engine, "doc-1")
            .await
            .unwrap());

        EmbeddingJob::claim_next(&engine).await.unwrap();
        assert!(EmbeddingJob::in_progress_exists(&engine, "doc-1")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn delete_removes_pending_and_active_work() {
        let engine = migrated_engine().await;
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap();

        EmbeddingJob::delete(&engine, "doc-1").await.unwrap();

        let remaining: Vec<EmbeddingJob> = engine
            .fetch_all_as(&Select::from(&EmbeddingJob::table()))
            .await
            .unwrap();
        assert!(remaining.is_empty());
    }

    #[tokio::test]
    async fn reset_in_progress_recovers_interrupted_jobs() {
        let engine = migrated_engine().await;
        EmbeddingJob::enqueue(&engine, "doc-1").await.unwrap();
        EmbeddingJob::claim_next(&engine).await.unwrap();

        let recovered = EmbeddingJob::reset_in_progress(&engine).await.unwrap();
        assert_eq!(recovered, 1);

        let fetched: EmbeddingJob = engine
            .fetch_one_as(&Select::from(&EmbeddingJob::table()))
            .await
            .unwrap();
        assert_eq!(fetched.status, EmbeddingJobStatus::Pending);
        assert!(!fetched.dirty);
    }
}
