use rusqlite::Connection as RawConnection;

use crate::error::{Error, Result};

/// A single, numbered schema change.
///
/// Versions are tracked via SQLite's `PRAGMA user_version`, so they must be
/// positive and registered in strictly increasing order.
pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

/// An ordered list of [`Migration`]s that can be applied to a connection.
///
/// ```
/// use rusty_sqlite::Migrations;
///
/// let migrations = Migrations::new()
///     .add(1, "create notes", "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL);")
///     .add(2, "add created_at", "ALTER TABLE notes ADD COLUMN created_at TEXT;");
///
/// let mut conn = rusty_sqlite::Connection::open_in_memory().unwrap();
/// conn.migrate(&migrations).unwrap();
/// ```
#[derive(Default)]
pub struct Migrations {
    steps: Vec<Migration>,
}

impl Migrations {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a migration step. Steps must be added in strictly
    /// increasing `version` order; this is validated when [`Migrations::run`]
    /// is called, not here, so the builder chain stays infallible.
    pub fn add(mut self, version: i64, name: &'static str, sql: &'static str) -> Self {
        self.steps.push(Migration { version, name, sql });
        self
    }

    /// The highest registered migration version, or `0` if none are registered.
    pub fn latest_version(&self) -> i64 {
        self.steps.last().map(|m| m.version).unwrap_or(0)
    }

    fn validate_order(&self) -> Result<()> {
        for pair in self.steps.windows(2) {
            if pair[1].version <= pair[0].version {
                return Err(Error::OutOfOrderMigration {
                    version: pair[1].version,
                    previous: pair[0].version,
                });
            }
        }
        Ok(())
    }

    /// Applies every migration whose version is greater than the
    /// connection's current `user_version`, each in its own transaction,
    /// advancing `user_version` after every successful step. Already-applied
    /// migrations are skipped, so this is safe to call on every startup.
    pub fn run(&self, conn: &mut RawConnection) -> Result<()> {
        self.validate_order()?;

        let current: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let latest = self.latest_version();
        if current > latest {
            return Err(Error::SchemaTooNew {
                path: conn.path().map(Into::into),
                found: current,
                latest,
            });
        }

        for step in self.steps.iter().filter(|m| m.version > current) {
            let tx = conn.transaction()?;
            tx.execute_batch(step.sql)
                .map_err(|source| Error::Migration {
                    version: step.version,
                    name: step.name.to_string(),
                    source,
                })?;
            tx.pragma_update(None, "user_version", step.version)?;
            tx.commit()?;
        }

        Ok(())
    }
}
