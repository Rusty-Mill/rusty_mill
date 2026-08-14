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
        Ok(Transaction {
            conn,
            snapshot: Some(snapshot),
            drop_behavior: DropBehavior::Rollback,
            finished: false,
        })
    }

    /// Commits: keeps the changes made since the transaction began.
    pub fn commit(mut self) -> Result<()> {
        self.snapshot = None;
        self.finished = true;
        Ok(())
    }

    /// Rolls back to the pre-transaction snapshot.
    pub fn rollback(mut self) -> Result<()> {
        if let Some(snapshot) = self.snapshot.take() {
            self.conn.restore_db(snapshot);
            self.conn.fire_rollback_hook();
        }
        self.finished = true;
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
            DropBehavior::Commit => self.snapshot = None,
            DropBehavior::Rollback => {
                if let Some(snapshot) = self.snapshot.take() {
                    self.conn.restore_db(snapshot);
                    self.conn.fire_rollback_hook();
                }
            }
            DropBehavior::Panic => panic!("Transaction dropped without commit or rollback"),
        }
        self.finished = true;
        Ok(())
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
}
