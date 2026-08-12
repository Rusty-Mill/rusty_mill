//! A thin, ergonomic wrapper over [`rusqlite`] for embedding SQLite as an
//! application's persistence layer.
//!
//! This crate does not try to replace `rusqlite` — it re-exports it — and
//! instead fills three gaps that come up in every consumer that embeds
//! SQLite directly:
//!
//! - **Cross-platform connections by construction**: [`Connection::open`]
//!   and [`Connection::open_in_memory`] use `rusqlite`'s `bundled` feature
//!   (compiled-in SQLite, no system dependency) and apply sane default
//!   pragmas (WAL journaling, foreign keys, a busy timeout).
//! - **Typed FTS5 schema building**: [`Fts5TableBuilder`] renders
//!   `CREATE VIRTUAL TABLE ... USING fts5(...)` statements, which
//!   `rusqlite` only ever exposes as hand-written SQL.
//! - **Migration lifecycle management**: [`Migrations`] tracks schema
//!   version via `PRAGMA user_version` and applies pending steps in order,
//!   each in its own transaction.
//!
//! Enable the `pool` feature for an [`r2d2`]-backed connection [`Pool`] for
//! multi-threaded applications.

mod connection;
mod error;
mod fts5;
mod migration;
#[cfg(feature = "pool")]
mod pool;

pub use connection::{Connection, OpenOptions};
pub use error::{Error, Result};
pub use fts5::{Fts5TableBuilder, Fts5Tokenizer};
pub use migration::{Migration, Migrations};
#[cfg(feature = "pool")]
pub use pool::{build_pool, Pool, PooledConnection};

pub use rusqlite;
