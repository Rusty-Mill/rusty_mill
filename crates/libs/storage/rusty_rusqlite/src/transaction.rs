//! `Transaction`/`Savepoint`: RAII guards over a snapshot-based rollback
//! mechanism (Part B gap rows "Connection: transaction & savepoint
//! management" and "Transaction: new*, savepoint*, drop_behavior,
//! commit, rollback, finish" / "Savepoint").

use crate::connection::Connection;
use crate::error::Result;
use crate::storage::Table;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

/// The locking mode requested for a transaction. Accepted for API-shape
/// compatibility with `rusqlite` only — this crate's single-writer
/// in-memory model doesn't distinguish between them, so all three behave
/// identically today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionBehavior {
    Deferred,
    Immediate,
    Exclusive,
}

/// The transaction-lock state for a database, as returned by
/// [`crate::Connection::transaction_state`].
///
/// **Design deviation, stated plainly:** real SQLite distinguishes `Read`
/// (a read transaction is open) from `Write` (a write transaction is
/// open) lock states. This crate's single-writer in-memory snapshot
/// model has no separate read/write lock to distinguish — any open
/// [`Transaction`]/[`Savepoint`] (at any nesting depth) reports as
/// `Write`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionState {
    /// No transaction is open.
    None,
    /// A transaction (or nested savepoint) is open.
    Write,
}

/// What happens to an unfinished [`Transaction`]/[`Savepoint`] when it's
/// dropped. Missing `rusqlite::DropBehavior::Ignore`: that variant leaves
/// the transaction open past the guard's lifetime, which doesn't fit this
/// crate's ownership-based (rather than handle-based) transaction model —
/// there is no "still open, but nothing references it" state to be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropBehavior {
    /// Roll back to the pre-transaction snapshot. The default.
    Rollback,
    /// Keep whatever changes were made.
    Commit,
    /// Panic instead of silently doing either.
    Panic,
}

/// An RAII transaction guard. Rolls back on drop unless [`Transaction::commit`]
/// was called or [`Transaction::set_drop_behavior`] was changed.
pub struct Transaction<'conn> {
    conn: &'conn mut Connection,
    snapshot: Option<HashMap<String, Table>>,
    drop_behavior: DropBehavior,
    finished: bool,
}

impl<'conn> Transaction<'conn> {
    pub(crate) fn new(conn: &'conn mut Connection) -> Result<Transaction<'conn>> {
        let snapshot = conn.snapshot_db();
        conn.db().notify_virtual_tables_begin()?;
        conn.increment_transaction_depth();
        Ok(Transaction {
            conn,
            snapshot: Some(snapshot),
            drop_behavior: DropBehavior::Rollback,
            finished: false,
        })
    }

    /// Commits: keeps the changes made since the transaction began.
    /// Notifies any [`crate::TransactionVTab`]-participating virtual
    /// tables (issue #95) — this guard's own snapshot/restore mechanism
    /// only ever covered native tables.
    pub fn commit(mut self) -> Result<()> {
        self.conn.db().notify_virtual_tables_commit()?;
        self.snapshot = None;
        self.mark_finished();
        Ok(())
    }

    /// Rolls back to the pre-transaction snapshot, and notifies any
    /// [`crate::TransactionVTab`]-participating virtual tables (issue
    /// #95) so they can undo their own state the same way.
    pub fn rollback(mut self) -> Result<()> {
        if let Some(snapshot) = self.snapshot.take() {
            self.conn.restore_db(snapshot);
            self.conn.db().notify_virtual_tables_rollback()?;
            self.conn.fire_rollback_hook();
        }
        self.mark_finished();
        Ok(())
    }

    /// Commits or rolls back per [`Transaction::drop_behavior`], then
    /// consumes the guard. Equivalent to just letting it drop, spelled
    /// out for callers that want the outcome explicit at the call site.
    pub fn finish(mut self) -> Result<()> {
        self.finish_mut()
    }

    fn finish_mut(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        match self.drop_behavior {
            DropBehavior::Commit => {
                self.conn.db().notify_virtual_tables_commit()?;
                self.snapshot = None;
            }
            DropBehavior::Rollback => {
                if let Some(snapshot) = self.snapshot.take() {
                    self.conn.restore_db(snapshot);
                    self.conn.db().notify_virtual_tables_rollback()?;
                    self.conn.fire_rollback_hook();
                }
            }
            DropBehavior::Panic => panic!("Transaction dropped without commit or rollback"),
        }
        self.mark_finished();
        Ok(())
    }

    /// Marks this guard as finished and lets the connection know its
    /// transaction depth has decreased by one. The single place all of
    /// `commit`/`rollback`/`finish_mut` funnel through, so depth
    /// tracking can't drift out of sync with which of those actually ran.
    fn mark_finished(&mut self) {
        self.conn.decrement_transaction_depth();
        self.finished = true;
    }

    /// Returns what will happen if this guard is dropped without an
    /// explicit `commit`/`rollback`.
    pub fn drop_behavior(&self) -> DropBehavior {
        self.drop_behavior
    }

    /// Sets what happens if this guard is dropped without an explicit
    /// `commit`/`rollback`.
    pub fn set_drop_behavior(&mut self, behavior: DropBehavior) {
        self.drop_behavior = behavior;
    }
}

impl<'conn> Drop for Transaction<'conn> {
    fn drop(&mut self) {
        let _ = self.finish_mut();
    }
}

impl<'conn> Deref for Transaction<'conn> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn
    }
}

impl<'conn> DerefMut for Transaction<'conn> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.conn
    }
}

/// An RAII savepoint guard — the same mechanism as [`Transaction`], plus a
/// name. Real SQLite savepoints nest (a savepoint started inside another
/// savepoint or transaction); this crate's snapshot-based rollback nests
/// correctly too, since each guard independently captures and restores
/// the full table state at its own start/end.
pub struct Savepoint<'conn> {
    name: Option<String>,
    inner: Transaction<'conn>,
}

impl<'conn> Savepoint<'conn> {
    pub(crate) fn new(
        conn: &'conn mut Connection,
        name: Option<String>,
    ) -> Result<Savepoint<'conn>> {
        Ok(Savepoint {
            name,
            inner: Transaction::new(conn)?,
        })
    }

    /// Returns this savepoint's name, if one was given.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Commits: keeps the changes made since the savepoint began.
    pub fn commit(self) -> Result<()> {
        self.inner.commit()
    }

    /// Rolls back to the pre-savepoint snapshot.
    pub fn rollback(self) -> Result<()> {
        self.inner.rollback()
    }
}

impl<'conn> Deref for Savepoint<'conn> {
    type Target = Transaction<'conn>;
    fn deref(&self) -> &Transaction<'conn> {
        &self.inner
    }
}

impl<'conn> DerefMut for Savepoint<'conn> {
    fn deref_mut(&mut self) -> &mut Transaction<'conn> {
        &mut self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn_with_table() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn
    }

    #[test]
    fn commit_keeps_changes() {
        let mut conn = conn_with_table();
        {
            let mut tx = conn.transaction().unwrap();
            tx.execute("INSERT INTO t VALUES (1)").unwrap();
            tx.commit().unwrap();
        }
        let rows: Vec<i64> = conn
            .query_map("SELECT * FROM t", |row| row.get::<i64>(0))
            .unwrap();
        assert_eq!(rows, vec![1]);
    }

    #[test]
    fn rollback_undoes_changes() {
        let mut conn = conn_with_table();
        {
            let mut tx = conn.transaction().unwrap();
            tx.execute("INSERT INTO t VALUES (1)").unwrap();
            tx.rollback().unwrap();
        }
        let rows: Vec<i64> = conn
            .query_map("SELECT * FROM t", |row| row.get::<i64>(0))
            .unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn drop_without_commit_rolls_back_by_default() {
        let mut conn = conn_with_table();
        {
            let mut tx = conn.transaction().unwrap();
            tx.execute("INSERT INTO t VALUES (1)").unwrap();
            // no commit/rollback -- dropped here
        }
        let rows: Vec<i64> = conn
            .query_map("SELECT * FROM t", |row| row.get::<i64>(0))
            .unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn drop_with_commit_behavior_keeps_changes() {
        let mut conn = conn_with_table();
        {
            let mut tx = conn.transaction().unwrap();
            tx.set_drop_behavior(DropBehavior::Commit);
            tx.execute("INSERT INTO t VALUES (1)").unwrap();
            // no explicit commit -- dropped here, but behavior is Commit
        }
        let rows: Vec<i64> = conn
            .query_map("SELECT * FROM t", |row| row.get::<i64>(0))
            .unwrap();
        assert_eq!(rows, vec![1]);
    }

    #[test]
    fn savepoint_has_a_name() {
        let mut conn = conn_with_table();
        let sp = conn.savepoint_with_name("sp1").unwrap();
        assert_eq!(sp.name(), Some("sp1"));
    }

    #[test]
    fn savepoint_rollback_undoes_changes() {
        let mut conn = conn_with_table();
        {
            let mut sp = conn.savepoint().unwrap();
            sp.execute("INSERT INTO t VALUES (1)").unwrap();
            sp.rollback().unwrap();
        }
        let rows: Vec<i64> = conn
            .query_map("SELECT * FROM t", |row| row.get::<i64>(0))
            .unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn explicit_rollback_fires_rollback_hook() {
        let mut conn = conn_with_table();
        let rolled_back = std::rc::Rc::new(std::cell::RefCell::new(false));
        let rolled_back_clone = std::rc::Rc::clone(&rolled_back);
        conn.rollback_hook(Some(move || {
            *rolled_back_clone.borrow_mut() = true;
        }));

        let mut tx = conn.transaction().unwrap();
        tx.execute("INSERT INTO t VALUES (1)").unwrap();
        tx.rollback().unwrap();

        assert!(*rolled_back.borrow());
    }

    #[test]
    fn drop_triggered_rollback_fires_rollback_hook() {
        let mut conn = conn_with_table();
        let rolled_back = std::rc::Rc::new(std::cell::RefCell::new(false));
        let rolled_back_clone = std::rc::Rc::clone(&rolled_back);
        conn.rollback_hook(Some(move || {
            *rolled_back_clone.borrow_mut() = true;
        }));

        {
            let mut tx = conn.transaction().unwrap();
            tx.execute("INSERT INTO t VALUES (1)").unwrap();
            // no commit/rollback -- dropped here, defaults to Rollback
        }

        assert!(*rolled_back.borrow());
    }

    #[test]
    fn commit_does_not_fire_rollback_hook() {
        let mut conn = conn_with_table();
        let rolled_back = std::rc::Rc::new(std::cell::RefCell::new(false));
        let rolled_back_clone = std::rc::Rc::clone(&rolled_back);
        conn.rollback_hook(Some(move || {
            *rolled_back_clone.borrow_mut() = true;
        }));

        let mut tx = conn.transaction().unwrap();
        tx.execute("INSERT INTO t VALUES (1)").unwrap();
        tx.commit().unwrap();

        assert!(!*rolled_back.borrow());
    }

    #[test]
    fn transaction_state_reflects_open_and_closed_transactions() {
        let mut conn = conn_with_table();
        assert_eq!(
            conn.transaction_state(None).unwrap(),
            TransactionState::None
        );

        let tx = conn.transaction().unwrap();
        assert_eq!(tx.transaction_state(None).unwrap(), TransactionState::Write);
        tx.commit().unwrap();

        assert_eq!(
            conn.transaction_state(Some("main")).unwrap(),
            TransactionState::None
        );
    }

    #[test]
    fn transaction_state_tracks_nested_savepoint_depth() {
        let mut conn = conn_with_table();
        let mut tx = conn.transaction().unwrap();
        assert_eq!(tx.transaction_state(None).unwrap(), TransactionState::Write);

        let sp = tx.savepoint().unwrap();
        assert_eq!(sp.transaction_state(None).unwrap(), TransactionState::Write);
        sp.rollback().unwrap();

        // Still inside the outer transaction after the inner savepoint
        // finished.
        assert_eq!(tx.transaction_state(None).unwrap(), TransactionState::Write);
        tx.rollback().unwrap();
        assert_eq!(
            conn.transaction_state(None).unwrap(),
            TransactionState::None
        );
    }

    #[test]
    fn transaction_state_on_non_main_database_is_an_error() {
        let conn = conn_with_table();
        assert!(conn.transaction_state(Some("other")).is_err());
    }

    #[test]
    fn transaction_behavior_defaults_to_deferred_and_round_trips() {
        let mut conn = conn_with_table();
        assert_eq!(conn.transaction_behavior(), TransactionBehavior::Deferred);
        conn.set_transaction_behavior(TransactionBehavior::Exclusive);
        assert_eq!(conn.transaction_behavior(), TransactionBehavior::Exclusive);
    }

    /// A writable, transaction-aware vtab (issue #95) — an in-memory
    /// integer list that snapshots itself on `begin` and restores on
    /// `rollback`, proving real [`Connection::transaction`]/
    /// [`crate::Transaction`] usage actually notifies it (not just the
    /// vtab-internal unit tests in `vtab.rs`, which call
    /// `begin`/`commit`/`rollback` directly without a real
    /// `Transaction` guard in the loop at all).
    struct ListVTab {
        rows: std::cell::RefCell<Vec<i64>>,
        snapshot: std::cell::RefCell<Option<Vec<i64>>>,
    }

    struct ListCursor {
        rows: Vec<i64>,
        pos: usize,
    }

    impl crate::vtab::VTab for ListVTab {
        type Cursor = ListCursor;

        fn column_names(&self) -> Vec<String> {
            vec!["value".to_string()]
        }

        fn open(&self) -> Result<ListCursor> {
            Ok(ListCursor {
                rows: self.rows.borrow().clone(),
                pos: 0,
            })
        }

        fn insert(&self, row: Vec<crate::value::Value>) -> Result<()> {
            match row.as_slice() {
                [crate::value::Value::Integer(n)] => {
                    self.rows.borrow_mut().push(*n);
                    Ok(())
                }
                other => Err(crate::error::Error::ColumnCountMismatch {
                    expected: 1,
                    actual: other.len(),
                }),
            }
        }

        fn begin(&self) -> Result<()> {
            *self.snapshot.borrow_mut() = Some(self.rows.borrow().clone());
            Ok(())
        }

        fn commit(&self) -> Result<()> {
            *self.snapshot.borrow_mut() = None;
            Ok(())
        }

        fn rollback(&self) -> Result<()> {
            if let Some(snap) = self.snapshot.borrow_mut().take() {
                *self.rows.borrow_mut() = snap;
            }
            Ok(())
        }
    }

    impl crate::vtab::UpdateVTab for ListVTab {}
    impl crate::vtab::TransactionVTab for ListVTab {}

    impl crate::vtab::VTabCursor for ListCursor {
        fn filter(&mut self, _filter: Option<&crate::dml_select::Expr>) -> Result<()> {
            Ok(())
        }

        fn next(&mut self) -> Result<()> {
            self.pos += 1;
            Ok(())
        }

        fn eof(&self) -> bool {
            self.pos >= self.rows.len()
        }

        fn column(&self, ctx: &mut crate::vtab::Context, _i: usize) -> Result<()> {
            ctx.set_result(&self.rows[self.pos])
        }
    }

    fn conn_with_list_vtab(initial: Vec<i64>) -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.create_module(
            "v",
            ListVTab {
                rows: std::cell::RefCell::new(initial),
                snapshot: std::cell::RefCell::new(None),
            },
        )
        .unwrap();
        conn
    }

    #[test]
    fn transaction_rollback_undoes_a_writable_vtab_insert() {
        let mut conn = conn_with_list_vtab(vec![1, 2]);
        {
            let mut tx = conn.transaction().unwrap();
            tx.execute("INSERT INTO v VALUES (3)").unwrap();
            tx.rollback().unwrap();
        }
        let values: Vec<i64> = conn.query_map("SELECT * FROM v", |row| row.get(0)).unwrap();
        assert_eq!(values, vec![1, 2]);
    }

    #[test]
    fn transaction_commit_keeps_a_writable_vtab_insert() {
        let mut conn = conn_with_list_vtab(vec![1]);
        {
            let mut tx = conn.transaction().unwrap();
            tx.execute("INSERT INTO v VALUES (2)").unwrap();
            tx.commit().unwrap();
        }
        let values: Vec<i64> = conn.query_map("SELECT * FROM v", |row| row.get(0)).unwrap();
        assert_eq!(values, vec![1, 2]);
    }

    #[test]
    fn drop_without_commit_rolls_back_a_writable_vtab_insert() {
        let mut conn = conn_with_list_vtab(vec![1]);
        {
            let mut tx = conn.transaction().unwrap();
            tx.execute("INSERT INTO v VALUES (2)").unwrap();
            // no commit/rollback -- dropped here, defaults to Rollback
        }
        let values: Vec<i64> = conn.query_map("SELECT * FROM v", |row| row.get(0)).unwrap();
        assert_eq!(values, vec![1]);
    }
}
