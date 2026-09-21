//! Real MVCC version chains (`ADR-0072` / `MVCC2-FR-001`..`010`,
//! `docs/design/MVCC-PRODUCTION-DESIGN.md`): per-`(record, field)`
//! `(new_value, txn_id)` history, adapted from the phase-1 spike's
//! `(old_value, superseded_by_txn)` shape in
//! `crate::generic_spike::mvcc_spike` — see that module's docs for the
//! guarantees this ports (a snapshot retains its pre-write view; a
//! chain walk finds the correct interval, not just "the previous one";
//! a conflicting commit applies nothing; GC reclaims only what it
//! should and never resurrects reclaimed history), and `ADR-0072`'s
//! "Acceptance and implementation" (rounds seven and nine) for why no
//! before-image is captured and why a txn id is assigned at apply time.
//!
//! # Key granularity: `(RecordId, FieldRef)`, not the whole record
//!
//! A whole-record write (`WriteOp::Insert`/`Replace`) already carries
//! its complete new field list in the operation itself, and a
//! single-field `TransactionOp` (session `Commit`, `UpdateField`) already
//! carries its one new value — so recording one chain entry per field,
//! at the same `txn_id`, needs no read of anything: the new value is
//! already in hand at the call site. Using the same per-field key for
//! every write path keeps one mechanism for all of them (`MVCC2-FR-008`)
//! rather than a whole-record shape that would need to read a record's
//! untouched fields to fill in a complete snapshot.
//!
//! Record existence is tracked the same way, under the reserved
//! [`EXISTENCE_FIELD`] key: an `Insert` writes it `Some(Bool(true))`; a
//! `Delete` writes it `None` — a tombstone, in exactly the sense every
//! other key's "no qualifying entry" already carries. `EXISTENCE_FIELD`
//! is never a real domain field tag — every domain's tags are small and
//! dense starting at `0`.
//!
//! # Txn ids are assigned at apply time, for every write path uniformly
//!
//! `ADR-0072` round nine: assigning a shared counter's next id at
//! journal-append time (inside `journal.rs`'s own mutex, before the
//! turn-gate admits the batch to apply) leaves a real gap before a
//! journaled write actually applies, during which a concurrent
//! non-journaled write — which has no such gap, since it assigns at
//! apply time under the store's own exclusive section — can be assigned
//! a *higher* id while applying *first*, corrupting the id-order/
//! apply-order correspondence a snapshot's read depends on. The fix:
//! every write path calls [`TxnCounter::next`] from *inside* the same
//! `with_exclusive` critical section that already runs `apply_batch`/
//! `apply_prepared`, immediately before folding the write into
//! [`MvccIndex`] — never earlier. This makes a txn id a purely
//! reconstructed quantity, like `last_committed_txn` already is,
//! re-derived by folding log entries in log order at open — so no new
//! insert-log/journal entry kind is needed at all; today's entries carry
//! everything a fold needs already.
//!
//! # Server-layer, not generic-layer
//!
//! Lives under `src/server/`, not `src/generic/`, because it is keyed by
//! [`crate::server::protocol::FieldRef`]/[`ScanValue`] — the wire shape
//! only the server layer has an opinion about. `crate::generic::insert_log`
//! carries no MVCC-specific payload and needs no changes: an adapter
//! folds its own record type into this module's shape (via its existing
//! `fields_of`-equivalent) when reconstructing at open.

use super::protocol::{FieldRef, RecordId, ScanValue};
use crate::durability::sync_parent_dir;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

/// A table-local commit sequence number (`MVCC2-FR-002`).
pub type TxnId = u64;

/// Reserved for a baseline entry seeded at MVCC activation
/// (`MVCC2-FR-001`). Every real, assigned id starts at `1`
/// ([`TxnCounter::next`]), so a baseline entry is always the oldest
/// possible entry in any chain.
pub const BASELINE_TXN: TxnId = 0;

/// The reserved field tag tracking whether a record exists at all, under
/// the same per-key chain every real field uses — never a real domain
/// tag (every domain's are small and dense from `0`).
pub const EXISTENCE_FIELD: FieldRef = FieldRef::MAX;

/// `MVCC2-FR-002`'s shared per-table counter: one `AtomicU64`, seeded at
/// open from reconstructed state, called from inside whichever
/// `with_exclusive` section each write path already runs its apply in —
/// see this module's own doc comment for why apply time, not append
/// time. A plain atomic increment needs no lock of its own: every caller
/// already holds the table's one exclusive section, so calls are already
/// serialized.
#[derive(Debug, Default)]
pub struct TxnCounter(AtomicU64);

impl TxnCounter {
    /// Seed from the highest txn id already observed (`0` if none) — the
    /// next call to [`Self::next`] returns `seeded + 1`.
    pub fn seeded_at(last_committed: TxnId) -> Self {
        Self(AtomicU64::new(last_committed))
    }

    /// Assign and return the next txn id. Real ids start at `1`
    /// ([`BASELINE_TXN`] is reserved for activation baselines).
    pub fn next(&self) -> TxnId {
        self.0.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The highest id assigned so far, without assigning one.
    pub fn current(&self) -> TxnId {
        self.0.load(Ordering::SeqCst)
    }
}

/// A read cannot reconstruct a value after its history was reclaimed
/// (`MVCC2-FR-006`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryReclaimed;

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    txn_id: TxnId,
    /// `None` — a tombstone: this key had no value as of this txn (a
    /// `Delete`'s [`EXISTENCE_FIELD`] entry). Absence and deletion are
    /// the same "no qualifying entry" shape to a reader — see
    /// [`MvccIndex::read`].
    value: Option<ScanValue>,
}

#[derive(Debug, Default)]
struct Chain {
    /// The lowest `txn_id` ever folded into this chain — unlike
    /// `reclaimed_through`, GC never changes this: it is what lets a
    /// snapshot before a key's true creation see absence unconditionally,
    /// even once GC has dropped the entry that was actually written at
    /// this txn. `None` only transiently, inside [`MvccIndex::fold`].
    created_at: Option<TxnId>,
    /// The txn id of the oldest entry [`MvccIndex::gc`] has *kept* for
    /// this chain — every entry it dropped answered some interval below
    /// this id, so a snapshot strictly below it can no longer be
    /// answered (`MVCC2-FR-006`). `0` (no entry ever dropped) never
    /// rejects a snapshot, since every real txn id is `>= 1`.
    reclaimed_through: TxnId,
    /// Ascending by `txn_id`; the last entry is always the chain's
    /// current value.
    entries: Vec<Entry>,
}

/// One table's MVCC version index: per-`(id, field)` chains, read and
/// written by every mutation path once the table is MVCC-active
/// (`MVCC2-FR-008`).
#[derive(Debug, Default)]
pub struct MvccIndex {
    chains: HashMap<(RecordId, FieldRef), Chain>,
    /// The highest txn id ever recorded — `0` if MVCC has never produced
    /// a real (non-baseline) entry on this table.
    last_committed: TxnId,
}

impl MvccIndex {
    pub fn last_committed(&self) -> TxnId {
        self.last_committed
    }

    /// `MVCC2-FR-001`/`MVCC2-FR-003`: fold one historical entry —
    /// idempotent by construction, a repeat of an already-recorded
    /// `(key, txn_id)` is a no-op, so a caller replaying/reconstructing
    /// need not separately track what it already folded
    /// (`MVCC2-FR-010`'s crash-safe double-flush requirement).
    pub fn fold(&mut self, key: (RecordId, FieldRef), txn_id: TxnId, value: Option<ScanValue>) {
        let chain = self.chains.entry(key).or_default();
        if chain.entries.iter().any(|e| e.txn_id == txn_id) {
            return;
        }
        chain.created_at = Some(chain.created_at.map_or(txn_id, |c| c.min(txn_id)));
        let pos = chain.entries.partition_point(|e| e.txn_id < txn_id);
        chain.entries.insert(pos, Entry { txn_id, value });
        self.last_committed = self.last_committed.max(txn_id);
    }

    /// `MVCC2-FR-008`: record one live or replayed write — the caller's
    /// own already-serialized apply already assigned `txn_id`
    /// ([`TxnCounter::next`]); this only updates the in-memory index.
    pub fn record_write(
        &mut self,
        key: (RecordId, FieldRef),
        txn_id: TxnId,
        value: Option<ScanValue>,
    ) {
        self.fold(key, txn_id, value);
    }

    /// `MVCC2-FR-006`: the value visible to a snapshot at `snapshot_txn`
    /// — the newest entry at or below it. `Ok(None)` is "no qualifying
    /// entry": never existed by then, or a tombstone at or before the
    /// snapshot — the two are the same shape to a reader, matching
    /// `MVCC-SPIKE-DESIGN.md`'s own pre-creation-absence case.
    /// [`HistoryReclaimed`] when GC has removed the interval a snapshot
    /// below its boundary would need.
    pub fn read(
        &self,
        key: &(RecordId, FieldRef),
        snapshot_txn: TxnId,
    ) -> Result<Option<ScanValue>, HistoryReclaimed> {
        let Some(chain) = self.chains.get(key) else {
            return Ok(None);
        };
        // A snapshot before this key's true creation sees absence
        // unconditionally — `created_at` never moves under GC, unlike
        // `reclaimed_through`, so this holds even once the entry actually
        // written at `created_at` has itself been dropped.
        match chain.created_at {
            Some(created) if snapshot_txn >= created => {}
            _ => return Ok(None),
        }
        if let Some(entry) = chain
            .entries
            .iter()
            .rev()
            .find(|e| e.txn_id <= snapshot_txn)
        {
            return Ok(entry.value.clone());
        }
        if snapshot_txn < chain.reclaimed_through {
            return Err(HistoryReclaimed);
        }
        Ok(None)
    }

    /// `MVCC2-FR-007`: whether any of `keys` was last written after
    /// `snapshot_txn` — a write-write conflict against a session's own
    /// staged targets.
    pub fn conflicts(
        &self,
        keys: impl IntoIterator<Item = (RecordId, FieldRef)>,
        snapshot_txn: TxnId,
    ) -> bool {
        keys.into_iter().any(|key| {
            self.chains
                .get(&key)
                .and_then(|chain| chain.entries.last())
                .is_some_and(|entry| entry.txn_id > snapshot_txn)
        })
    }

    /// `MVCC2-FR-010` item 5 (GC proper). Given the minimum open-snapshot
    /// txn id (`None` — no snapshot open, reclaim everything below the
    /// table's own current state), drop every chain entry older than the
    /// newest one at or below the boundary, retaining that newest one —
    /// which is always at least the chain's own current (newest overall)
    /// entry, so a live key's current value is never lost.
    ///
    /// Returns how many entries were dropped (`HRC-FR-001`, ADR-0096).
    pub fn gc(&mut self, min_open_snapshot: Option<TxnId>) -> usize {
        let boundary = min_open_snapshot.unwrap_or(TxnId::MAX);
        let mut dropped = 0;
        for chain in self.chains.values_mut() {
            let Some(keep_from) = chain.entries.iter().rposition(|e| e.txn_id <= boundary) else {
                continue;
            };
            if keep_from > 0 {
                // Every dropped entry (indices `0..keep_from`) answered
                // some interval below the *kept* entry's own txn id —
                // that kept entry is what a snapshot now needs and no
                // longer has, so it (not the last dropped entry) is the
                // new rejection boundary.
                chain.reclaimed_through =
                    chain.reclaimed_through.max(chain.entries[keep_from].txn_id);
                chain.entries.drain(0..keep_from);
                dropped += keep_from;
            }
        }
        dropped
    }

    /// Every chain entry currently held, across every key — the size
    /// [`Self::gc`] bounds (`HRC-FR-001`, ADR-0096).
    pub fn history_len(&self) -> usize {
        self.chains.values().map(|chain| chain.entries.len()).sum()
    }

    /// Every `(key, txn_id, value)` triple currently held, for the
    /// persistent history store's own flush at a checkpoint/compact
    /// boundary ([`flush`]/[`load`]).
    fn snapshot(&self) -> Vec<((RecordId, FieldRef), TxnId, Option<ScanValue>)> {
        self.chains
            .iter()
            .flat_map(|(key, chain)| {
                chain
                    .entries
                    .iter()
                    .map(move |e| (*key, e.txn_id, e.value.clone()))
            })
            .collect()
    }
}

/// `MVCC2-FR-005`: the small, in-memory, per-table set of currently open
/// MVCC snapshots (server-process lifetime only, never persisted —
/// `ADR-0071`'s own decision) — refcounted, since more than one session
/// may share a `snapshot_txn`. Used only by [`MvccIndex::gc`]'s own
/// caller to compute the minimum open snapshot.
#[derive(Debug, Default)]
pub struct OpenSnapshots(Mutex<BTreeMap<TxnId, usize>>);

impl OpenSnapshots {
    pub fn register(&self, snapshot_txn: TxnId) {
        *self.0.lock().unwrap().entry(snapshot_txn).or_insert(0) += 1;
    }

    /// `Commit`, `Rollback`, or disconnect — deregisters one registration
    /// of `snapshot_txn`; a no-op if none is registered (defensive: a
    /// session that never actually opened an MVCC snapshot has nothing
    /// to deregister).
    pub fn deregister(&self, snapshot_txn: TxnId) {
        let mut set = self.0.lock().unwrap();
        if let std::collections::btree_map::Entry::Occupied(mut e) = set.entry(snapshot_txn) {
            *e.get_mut() -= 1;
            if *e.get() == 0 {
                e.remove();
            }
        }
    }

    /// The oldest open snapshot, or `None` if none is open.
    pub fn minimum(&self) -> Option<TxnId> {
        self.0.lock().unwrap().keys().next().copied()
    }
}

/// A table's whole MVCC state: whether it has ever gone MVCC-active
/// (`MVCC2-FR-001`), the shared txn counter, the in-memory version
/// index, and the open-snapshot set GC reads. One instance per adapter,
/// constructed at table open by [`MvccState::open`].
#[derive(Debug)]
pub struct MvccState {
    active: AtomicBool,
    counter: TxnCounter,
    index: Mutex<MvccIndex>,
    open_snapshots: OpenSnapshots,
}

/// `<path>.mvcc` — the persistent MVCC history store (`MVCC2-FR-010`):
/// the version index's full current state, plus how many entries of the
/// insert log and the journal were already reflected in it as of this
/// flush (`MVCC2-FR-003`'s "combine the store with whatever the logs
/// still hold" — the two position counters are what tells that
/// combination which log entries are the "still hold" remainder).
/// Written via write-to-temp-then-rename (`STORAGE-014`'s precedent).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PersistedMvcc {
    last_committed: TxnId,
    insert_log_entries_reflected: usize,
    journal_entries_reflected: usize,
    chains: Vec<PersistedChain>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PersistedChain {
    id: RecordId,
    field: FieldRef,
    /// Persisted explicitly, not re-derived from `entries`: GC can drop
    /// the entry actually written at `created_at`, and the pre-creation
    /// guarantee (`MvccIndex::read`) must survive that regardless.
    created_at: TxnId,
    reclaimed_through: TxnId,
    entries: Vec<(TxnId, Option<ScanValue>)>,
}

fn mvcc_store_path(mmap_path: &Path) -> PathBuf {
    let mut path = mmap_path.as_os_str().to_owned();
    path.push(".mvcc");
    PathBuf::from(path)
}

/// What [`MvccState::open`] found on disk, for the adapter's own
/// insert-log/journal fold to skip past (`MVCC2-FR-003`).
pub struct Reconstructed {
    pub state: MvccState,
    pub insert_log_entries_reflected: usize,
    pub journal_entries_reflected: usize,
}

impl MvccState {
    /// `MVCC2-FR-001`/`003`: load `<mmap_path>.mvcc` if it exists — the
    /// durable "has this table ever gone MVCC-active" signal, independent
    /// of insert-log/journal entry presence, since both logs are
    /// periodically reclaimed. A missing file means the table has never
    /// gone MVCC-active: an empty, inactive state, reflected counts of
    /// `0` — the adapter's own fold then covers the whole of both logs,
    /// which is correct only once the table's first `Begin` activates it
    /// and seeds a baseline (`MVCC2-FR-001`); until then the adapter must
    /// not fold anything (a table with no activation history has no
    /// baseline to make a pre-activation record visible).
    ///
    /// # Errors
    ///
    /// Returns [`crate::durability::DurabilityError`] if the file exists
    /// but can't be read or decoded.
    pub fn open(mmap_path: &Path) -> Result<Reconstructed, crate::durability::DurabilityError> {
        let path = mvcc_store_path(mmap_path);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Reconstructed {
                    state: MvccState {
                        active: AtomicBool::new(false),
                        counter: TxnCounter::seeded_at(BASELINE_TXN),
                        index: Mutex::new(MvccIndex::default()),
                        open_snapshots: OpenSnapshots::default(),
                    },
                    insert_log_entries_reflected: 0,
                    journal_entries_reflected: 0,
                })
            }
            Err(e) => return Err(e.into()),
        };
        let persisted: PersistedMvcc = crate::codec::decode(&bytes)?;
        let mut index = MvccIndex::default();
        for chain in persisted.chains {
            let key = (chain.id, chain.field);
            for (txn_id, value) in chain.entries {
                index.fold(key, txn_id, value);
            }
            if let Some(c) = index.chains.get_mut(&key) {
                c.created_at = Some(chain.created_at);
                c.reclaimed_through = chain.reclaimed_through;
            }
        }
        index.last_committed = index.last_committed.max(persisted.last_committed);
        Ok(Reconstructed {
            state: MvccState {
                active: AtomicBool::new(true),
                counter: TxnCounter::seeded_at(index.last_committed),
                index: Mutex::new(index),
                open_snapshots: OpenSnapshots::default(),
            },
            insert_log_entries_reflected: persisted.insert_log_entries_reflected,
            journal_entries_reflected: persisted.journal_entries_reflected,
        })
    }

    /// `MVCC2-FR-010`: durably flush the current index state to
    /// `<mmap_path>.mvcc`, recording how many entries of each log are
    /// reflected as of this flush — called just before the caller clears
    /// the insert log or truncates the journal, from inside the same
    /// lock/gate that already guards that reclamation
    /// (`Compact`/`open`'s insert-log clear, a journal checkpoint).
    ///
    /// # Errors
    ///
    /// Returns [`crate::durability::DurabilityError`] if the temp file
    /// can't be written or the rename fails.
    pub fn flush(
        &self,
        mmap_path: &Path,
        insert_log_entries_reflected: usize,
        journal_entries_reflected: usize,
    ) -> Result<(), crate::durability::DurabilityError> {
        let index = self.index.lock().unwrap();
        let mut by_key: HashMap<(RecordId, FieldRef), PersistedChain> = HashMap::new();
        for (key, txn_id, value) in index.snapshot() {
            let chain = by_key.entry(key).or_insert_with(|| PersistedChain {
                id: key.0,
                field: key.1,
                created_at: 0,
                reclaimed_through: 0,
                entries: Vec::new(),
            });
            chain.entries.push((txn_id, value));
        }
        for (key, chain) in by_key.iter_mut() {
            if let Some(c) = index.chains.get(key) {
                chain.created_at = c.created_at.unwrap_or(0);
                chain.reclaimed_through = c.reclaimed_through;
            }
        }
        let persisted = PersistedMvcc {
            last_committed: index.last_committed,
            insert_log_entries_reflected,
            journal_entries_reflected,
            chains: by_key.into_values().collect(),
        };
        drop(index);
        let encoded = crate::codec::encode(&persisted)?;
        let path = mvcc_store_path(mmap_path);
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)?;
            file.write_all(&encoded)?;
            file.sync_data()?;
        }
        std::fs::rename(&tmp, &path)?;
        sync_parent_dir(&path)?;
        self.active.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// `MVCC2-FR-001`: whether this table has ever gone MVCC-active.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// `MVCC2-FR-001`: mark the table active immediately (before any
    /// baseline scan or flush completes) — necessary so an ordinary
    /// write from a different connection, landing after this `Begin` but
    /// before this session's own first mutation, is still correctly
    /// stamped. Idempotent.
    pub fn activate(&self) {
        self.active.store(true, Ordering::SeqCst);
    }

    pub fn counter(&self) -> &TxnCounter {
        &self.counter
    }

    pub fn open_snapshots(&self) -> &OpenSnapshots {
        &self.open_snapshots
    }

    /// Run `f` with exclusive access to the version index — every write
    /// path's `record_write` call, every read path's `read`/`conflicts`
    /// call, and GC all go through here. Callers on the write path
    /// already hold the table's own `with_exclusive` section; this lock
    /// is a short, uncontended, purely in-memory nesting within it, never
    /// held across I/O (`ADR-0072`'s "in-memory decision, not a
    /// durability step").
    pub fn with_index<T>(&self, f: impl FnOnce(&mut MvccIndex) -> T) -> T {
        f(&mut self.index.lock().unwrap())
    }

    /// `HRC-FR-002` (ADR-0096): reclaim every chain entry no open
    /// snapshot can still need — [`MvccIndex::gc`] at the oldest open
    /// snapshot's txn id (everything below the current state when none
    /// is open). Called by an adapter's `Compact` inside its exclusive
    /// section, before the history flush, so what is flushed is the
    /// reclaimed index. Returns the number of entries dropped; `0` when
    /// MVCC was never activated.
    pub fn reclaim(&self) -> usize {
        if !self.is_active() {
            return 0;
        }
        let boundary = self.open_snapshots.minimum();
        self.with_index(|index| index.gc(boundary))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn id(n: u128) -> RecordId {
        Uuid::from_u128(n)
    }

    fn key(n: u128, field: FieldRef) -> (RecordId, FieldRef) {
        (id(n), field)
    }

    #[test]
    fn snapshot_retains_pre_write_value() {
        let mut index = MvccIndex::default();
        let k = key(1, 0);
        index.record_write(k, 1, Some(ScanValue::I64(3)));
        let old_snapshot = index.last_committed();
        index.record_write(k, 2, Some(ScanValue::I64(4)));
        assert_eq!(index.read(&k, old_snapshot), Ok(Some(ScanValue::I64(3))));
        assert_eq!(
            index.read(&k, index.last_committed()),
            Ok(Some(ScanValue::I64(4)))
        );
    }

    #[test]
    fn chain_walk_finds_middle_interval_not_first_or_latest() {
        let mut index = MvccIndex::default();
        let k = key(1, 0);
        index.record_write(k, 1, Some(ScanValue::I64(3)));
        let first = 1;
        index.record_write(k, 2, Some(ScanValue::I64(4)));
        let middle = 2;
        index.record_write(k, 3, Some(ScanValue::I64(5)));
        index.record_write(k, 4, Some(ScanValue::I64(6)));
        assert_eq!(index.read(&k, first), Ok(Some(ScanValue::I64(3))));
        assert_eq!(index.read(&k, middle), Ok(Some(ScanValue::I64(4))));
    }

    #[test]
    fn conflicts_detects_a_write_after_the_snapshot() {
        let mut index = MvccIndex::default();
        let k = key(1, 0);
        index.record_write(k, 1, Some(ScanValue::I64(3)));
        assert!(!index.conflicts([k], 1));
        index.record_write(k, 2, Some(ScanValue::I64(4)));
        assert!(index.conflicts([k], 1));
        assert!(!index.conflicts([k], 2));
    }

    #[test]
    fn pre_creation_snapshot_sees_absence_even_after_overwrites_and_gc() {
        let mut index = MvccIndex::default();
        let k = key(1, 0);
        let before = index.last_committed();
        assert_eq!(index.read(&k, before), Ok(None));
        index.record_write(k, 1, Some(ScanValue::I64(3)));
        assert_eq!(index.read(&k, before), Ok(None));
        index.record_write(k, 2, Some(ScanValue::I64(4)));
        assert_eq!(index.read(&k, before), Ok(None));
        index.gc(None);
        assert_eq!(index.read(&k, before), Ok(None));
    }

    #[test]
    fn delete_tombstones_the_existence_field() {
        let mut index = MvccIndex::default();
        let k = (id(1), EXISTENCE_FIELD);
        index.record_write(k, 1, Some(ScanValue::Bool(true)));
        let existed = index.last_committed();
        index.record_write(k, 2, None);
        assert_eq!(
            index.read(&k, existed),
            Ok(Some(ScanValue::Bool(true))),
            "still visible before the delete"
        );
        assert_eq!(index.read(&k, index.last_committed()), Ok(None));
    }

    #[test]
    fn gc_preserves_boundary_and_newer_snapshots_reject_reclaimed_interval() {
        let mut index = MvccIndex::default();
        let k = key(1, 0);
        index.record_write(k, 1, Some(ScanValue::I64(3)));
        let stale = 1;
        index.record_write(k, 2, Some(ScanValue::I64(4)));
        let boundary = 2;
        index.record_write(k, 3, Some(ScanValue::I64(5)));
        let newer = 3;
        index.record_write(k, 4, Some(ScanValue::I64(6)));
        index.gc(Some(stale));
        assert_eq!(index.read(&k, stale), Ok(Some(ScanValue::I64(3))));
        index.gc(Some(boundary));
        assert_eq!(index.read(&k, boundary), Ok(Some(ScanValue::I64(4))));
        assert_eq!(index.read(&k, newer), Ok(Some(ScanValue::I64(5))));
        assert_eq!(index.read(&k, stale), Err(HistoryReclaimed));
        index.gc(Some(index.last_committed()));
        assert_eq!(index.read(&k, boundary), Err(HistoryReclaimed));
        assert_eq!(
            index.read(&k, index.last_committed()),
            Ok(Some(ScanValue::I64(6)))
        );
    }

    /// `HRC-FR-001` (ADR-0096): `gc` reports what it dropped and
    /// `history_len` is what remains — nothing below the boundary's
    /// kept entry survives, and with no snapshot open one entry per
    /// chain does.
    #[test]
    fn gc_reports_the_entries_it_dropped_and_history_len_what_remains() {
        let mut index = MvccIndex::default();
        let (a, b) = (key(1, 0), key(2, 0));
        for txn in 1..=5 {
            index.record_write(a, txn, Some(ScanValue::I64(txn as i64)));
        }
        index.record_write(b, 6, Some(ScanValue::I64(60)));
        assert_eq!(index.history_len(), 6);
        assert_eq!(
            index.gc(Some(3)),
            2,
            "txn 1 and 2 are below the kept entry at 3"
        );
        assert_eq!(index.history_len(), 4);
        assert_eq!(index.gc(Some(3)), 0, "idempotent at the same boundary");
        assert_eq!(
            index.gc(None),
            2,
            "no snapshot open: one entry per chain remains"
        );
        assert_eq!(index.history_len(), 2);
        assert_eq!(
            index.read(&a, index.last_committed()),
            Ok(Some(ScanValue::I64(5)))
        );
        assert_eq!(
            index.read(&b, index.last_committed()),
            Ok(Some(ScanValue::I64(60)))
        );
    }

    /// `HRC-FR-002` (ADR-0096): `MvccState::reclaim` is a no-op until
    /// MVCC is activated, then reclaims at the oldest open snapshot.
    #[test]
    fn state_reclaim_honours_activation_and_the_oldest_open_snapshot() {
        let state = MvccState {
            active: AtomicBool::new(false),
            counter: TxnCounter::seeded_at(BASELINE_TXN),
            index: Mutex::new(MvccIndex::default()),
            open_snapshots: OpenSnapshots::default(),
        };
        let k = key(1, 0);
        state.with_index(|index| {
            for txn in 1..=4 {
                index.record_write(k, txn, Some(ScanValue::I64(txn as i64)));
            }
        });
        assert_eq!(state.reclaim(), 0, "not active: nothing reclaimed");
        state.activate();
        state.open_snapshots().register(2);
        assert_eq!(
            state.reclaim(),
            1,
            "only txn 1 is below the open snapshot at 2"
        );
        state.open_snapshots().deregister(2);
        assert_eq!(
            state.reclaim(),
            2,
            "nothing open: only the current entry remains"
        );
        assert_eq!(state.with_index(|index| index.history_len()), 1);
    }

    #[test]
    fn conflicting_batch_applies_nothing_is_the_caller_s_job_but_conflicts_says_so() {
        let mut index = MvccIndex::default();
        let (age, other) = (key(1, 0), key(2, 0));
        index.record_write(age, 1, Some(ScanValue::I64(3)));
        index.record_write(other, 2, Some(ScanValue::I64(8)));
        let stale_snapshot = 2;
        index.record_write(age, 3, Some(ScanValue::I64(4)));
        assert!(index.conflicts([age], stale_snapshot));
        assert!(!index.conflicts([other], stale_snapshot));
    }

    #[test]
    fn open_snapshots_tracks_the_minimum_across_refcounted_registrations() {
        let set = OpenSnapshots::default();
        assert_eq!(set.minimum(), None);
        set.register(5);
        set.register(5);
        set.register(3);
        assert_eq!(set.minimum(), Some(3));
        set.deregister(3);
        assert_eq!(set.minimum(), Some(5));
        set.deregister(5);
        assert_eq!(set.minimum(), Some(5));
        set.deregister(5);
        assert_eq!(set.minimum(), None);
    }

    #[test]
    fn txn_counter_seeds_and_advances_monotonically() {
        let counter = TxnCounter::seeded_at(41);
        assert_eq!(counter.current(), 41);
        assert_eq!(counter.next(), 42);
        assert_eq!(counter.next(), 43);
        assert_eq!(
            TxnCounter::default().next(),
            1,
            "a fresh counter starts real ids at 1"
        );
    }

    #[test]
    fn flush_and_open_round_trip_the_index_and_reflected_counts() {
        let dir = crate::test_support::fresh_temp_dir("mvcc_flush_round_trip").unwrap();
        let mmap_path = dir.join("table.mmap");
        let reconstructed = MvccState::open(&mmap_path).unwrap();
        assert!(!reconstructed.state.is_active());
        assert_eq!(reconstructed.insert_log_entries_reflected, 0);
        let k = key(1, 0);
        reconstructed
            .state
            .with_index(|index| index.record_write(k, 1, Some(ScanValue::I64(7))));
        reconstructed.state.flush(&mmap_path, 3, 2).unwrap();

        let reopened = MvccState::open(&mmap_path).unwrap();
        assert!(reopened.state.is_active());
        assert_eq!(reopened.insert_log_entries_reflected, 3);
        assert_eq!(reopened.journal_entries_reflected, 2);
        assert_eq!(
            reopened.state.with_index(|index| index.read(&k, 1)),
            Ok(Some(ScanValue::I64(7)))
        );
        assert_eq!(reopened.state.counter().current(), 1);
    }
}
