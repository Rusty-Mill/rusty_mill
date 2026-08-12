use std::path::PathBuf;

/// The error type returned by all fallible `rusty_sqlite` operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A `rusqlite`/SQLite call failed.
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// A migration step failed to apply.
    #[error("migration {version} (\"{name}\") failed: {source}")]
    Migration {
        version: i64,
        name: String,
        #[source]
        source: rusqlite::Error,
    },

    /// The database's recorded schema version is newer than any migration
    /// this build knows about, which means it was written by a newer
    /// version of the application and should not be touched.
    #[error(
        "database{} has schema version {found}, which is newer than the {latest} migrations known to this build"
    , .path.as_ref().map(|p| format!(" at {p:?}")).unwrap_or_default())]
    SchemaTooNew {
        path: Option<PathBuf>,
        found: i64,
        latest: i64,
    },

    /// Migrations were registered out of order.
    #[error(
        "migrations must be registered in strictly increasing version order; version {version} was registered after version {previous}"
    )]
    OutOfOrderMigration { version: i64, previous: i64 },

    /// Building or checking out a pooled connection failed.
    #[cfg(feature = "pool")]
    #[error("connection pool error: {0}")]
    Pool(#[from] r2d2::Error),
}

/// A `Result` alias using [`Error`] as its error type.
pub type Result<T> = std::result::Result<T, Error>;
