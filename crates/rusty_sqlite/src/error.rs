use std::fmt;
use std::path::PathBuf;

/// The error type returned by all fallible `rusty_sqlite` operations.
#[derive(Debug)]
pub enum Error {
    /// A `rusqlite`/SQLite call failed.
    Sqlite(rusqlite::Error),

    /// A migration step failed to apply.
    Migration {
        version: i64,
        name: String,
        source: rusqlite::Error,
    },

    /// The database's recorded schema version is newer than any migration
    /// this build knows about, which means it was written by a newer
    /// version of the application and should not be touched.
    SchemaTooNew {
        path: Option<PathBuf>,
        found: i64,
        latest: i64,
    },

    /// Migrations were registered out of order.
    OutOfOrderMigration { version: i64, previous: i64 },

    /// A pooled connection could not be acquired within the pool's timeout.
    #[cfg(feature = "pool")]
    PoolTimeout,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Sqlite(e) => write!(f, "sqlite error: {e}"),
            Error::Migration {
                version,
                name,
                source,
            } => write!(f, "migration {version} (\"{name}\") failed: {source}"),
            Error::SchemaTooNew {
                path,
                found,
                latest,
            } => {
                let at = path
                    .as_ref()
                    .map(|p| format!(" at {p:?}"))
                    .unwrap_or_default();
                write!(
                    f,
                    "database{at} has schema version {found}, which is newer than the {latest} migrations known to this build"
                )
            }
            Error::OutOfOrderMigration { version, previous } => write!(
                f,
                "migrations must be registered in strictly increasing version order; version {version} was registered after version {previous}"
            ),
            #[cfg(feature = "pool")]
            Error::PoolTimeout => write!(f, "timed out waiting for a pooled connection"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Sqlite(e) => Some(e),
            Error::Migration { source, .. } => Some(source),
            Error::SchemaTooNew { .. } | Error::OutOfOrderMigration { .. } => None,
            #[cfg(feature = "pool")]
            Error::PoolTimeout => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sqlite(e)
    }
}

/// A `Result` alias using [`Error`] as its error type.
pub type Result<T> = std::result::Result<T, Error>;
