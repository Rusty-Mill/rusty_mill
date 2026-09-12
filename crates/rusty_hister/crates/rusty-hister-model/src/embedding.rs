//! `EmbeddingJob` — a Rust port of `server/model/embedding.go`. A durable,
//! deduplicated request to embed the latest indexed version of a document
//! (backs §5.9's embedding queue). Completed jobs are deleted; failed jobs
//! are retained with their last error until new document contents enqueue
//! them again; a `dirty` job indicates the document changed while a worker
//! was processing it.
//!
//! The queue's actual state-machine operations (`EnqueueEmbeddingJob`,
//! `ClaimNextEmbeddingJob`, `CompleteEmbeddingJob`, `RetryEmbeddingJob`,
//! `FailEmbeddingJob`, `ReleaseEmbeddingJob`, `ResetInProgressEmbeddingJobs`)
//! are query-layer behavior — several of them (`EnqueueEmbeddingJob`'s
//! `ON CONFLICT` upsert, `ClaimNextEmbeddingJob`'s claim-loop) are also
//! genuinely dialect-sensitive SQL, not schema, and are a follow-up
//! increment — see the crate root docs.

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
}
