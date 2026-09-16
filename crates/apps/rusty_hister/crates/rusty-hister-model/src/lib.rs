//! Hister's persisted schema — a Rust port of `server/model/` (GORM models)
//! onto [`rusty_db`](../../../rusty_db), dual SQLite/Postgres.
//!
//! This increment ports the **schema**: the nine `#[derive(Mapped)]` types
//! from `automigrate()`'s list (capability inventory §7.2), and the
//! fresh-install migration that creates them ([`SQLITE_MIGRATIONS`],
//! [`POSTGRES_MIGRATIONS`]). Hister's own `Database` singleton-row
//! schema-version tracker has no equivalent here — `rusty_db`'s
//! [`rusty_db::Migrator`] already solves the same problem (tracking which
//! migrations have applied) via its own bookkeeping table, so this is a
//! substitution, not a dropped capability.
//!
//! Six of the nine model files' query layers are ported too:
//!
//! - `EmbeddingJob::{enqueue, claim_next, complete, retry, fail, release,
//!   in_progress_exists, delete, reset_in_progress}` (§5.9's embedding
//!   queue).
//! - `WebSession::{create, get, refresh, delete}` (`refresh`, not
//!   `update`, since `#[derive(Mapped)]` already generates an `update()`
//!   instance method a same-named associated function would collide
//!   with).
//! - `DocumentVersion::{save, move_versions, count, list, list_until}`.
//! - `CrawlJob::{generate_id, create, create_with_urls, get,
//!   update_status, list, delete}` and `CrawlURL::{insert_if_not_exists,
//!   bulk_insert, mark_done_and_enqueue_links, insert_done, next_pending,
//!   update_status, mark_failed, reset_in_progress, count_by_status,
//!   count, list_failed, list, job_stats}` — together, all of
//!   `crawl.go`.
//! - `Link::get_or_create`/`History::get_or_create` and
//!   `HistoryLink::{delete_by_user_and_url, delete_by_user_query_and_url,
//!   set_pinned, record_selection, urls_by_query, latest_items,
//!   timestamps, suggest_query}` — together, all of `history.go`.
//!
//! Everything else below is still query-layer behavior, not schema, and
//! is **not yet ported** — a deliberate, explicitly-flagged follow-up
//! increment, the same split `rusty-hister-extractor` used
//! (chain-of-responsibility mechanism landed before any concrete
//! extractor):
//!
//! - `user.go`'s auth/token helpers (password hashing, token regen, admin
//!   flag).
//! - The legacy pre-GORM `indexer_versions` read path (capability inventory
//!   §7.3) and whether `rusty_hister` needs to *open* a pre-existing
//!   Hister-Go-created database file at all — both still-open items, see
//!   `docs/PROJECT-STATUS.md`.
//!
//! `DocumentType`'s `rusty-hister-core` representation stays a plain Rust
//! enum, not a `#[derive(MappedEnum)]` one, precisely so `rusty-hister-core`
//! stays free of a `rusty_db` dependency (ADR-0001's crate-split
//! rationale) — this crate's document-adjacent tables reference documents
//! by URL/ID, not by embedding `rusty_hister_core::DocumentType` directly.

mod crawl;
mod embedding;
mod history;
mod migrations;
mod session;
mod user;
mod version;

pub use crawl::{CrawlJob, CrawlJobStats, CrawlJobStatus, CrawlURL, CrawlUrlStatus};
pub use embedding::{EmbeddingJob, EmbeddingJobStatus};
pub use history::{History, HistoryItem, HistoryItemsFilter, HistoryLink, Link, UrlCount};
pub use migrations::{POSTGRES_MIGRATIONS, SQLITE_MIGRATIONS};
pub use session::WebSession;
pub use user::{RenameOutcome, User};
pub use version::DocumentVersion;

/// Renders `count` placeholders in `dialect`'s own syntax (`?`, `$1`, ...),
/// 1-indexed. Shared by the query-layer functions that need raw SQL for a
/// `CASE`-based `SET` clause or a database-assigned surrogate key —
/// anything `rusty_db`'s query builder doesn't cover — so the same SQL
/// text still works against every supported backend.
pub(crate) fn placeholders(dialect: &dyn rusty_db::Dialect, count: usize) -> Vec<String> {
    (1..=count).map(|i| dialect.placeholder(i)).collect()
}
