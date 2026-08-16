//! Hook types (Part B gap row "hooks module"): the WAL-adjacent
//! [`Wal`]/[`CheckpointMode`] scaffolding below, plus the types used by
//! [`crate::Connection`]'s `commit_hook`/`rollback_hook`/`update_hook`/
//! `authorizer`/`trace`/`profile`/`progress_handler` setters (issue #20's
//! "Connection + hooks module" gap).
//!
//! **Design deviation, stated plainly:** real SQLite's [`Action`] (shared
//! between the update hook and the authorizer callback) has roughly 30
//! variants — one per `SQLITE_*` action code, covering things like
//! `CREATE INDEX`, `ATTACH`, `PRAGMA`, and trigger/view operations this
//! engine has no concept of. [`Action`] here only lists the operations
//! this engine can actually perform (`CREATE TABLE`, `INSERT`, `SELECT`)
//! — there's no value in a `CreateIndex` variant that could never be
//! constructed. Extend it as real statement types land.

/// A row-level change reported to `Connection::update_hook`, or an
/// operation reported to `Connection::authorizer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    CreateTable,
    /// `DROP TABLE` (issue #120).
    DropTable,
    Insert,
    Select,
    /// Not yet reachable — no `UPDATE` statement exists yet. Kept so
    /// `update_hook` callers can match on it now rather than needing a
    /// breaking enum change once `UPDATE` lands.
    Update,
    /// Not yet reachable — no `DELETE` statement exists yet. Same
    /// reasoning as `Update`.
    Delete,
}

/// What `Connection::authorizer` is being asked to allow or deny.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthContext {
    pub action: Action,
    /// The table the action targets. `None` for actions that aren't
    /// table-scoped (none currently — kept as `Option` for forward
    /// compatibility with a future non-table-scoped `Action`).
    pub table_name: Option<String>,
}

/// An authorizer callback's verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authorization {
    Allow,
    Deny,
    /// Treated identically to `Deny` by this engine. Real SQLite's
    /// `Ignore` makes a denied *column read* silently return `NULL`
    /// instead of erroring the whole statement — there's no per-column
    /// read authorization here (only whole-statement/whole-table) to make
    /// that distinction meaningful.
    Ignore,
}

/// What transaction boundary is being crossed. Inert scaffolding, same
/// status as [`Wal`]/[`CheckpointMode`] below: this crate's explicit
/// transactions are started via [`crate::Connection::transaction`], a
/// Rust API call, not a parsed `BEGIN` SQL statement, so there's no hook
/// today that would report a boundary using this type. Kept so a future
/// `BEGIN`-parsing PR has a type to report through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionOperation {
    Begin,
    Commit,
    Rollback,
}

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

    #[test]
    fn auth_context_carries_action_and_table() {
        let ctx = AuthContext {
            action: Action::Insert,
            table_name: Some("t".to_string()),
        };
        assert_eq!(ctx.action, Action::Insert);
        assert_eq!(ctx.table_name.as_deref(), Some("t"));
    }

    #[test]
    fn action_and_authorization_variants_are_distinct() {
        assert_ne!(Action::Insert, Action::Select);
        assert_ne!(Authorization::Allow, Authorization::Deny);
        assert_ne!(Authorization::Deny, Authorization::Ignore);
    }

    #[test]
    fn transaction_operation_variants_are_distinct() {
        assert_ne!(TransactionOperation::Begin, TransactionOperation::Commit);
        assert_ne!(TransactionOperation::Commit, TransactionOperation::Rollback);
    }
}
