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
//! Two things below are query-layer behavior, not schema, and are **not
//! yet ported** — a deliberate, explicitly-flagged follow-up increment,
//! the same split `rusty-hister-extractor` used (chain-of-responsibility
//! mechanism landed before any concrete extractor):
//!
//! - The domain operations each model file's Go source builds on top of its
//!   table (`EnqueueEmbeddingJob`/`ClaimNextEmbeddingJob`/..., the
//!   `history.go` search/pin/timeline queries, `user.go`'s auth/token
//!   helpers, `CreateCrawlJob`/`CreateNamedCrawlJobWithURLs`, `WebSession`'s
//!   lookup/expiry helpers, `SaveDocumentVersion`/`GetDocumentVersionsUntil`).
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

pub use crawl::{CrawlJob, CrawlJobStatus, CrawlURL, CrawlUrlStatus};
pub use embedding::{EmbeddingJob, EmbeddingJobStatus};
pub use history::{History, HistoryLink, Link};
pub use migrations::{POSTGRES_MIGRATIONS, SQLITE_MIGRATIONS};
pub use session::WebSession;
pub use user::User;
pub use version::DocumentVersion;
