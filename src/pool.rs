use std::path::Path;

use r2d2_sqlite::SqliteConnectionManager;

use crate::error::Result;

/// A pool of SQLite connections, all opened against the same on-disk file.
///
/// Requires the `pool` feature. SQLite allows multiple connections to the
/// same database (WAL mode makes concurrent readers cheap), so a pool is
/// useful for multi-threaded applications that would otherwise serialize on
/// a single [`crate::Connection`].
pub type Pool = r2d2::Pool<SqliteConnectionManager>;

/// A connection checked out from a [`Pool`].
pub type PooledConnection = r2d2::PooledConnection<SqliteConnectionManager>;

/// Builds a [`Pool`] of up to `max_size` connections against the database
/// at `path`, each with WAL journaling and foreign key enforcement enabled.
pub fn build_pool(path: impl AsRef<Path>, max_size: u32) -> Result<Pool> {
    let manager = SqliteConnectionManager::file(path.as_ref()).with_init(|conn| {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        Ok(())
    });
    Ok(r2d2::Pool::builder().max_size(max_size).build(manager)?)
}
