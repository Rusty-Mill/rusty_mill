use std::path::Path;
use std::time::Duration;

use rusqlite::Connection as RawConnection;

use crate::error::Result;
use crate::migration::Migrations;

/// Pragmas applied when a connection is opened.
///
/// The defaults favor safe concurrent access for an application-embedded
/// database: WAL journaling so readers don't block a writer, foreign key
/// enforcement on, and a busy timeout so concurrent writers block-and-retry
/// instead of failing immediately with `SQLITE_BUSY`.
#[derive(Debug, Clone)]
pub struct OpenOptions {
    pub wal: bool,
    pub foreign_keys: bool,
    pub busy_timeout: Duration,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            wal: true,
            foreign_keys: true,
            busy_timeout: Duration::from_secs(5),
        }
    }
}

/// A single SQLite connection, opened with [`OpenOptions`]'s pragmas applied.
///
/// This wraps [`rusqlite::Connection`] rather than replacing it: reach for
/// [`Connection::as_raw`]/[`Connection::as_raw_mut`] for anything not
/// exposed directly, and [`Connection::into_raw`] to drop down to `rusqlite`
/// entirely.
pub struct Connection {
    raw: RawConnection,
}

impl Connection {
    /// Opens a private, in-memory database. Useful for tests and ephemeral
    /// caches; nothing is persisted.
    pub fn open_in_memory() -> Result<Self> {
        let raw = RawConnection::open_in_memory()?;
        let conn = Self { raw };
        conn.apply_pragmas(&OpenOptions::default())?;
        Ok(conn)
    }

    /// Opens (creating if needed) an on-disk database at `path`, applying
    /// the default [`OpenOptions`].
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, OpenOptions::default())
    }

    /// Opens (creating if needed) an on-disk database at `path` with custom
    /// [`OpenOptions`].
    pub fn open_with(path: impl AsRef<Path>, options: OpenOptions) -> Result<Self> {
        let raw = RawConnection::open(path)?;
        let conn = Self { raw };
        conn.apply_pragmas(&options)?;
        Ok(conn)
    }

    fn apply_pragmas(&self, options: &OpenOptions) -> Result<()> {
        if options.wal {
            self.raw.pragma_update(None, "journal_mode", "WAL")?;
        }
        // Bundled SQLite compiles with foreign key enforcement on by
        // default, so this must always be set explicitly rather than only
        // when enabling it, or `foreign_keys: false` would be a no-op.
        self.raw
            .pragma_update(None, "foreign_keys", options.foreign_keys)?;
        self.raw.busy_timeout(options.busy_timeout)?;
        Ok(())
    }

    /// Applies every not-yet-applied step in `migrations` to this
    /// connection. See [`Migrations::run`].
    pub fn migrate(&mut self, migrations: &Migrations) -> Result<()> {
        migrations.run(&mut self.raw)
    }

    /// Borrows the underlying [`rusqlite::Connection`].
    pub fn as_raw(&self) -> &RawConnection {
        &self.raw
    }

    /// Mutably borrows the underlying [`rusqlite::Connection`].
    pub fn as_raw_mut(&mut self) -> &mut RawConnection {
        &mut self.raw
    }

    /// Consumes this wrapper, returning the underlying [`rusqlite::Connection`].
    pub fn into_raw(self) -> RawConnection {
        self.raw
    }
}
