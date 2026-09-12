//! Hister's persisted schema — a Rust port of `server/model/` (GORM models)
//! onto [`rusty_db`](../../../rusty_db), dual SQLite/Postgres.
//!
//! **Bootstrap stage — no implementation yet.** See
//! `docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md` §7 for the ten
//! models to port (`Database`, `History`, `Link`, `HistoryLink`, `User`,
//! `CrawlJob`, `CrawlURL`, `DocumentVersion`, `EmbeddingJob`, `WebSession`),
//! the two-phase pre/post migration mechanism, the UTC-everywhere timestamp
//! discipline (§7.1), and the legacy pre-GORM `indexer_versions` read path
//! (§7.3) that a naive port would silently drop.
