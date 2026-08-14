//! WAL-adjacent hook types (Part B gap row "hooks module: Wal,
//! CheckpointMode types"). Inert scaffolding: this crate has no WAL
//! (write-ahead log) support — see `ARCHITECTURE.md`'s non-goals — so
//! nothing produces a [`Wal`] value or acts on [`CheckpointMode`] yet.
//! These types exist so the eventual `Connection::wal_hook` (tracked
//! separately) has somewhere to point, without that decision blocking on
//! WAL support landing first.

/// The checkpoint mode SQLite's `wal_checkpoint` operation would run
/// under. See <https://www.sqlite.org/c3ref/wal_checkpoint_v2.html>.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointMode {
    /// Checkpoint as many frames as possible without waiting for readers
    /// or writers to finish.
    Passive,
    /// Block until all readers and writers are done, then checkpoint.
    Full,
    /// Like `Full`, and additionally block new readers/writers until the
    /// checkpoint completes.
    Restart,
    /// Like `Restart`, and additionally truncate the WAL file afterward.
    Truncate,
}

/// A handle to a database's write-ahead log, as passed to a
/// `Connection::wal_hook` callback (not yet implemented — this type is
/// inert until that lands).
#[derive(Debug)]
pub struct Wal {
    database_name: String,
}

impl Wal {
    /// Returns the name of the database this WAL handle refers to (e.g.
    /// `"main"`).
    pub fn database_name(&self) -> &str {
        &self.database_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_mode_variants_are_distinct() {
        assert_ne!(CheckpointMode::Passive, CheckpointMode::Full);
        assert_ne!(CheckpointMode::Restart, CheckpointMode::Truncate);
        assert_eq!(CheckpointMode::Passive, CheckpointMode::Passive);
    }

    #[test]
    fn wal_exposes_database_name() {
        let wal = Wal {
            database_name: "main".to_string(),
        };
        assert_eq!(wal.database_name(), "main");
    }
}
