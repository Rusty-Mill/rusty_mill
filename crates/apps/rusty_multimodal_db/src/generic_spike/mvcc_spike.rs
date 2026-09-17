//! In-memory MVCC correctness spike (ADR-0071 / SPIKE-FR-001..006).
//!
//! The parent module is research-gated. Values are Dog-like `u32` ages,
//! with local UUID/field-tag keys, independent of the server's feature gate.
//! Each nonempty commit advances the counter once; exclusive `&mut self`
//! makes validation and application one single-threaded critical section.
//! Empty commits do not advance it. Staging replaces earlier writes to the
//! same key; reads show committed snapshot state, not staged values.
//!
//! Creation stamps distinguish pre-creation absence from missing history.
//! Per-key reclamation watermarks reject reads whose history GC removed,
//! even when a newer log entry would otherwise look like a valid answer.
//! GC takes a caller-supplied minimum: snapshots below it are not promised
//! retention. There is no session tracking, persistence, or production wiring.

use std::collections::HashMap;
use uuid::Uuid;

/// A table-local commit sequence number.
pub type TxnId = u64;

/// One Dog-like record's mutable field, using a spike-local field tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    /// Record identity.
    pub id: Uuid,
    /// Field identity within the record.
    pub field: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentValue {
    value: u32,
    created_at: TxnId,
    last_write_txn: TxnId,
    reclaimed_through: TxnId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VersionLogEntry {
    old_value: u32,
    superseded_by_txn: TxnId,
}

/// A failed commit; all staged writes remain unapplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitError {
    /// At least one target changed after the snapshot began.
    Conflict,
    /// The monotonic commit counter cannot advance without wrapping.
    TxnIdExhausted,
}

/// A read cannot reconstruct a value after its history was reclaimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryReclaimed;

/// A point in time and local staged writes; use only with its originating table.
#[derive(Debug)]
pub struct Snapshot {
    snapshot_txn: TxnId,
    staged: HashMap<Key, u32>,
}

impl Snapshot {
    /// The last committed transaction when this snapshot was taken.
    pub fn txn_id(&self) -> TxnId {
        self.snapshot_txn
    }
}

/// One in-memory table with primary values and an append-only side log.
/// Only explicit [`Self::gc`] removes retained log entries.
#[derive(Debug, Default)]
pub struct MvccTable {
    next_txn: TxnId,
    current: HashMap<Key, CurrentValue>,
    log: HashMap<Key, Vec<VersionLogEntry>>,
}

impl MvccTable {
    /// Capture the latest committed transaction, without registering a session.
    pub fn begin_snapshot(&self) -> Snapshot {
        Snapshot {
            snapshot_txn: self.next_txn,
            staged: HashMap::new(),
        }
    }

    /// Read committed state at `snap`, distinguishing absence from lost history.
    pub fn read(&self, snap: &Snapshot, key: Key) -> Result<Option<u32>, HistoryReclaimed> {
        let Some(current) = self.current.get(&key) else {
            return Ok(None);
        };
        if snap.snapshot_txn < current.created_at {
            return Ok(None);
        }
        if current.last_write_txn <= snap.snapshot_txn {
            return Ok(Some(current.value));
        }
        if snap.snapshot_txn < current.reclaimed_through {
            return Err(HistoryReclaimed);
        }
        self.log
            .get(&key)
            .into_iter()
            .flatten()
            .find(|entry| entry.superseded_by_txn > snap.snapshot_txn)
            .map(|entry| Some(entry.old_value))
            .ok_or(HistoryReclaimed)
    }

    /// Stage a value locally; neither the primary nor the log is changed.
    pub fn stage(&self, snap: &mut Snapshot, key: Key, value: u32) {
        snap.staged.insert(key, value);
    }

    /// Validate every target before applying any write, then commit one txn id.
    pub fn commit(&mut self, snap: Snapshot) -> Result<(), CommitError> {
        if snap.staged.keys().any(|key| {
            self.current
                .get(key)
                .is_some_and(|current| current.last_write_txn > snap.snapshot_txn)
        }) {
            return Err(CommitError::Conflict);
        }
        if snap.staged.is_empty() {
            return Ok(());
        }
        let txn = self
            .next_txn
            .checked_add(1)
            .ok_or(CommitError::TxnIdExhausted)?;
        for (key, value) in snap.staged {
            if let Some(current) = self.current.get_mut(&key) {
                self.log.entry(key).or_default().push(VersionLogEntry {
                    old_value: current.value,
                    superseded_by_txn: txn,
                });
                current.value = value;
                current.last_write_txn = txn;
            } else {
                self.current.insert(
                    key,
                    CurrentValue {
                        value,
                        created_at: txn,
                        last_write_txn: txn,
                        reclaimed_through: 0,
                    },
                );
            }
        }
        self.next_txn = txn;
        Ok(())
    }

    /// Compact history at/below the supplied minimum, or all history for `None`.
    /// The caller is responsible for supplying the actual oldest open snapshot.
    pub fn gc(&mut self, min_open_snapshot: Option<TxnId>) {
        let boundary = min_open_snapshot.unwrap_or(TxnId::MAX);
        for (key, current) in &mut self.current {
            if let Some(entries) = self.log.get_mut(key) {
                entries.retain(|entry| {
                    if entry.superseded_by_txn > boundary {
                        return true;
                    }
                    current.reclaimed_through =
                        current.reclaimed_through.max(entry.superseded_by_txn);
                    false
                });
            }
        }
        self.log.retain(|_, entries| !entries.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: u128, field: u16) -> Key {
        Key {
            id: Uuid::from_u128(id),
            field,
        }
    }

    fn write(table: &mut MvccTable, key: Key, value: u32) {
        let mut snap = table.begin_snapshot();
        table.stage(&mut snap, key, value);
        table.commit(snap).unwrap();
    }

    #[test]
    fn snapshot_retains_pre_write_value() {
        let mut table = MvccTable::default();
        let age = key(1, 0);
        write(&mut table, age, 3);
        let old = table.begin_snapshot();
        write(&mut table, age, 4);
        assert_eq!(table.read(&old, age), Ok(Some(3)));
        assert_eq!(table.read(&table.begin_snapshot(), age), Ok(Some(4)));
        assert_eq!(table.log[&age][0].old_value, 3);
        assert_eq!(table.log[&age][0].superseded_by_txn, 2);
    }

    #[test]
    fn chain_walk_finds_middle_interval_not_first_or_latest() {
        let mut table = MvccTable::default();
        let age = key(1, 0);
        write(&mut table, age, 3);
        let first = table.begin_snapshot();
        write(&mut table, age, 4);
        let middle = table.begin_snapshot();
        write(&mut table, age, 5);
        write(&mut table, age, 6);
        assert_eq!(table.read(&first, age), Ok(Some(3)));
        assert_eq!(table.read(&middle, age), Ok(Some(4)));
    }

    #[test]
    fn conflicting_batch_applies_nothing_including_uncontested_keys() {
        let mut table = MvccTable::default();
        let age = key(1, 0);
        let other = key(2, 0);
        write(&mut table, age, 3);
        write(&mut table, other, 8);
        let mut stale = table.begin_snapshot();
        table.stage(&mut stale, age, 20);
        table.stage(&mut stale, other, 21);
        table.stage(&mut stale, key(3, 0), 22);
        write(&mut table, age, 4);
        let before = (table.next_txn, table.current.clone(), table.log.clone());
        assert_eq!(table.commit(stale), Err(CommitError::Conflict));
        assert_eq!(
            before,
            (table.next_txn, table.current.clone(), table.log.clone())
        );
        let now = table.begin_snapshot();
        assert_eq!(table.read(&now, age), Ok(Some(4)));
        assert_eq!(table.read(&now, other), Ok(Some(8)));
        assert_eq!(table.read(&now, key(3, 0)), Ok(None));
    }

    #[test]
    fn uncontested_batch_commits_once_despite_other_field_write() {
        let mut table = MvccTable::default();
        let age = key(1, 0);
        write(&mut table, age, 3);
        let mut snap = table.begin_snapshot();
        table.stage(&mut snap, age, 4);
        table.stage(&mut snap, age, 5);
        table.stage(&mut snap, key(2, 0), 8);
        assert_eq!(table.read(&snap, age), Ok(Some(3)));
        assert_eq!(table.next_txn, 1);
        write(&mut table, key(1, 1), 99);
        table.commit(snap).unwrap();
        assert_eq!(table.next_txn, 3);
        assert_eq!(table.current[&age].last_write_txn, 3);
        assert_eq!(table.current[&key(2, 0)].last_write_txn, 3);
        let now = table.begin_snapshot();
        assert_eq!(table.read(&now, age), Ok(Some(5)));
        assert_eq!(table.read(&now, key(2, 0)), Ok(Some(8)));
        assert_eq!(table.read(&now, key(1, 1)), Ok(Some(99)));
    }

    #[test]
    fn pre_creation_snapshot_sees_absence_even_after_overwrites_and_gc() {
        let mut table = MvccTable::default();
        let age = key(1, 0);
        let before = table.begin_snapshot();
        assert_eq!(table.read(&before, age), Ok(None));
        write(&mut table, age, 3);
        assert!(table.log.is_empty());
        assert_eq!(table.read(&before, age), Ok(None));
        write(&mut table, age, 4);
        assert_eq!(table.read(&before, age), Ok(None));
        table.gc(None);
        assert_eq!(table.read(&before, age), Ok(None));
    }

    #[test]
    fn concurrent_creation_conflicts() {
        let mut table = MvccTable::default();
        let age = key(1, 0);
        let mut before = table.begin_snapshot();
        table.stage(&mut before, age, 9);
        write(&mut table, age, 3);
        assert_eq!(table.commit(before), Err(CommitError::Conflict));
        assert_eq!(table.read(&table.begin_snapshot(), age), Ok(Some(3)));
    }

    #[test]
    fn gc_preserves_boundary_and_newer_snapshots_rejects_reclaimed_interval() {
        let mut table = MvccTable::default();
        let age = key(1, 0);
        write(&mut table, age, 3);
        let stale = table.begin_snapshot();
        write(&mut table, age, 4);
        let boundary = table.begin_snapshot();
        write(&mut table, age, 5);
        let newer = table.begin_snapshot();
        write(&mut table, age, 6);
        table.gc(Some(stale.txn_id()));
        assert_eq!(table.log[&age].len(), 3);
        assert_eq!(table.read(&stale, age), Ok(Some(3)));
        table.gc(Some(boundary.txn_id()));
        assert_eq!(
            table.log[&age]
                .iter()
                .map(|e| (e.old_value, e.superseded_by_txn))
                .collect::<Vec<_>>(),
            vec![(4, 3), (5, 4)]
        );
        assert_eq!(table.read(&boundary, age), Ok(Some(4)));
        assert_eq!(table.read(&newer, age), Ok(Some(5)));
        // A later retained entry must not masquerade as the missing old value.
        assert_eq!(table.read(&stale, age), Err(HistoryReclaimed));
        table.gc(Some(table.next_txn));
        assert!(table.log.is_empty());
        assert_eq!(table.read(&boundary, age), Err(HistoryReclaimed));
        assert_eq!(table.read(&table.begin_snapshot(), age), Ok(Some(6)));
    }

    #[test]
    fn gc_none_reclaims_all_keys_and_repeated_gc_never_restores_history() {
        let mut table = MvccTable::default();
        let a = key(1, 0);
        let b = key(2, 0);
        write(&mut table, a, 3);
        write(&mut table, b, 8);
        let old = table.begin_snapshot();
        write(&mut table, a, 4);
        write(&mut table, b, 9);
        table.gc(None);
        assert!(table.log.is_empty());
        table.gc(Some(0));
        write(&mut table, a, 5);
        assert_eq!(table.read(&old, a), Err(HistoryReclaimed));
        assert_eq!(table.read(&old, b), Err(HistoryReclaimed));
        assert_eq!(table.read(&table.begin_snapshot(), a), Ok(Some(5)));
    }

    #[test]
    fn empty_commit_and_counter_exhaustion_do_not_mutate_state() {
        let mut table = MvccTable::default();
        table.commit(table.begin_snapshot()).unwrap();
        assert_eq!(table.next_txn, 0);
        let age = key(1, 0);
        write(&mut table, age, 3);
        table.next_txn = TxnId::MAX;
        let mut snap = table.begin_snapshot();
        table.stage(&mut snap, age, 4);
        table.stage(&mut snap, key(2, 0), 8);
        assert_eq!(table.commit(snap), Err(CommitError::TxnIdExhausted));
        assert_eq!(table.next_txn, TxnId::MAX);
        assert_eq!(table.current.len(), 1);
        assert_eq!(table.current[&age].value, 3);
        assert!(table.log.is_empty());
    }
}
