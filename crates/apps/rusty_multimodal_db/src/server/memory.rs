//! [`ConnectionStore`] adapter wrapping
//! [`crate::generic::production::GenericProductionStore<MemoryProductionStack>`]
//! for `Memory` — this crate's sixth domain and third front-door one
//! (`MEM-FR-006`, ADR-0048, `docs/design/SERVER-MEMORY-DOMAIN-DESIGN.md`),
//! `server`-gated alone like `reminder`/`entity`. Eleven fields:
//! `category` is equality-filterable, `access_count` scannable and
//! updatable (non-negative — the one domain rule), everything else
//! refused by `UpdateField` and changed only whole through
//! `Request::Replace` (`REP-FR-005`, ADR-0049), reachable through
//! `Query`/`Aggregate` like any field. `tags` is a `StrList`. One
//! relation since `ADR-0050` (`TBL-FR-008`): `mentions`, whose rows are
//! the `entity` table's — `describe_relations` says so, a same-table
//! `Join` over it is `Unsupported`, and a `Link` under it has its far
//! end checked by the server against that table (`TBL-FR-007`); no
//! `ChildOf` relation.
//!
//! See `crate::generic::memory`'s own module docs for what the
//! consumer's table holds that this record does not, and why.

use super::journal::{
    CheckpointFlush, CommitError, CommitGroup, JournalError, JournalStats, JournaledBatch,
    ReplayedBatch,
};
use super::mvcc::{self, MvccIndex, MvccState, TxnId};
use super::protocol::{
    DomainSchema, ErrorCode, FieldCapabilities, FieldDescriptor, FieldRef, JoinRelation,
    ParentLookup, Predicate, RecordId, RelationCapabilities, RelationDescriptor, ScanValue,
    TransactionOp, ValueKind, WriteOp, WriteResult,
};
use super::{
    bounded_filtered_page, bounded_walk_applies, copy_table_files, filtered_page_by_candidates,
    page_by_scan, page_by_scan_desc, page_key, predicate_matches, read_table_files,
    uuid_pair_bounds, validate_predicate, BackupReport, ConnectionStore, DeleteOutcome,
    InsertOutcome, KeyStats, LinkOutcome, PageRow, ReadTableFilesError, ReplaceIfOutcome,
    ReplaceOutcome,
};
use crate::durability::DurabilityError;
use crate::generic::insert_log::{self, LogEntry};
use crate::generic::memory::{
    open_memory_production_stack_portable, AccessCountField, CategoryField, Memory,
    MemoryProductionStack, UpdatedAtOrder, MEMORY_FOREIGN_TABLE, MEMORY_RELATION_LABELS,
};
use crate::generic::production::GenericProductionStore;
use crate::generic::query::{AllIds, Delete, GetById, Insert, MultiLink, Replace, UpdateField};
use crate::generic::traits::SchemaTag;
use crate::generic::{DeleteError, InsertError, LinkError, ReplaceError};
use std::ops::Bound;
use std::path::{Path, PathBuf};

pub const FIELD_CONTENT: FieldRef = 0;
pub const FIELD_CATEGORY: FieldRef = 1;
pub const FIELD_TAGS: FieldRef = 2;
pub const FIELD_SOURCE: FieldRef = 3;
pub const FIELD_METADATA_JSON: FieldRef = 4;
pub const FIELD_CREATED_AT: FieldRef = 5;
pub const FIELD_UPDATED_AT: FieldRef = 6;
pub const FIELD_MEMORY_TYPE: FieldRef = 7;
pub const FIELD_STATUS: FieldRef = 8;
pub const FIELD_SENSITIVE: FieldRef = 9;
pub const FIELD_ACCESS_COUNT: FieldRef = 10;
/// `SYN-FR-001` (ADR-0056): the soft-delete stamp; `0` is live.
pub const FIELD_DELETED_AT: FieldRef = 11;
/// `SYN-FR-001` (ADR-0056): the writing node; `""` is unattributed.
pub const FIELD_NODE_ID: FieldRef = 12;

/// Every field but `access_count`: refused by `UpdateField`
/// (`MEM-FR-004`) — changed only whole, with every other field, through
/// `replace_record` (`REP-FR-005`, ADR-0049).
const READ_ONLY_FIELDS: [FieldRef; 12] = [
    FIELD_CONTENT,
    FIELD_CATEGORY,
    FIELD_TAGS,
    FIELD_SOURCE,
    FIELD_METADATA_JSON,
    FIELD_CREATED_AT,
    FIELD_UPDATED_AT,
    FIELD_MEMORY_TYPE,
    FIELD_STATUS,
    FIELD_SENSITIVE,
    FIELD_DELETED_AT,
    FIELD_NODE_ID,
];

/// `MEM-FR-003`: `access_count` is a counter — a negative value is
/// `Malformed` before any write, the one domain rule this adapter adds.
fn valid_access_count(value: i64) -> bool {
    value >= 0
}

/// `ADR-0072`'s `MVCC2-FR-001`/`010`: an active table's MVCC state plus
/// the `mmap_path` [`MvccState::flush`] needs to name `<mmap_path>.mvcc`.
struct MvccHandle {
    state: MvccState,
    mmap_path: PathBuf,
}

/// One write an atomic batch made, for the MVCC record: `None` fields is
/// a deletion.
type RecordedWrite = (RecordId, Option<Vec<(FieldRef, ScanValue)>>);

pub struct MemoryConnectionStore {
    store: GenericProductionStore<MemoryProductionStack>,
    /// `JRN-FR-001` (ADR-0025) — see `DogConnectionStore::with_journal`.
    journal: Option<CommitGroup>,
    /// `BAK-FR-002` (ADR-0065) — see `DogConnectionStore::with_backup_source`.
    backup_source: Option<PathBuf>,
    /// `SYU-FR-001` (ADR-0097): when set, every in-place field write that
    /// no journal covers is `msync`ed (the stack's `Flush`) before it is
    /// acknowledged; unset, the acknowledgement precedes durability by
    /// up to the OS's own write-back — the documented loss window.
    sync_updates: bool,
    /// `JUF-FR-001` (ADR-0107): `with_journaled_updates` — in-place
    /// updates committed through the journal when there is one.
    journal_updates: bool,
    /// `JMR-FR-001` (ADR-0113) / `JMC-FR-001` (ADR-0114): what
    /// `with_journal`'s replay applied, in journal order, kept until the
    /// MVCC index is attached and folds it — the journal is truncated by
    /// then, so this is the replay's only record.
    replayed: Vec<ReplayedBatch>,
    /// `ART-FR-002` (ADR-0105): the automatic reclaim threshold, kept
    /// here so `with_mvcc` can apply it whichever order the builders run.
    mvcc_reclaim_every: Option<usize>,
    /// `ADR-0072`'s `MVCC2-FR-001`: `None` — the same "unset by default,
    /// opt in with the real path" shape `backup_source` already
    /// establishes — until [`Self::with_mvcc`] is called; `mvcc_supported`
    /// answers `false` until then, `Unsupported` for
    /// `SESSION_MVCC_ISOLATION`, matching `backup`/`fetch_snapshot`'s own
    /// precedent for `backup_source`.
    mvcc: Option<MvccHandle>,
}

impl MemoryConnectionStore {
    pub fn new(store: GenericProductionStore<MemoryProductionStack>) -> Self {
        Self {
            store,
            journal: None,
            backup_source: None,
            sync_updates: false,
            journal_updates: false,
            replayed: Vec::new(),
            mvcc_reclaim_every: None,
            mvcc: None,
        }
    }

    /// `BAK-FR-002` (ADR-0065) — see `DogConnectionStore::with_backup_source`.
    pub fn with_backup_source(mut self, path: PathBuf) -> Self {
        self.backup_source = Some(path);
        self
    }

    /// `SYU-FR-001` (ADR-0097): acknowledge an in-place field update —
    /// `UpdateField`, and a `Transaction` batch on a table with no
    /// journal — only after `msync` has forced the slot to disk. Opt-in;
    /// unset, the update sits in the page cache until `Flush`, a
    /// checkpoint, or the OS's write-back, as every version before.
    /// Journaled batches are unaffected: the redo entry is already
    /// `fsync`ed before the first slot write.
    pub fn with_synced_updates(mut self, enabled: bool) -> Self {
        self.sync_updates = enabled;
        self
    }

    /// `ART-FR-002` (ADR-0105): reclaim MVCC history automatically once
    /// `every` entries have been appended since the last reclaim
    /// (`None` — never: `Compact` only, the `ADR-0096` behaviour).
    /// Applies to the MVCC state this adapter holds now or activates
    /// later; a no-op on a table that never goes MVCC-active.
    pub fn with_mvcc_reclaim_every(mut self, every: Option<usize>) -> Self {
        self.mvcc_reclaim_every = every;
        if let Some(mvcc) = &self.mvcc {
            mvcc.state.set_reclaim_every(every);
        }
        self
    }

    /// `SYU-FR-002`: the `msync` `with_synced_updates` asks for, taken
    /// under the store's write lock; a failure withholds the
    /// acknowledgement as `Storage`.
    fn sync_update_ack(&self) -> Result<(), ErrorCode> {
        if !self.sync_updates {
            return Ok(());
        }
        self.store.flush().map_err(|_| ErrorCode::Storage)
    }

    /// `JUF-FR-001` (ADR-0107): opt in to committing every in-place
    /// field update through the journal as a one-operation batch — the
    /// redo entry `fsync`ed (group-committed with any concurrent batch)
    /// before the slot is written, no `msync` of the mapping, replayed
    /// at the next open like any journaled batch. Durable past a power
    /// loss like `with_synced_updates`, at a different cost: more for a
    /// single writer (the entry and the slot), less per update under
    /// concurrent writers (one `fsync` covers a group) — `RESULTS.md`.
    /// A no-op on an adapter with no journal.
    pub fn with_journaled_updates(mut self, enabled: bool) -> Self {
        self.journal_updates = enabled;
        self
    }

    /// `JUF-FR-001` (ADR-0107): the journaled commit of one in-place
    /// update — `Ok(false)` for a missing record, `update_field`'s own
    /// contract; `Journal` when the entry could not be made durable,
    /// nothing applied.
    fn journaled_update(&self, op: TransactionOp) -> Result<bool, ErrorCode> {
        match self.apply_transaction(std::slice::from_ref(&op), &[]) {
            Ok(()) => Ok(true),
            Err((_, ErrorCode::RecordNotFound)) => Ok(false),
            Err((_, code)) => Err(code),
        }
    }

    /// `JUF-FR-001`: whether `update_field` goes through the journal —
    /// only when there is one and the operator asked; otherwise the
    /// in-place write (and `msync`, if asked) as before.
    fn journals_updates(&self) -> bool {
        self.journal.is_some() && self.journal_updates
    }

    /// `ADR-0072`'s `MVCC2-FR-001`/`003`: enable real MVCC for this table,
    /// reconstructing `<mmap_path>.mvcc` (if any) and folding whatever the
    /// insert log still holds since that flush.
    ///
    /// **Known gap, honestly named rather than silently accepted, and
    /// deeper than a first look suggests**: `crate::generic::mmap_store::
    /// GenericMmapStore::open` already calls `insert_log::clear`
    /// *unconditionally*, at the end of its own reconciliation pass —
    /// confirmed by reading it directly, not assumed — which runs
    /// *before* this method (or any server-layer code) ever gets a
    /// chance to see the log's contents. This means: for a table with
    /// **any** pending insert-log entries at the moment of a reopen
    /// (crash or ordinary restart), those entries' history is gone
    /// before `with_mvcc` runs at all, regardless of whether it's chained
    /// after [`Self::with_journal`] or called directly — this method's
    /// own "fold whatever the insert log still holds" logic is only
    /// actually exercised by a *fresh* table (nothing to fold yet, so
    /// `open()`'s clear is a no-op) or one whose insert log was already
    /// empty at the last clean shutdown. Chaining after
    /// [`Self::with_journal`] adds an *identical* gap for journaled
    /// batches (that constructor also unconditionally replays-then-
    /// truncates before this runs). **Closing this for real needs a hook
    /// threaded through `GenericMmapStore::open` itself** (`MVCC2-FR-010`
    /// item 3's own plan) — a generic-layer change shared by every domain
    /// in this crate, deliberately not attempted in this round without
    /// its own dedicated review, given the risk of touching that
    /// crash-recovery-critical function. **What already works despite
    /// this**: every write made *after* a table opens is fully covered
    /// (`MVCC2-FR-008`), and `Compact`'s own flush (`Self::compact`)
    /// correctly persists history before *its* insert-log clear, since
    /// that happens at runtime, on an already-constructed adapter that
    /// controls the ordering — this gap is specific to the *open/reopen*
    /// path, not compaction.
    ///
    /// # Errors
    ///
    /// Returns [`DurabilityError`] if `<mmap_path>.mvcc` exists but can't
    /// be read/decoded, or if the insert log can't be read.
    pub fn with_mvcc(self, mmap_path: &Path) -> Result<Self, DurabilityError> {
        let log = insert_log::log_path(mmap_path);
        let entries = insert_log::read_entries(&log, Memory::SCHEMA_TAG)?;
        self.attach_mvcc(mmap_path, entries)
    }

    /// `JMC-FR-002` (ADR-0114): the journaled reopen — [`Self::open_with_mvcc`]'s
    /// read of the pending insert log before the reopen clears it,
    /// [`Self::with_journal`]'s replay, then the index attached with both
    /// folded in: the pending entries first (the older), the replayed
    /// batches after, in journal order, and the history flushed. The one
    /// constructor that composes the crash-atomic journal with real MVCC
    /// for an existing table; `memory_server` takes it when
    /// `SERVER_TXN_JOURNAL_PATH` and `SERVER_MVCC_ISOLATION` are both set.
    ///
    /// # Errors
    ///
    /// [`JournalError`] from the journal's own open or replay, or wrapping
    /// the [`DurabilityError`] of the pending-log read, the reopen, or the
    /// history's reconstruction and flush.
    pub fn open_with_mvcc_journaled(
        path: &Path,
        journal_path: &Path,
    ) -> Result<Self, JournalError> {
        let log = insert_log::log_path(path);
        let pending_entries = insert_log::read_entries(&log, Memory::SCHEMA_TAG)?;
        Self::persist_pending_history(path, pending_entries)?;
        let stack = open_memory_production_stack_portable(path)?;
        // `RGM-FR-009`: replay without truncating, fold and flush, then
        // truncate — the journal outlives the index's record of it.
        let adapter =
            Self::with_journal_inner(GenericProductionStore::new(stack), journal_path, false)?
                .attach_mvcc(path, Vec::new())?;
        if let Some(journal) = &adapter.journal {
            journal.truncate()?;
        }
        Ok(adapter)
    }

    /// `RGM-FR-009` (ADR-0121): before the reopen clears the insert log,
    /// fold its pending entries into the reconstructed index and flush
    /// the history, so the entries have a durable home *before* their
    /// only other one is destroyed. Nothing for an inactive index
    /// (`RGF-FR-001`) or an empty log.
    fn persist_pending_history(
        mmap_path: &Path,
        pending: Vec<LogEntry<Memory, RecordId>>,
    ) -> Result<(), DurabilityError> {
        let total = pending.len();
        let reconstructed = MvccState::open(mmap_path)?;
        if !reconstructed.state.is_active() || total == 0 {
            return Ok(());
        }
        if Self::fold_pending_log_entries(&reconstructed, pending) == 0 {
            return Ok(());
        }
        reconstructed.state.flush(mmap_path, total, 0)
    }

    /// The shared tail of [`Self::with_mvcc`], [`Self::open_with_mvcc`] and
    /// [`Self::open_with_mvcc_journaled`]: reconstruct the index, fold
    /// `pending` (skipping what the last flush already reflects), fold
    /// what `with_journal` replayed (`JMR-FR-001`, ADR-0113: one fresh
    /// txn per batch, in journal order, no reclaim trigger), and flush the
    /// history when anything was folded, so the next open depends on
    /// neither a log the reopen cleared nor a journal the replay
    /// truncated. An inactive index folds no replay: its first `Begin`
    /// seeds the baseline from the live store, which holds it already.
    fn attach_mvcc(
        mut self,
        mmap_path: &Path,
        pending: Vec<LogEntry<Memory, RecordId>>,
    ) -> Result<Self, DurabilityError> {
        let reconstructed = MvccState::open(mmap_path)?;
        // `RGF-FR-001` (ADR-0120): an inactive index folds nothing — its
        // first `Begin` seeds the baseline from the live store, which
        // already holds the log; a fold here would sit *above* that
        // baseline and a fresh snapshot would read the older value.
        let folded_pending = if reconstructed.state.is_active() {
            Self::fold_pending_log_entries(&reconstructed, pending)
        } else {
            0
        };
        reconstructed
            .state
            .set_reclaim_every(self.mvcc_reclaim_every);
        let replayed = std::mem::take(&mut self.replayed);
        let active = reconstructed.state.is_active();
        let fold_replayed = active && !replayed.is_empty();
        self.mvcc = Some(MvccHandle {
            state: reconstructed.state,
            mmap_path: mmap_path.to_path_buf(),
        });
        if fold_replayed {
            for batch in &replayed {
                match batch {
                    ReplayedBatch::Transaction(ops) => self.mvcc_record_transaction_quiet(ops),
                    ReplayedBatch::Write { ops, results } => {
                        self.mvcc_record_write_ops_quiet(ops, results)
                    }
                }
            }
        }
        // `RGM-FR-009`: an active index is flushed at every open — the
        // reopen just reset the insert log, so the recorded "entries
        // reflected" must be reset with it, fold or no fold.
        if active || folded_pending > 0 || fold_replayed {
            let journal_count = self
                .journal
                .as_ref()
                .map(CommitGroup::entries_since_checkpoint)
                .unwrap_or(0);
            if !self.mvcc_flush_now(journal_count) {
                return Err(DurabilityError::Io(std::io::Error::other(
                    "flushing the MVCC history after folding the pending log and the replayed journal at open",
                )));
            }
        }
        Ok(self)
    }

    /// `docs/design/MVCC-OPEN-HOOK-PROPOSAL.md` (Accepted, option (b)):
    /// the *reopen* counterpart to [`Self::with_mvcc`]. `with_mvcc`
    /// alone is wrong for a table reopened through the normal portable-
    /// open path, because `GenericMmapStore::open` (inside
    /// [`open_memory_production_stack_portable`]) unconditionally clears
    /// the insert log as part of its own primary-state fold — by the
    /// time `with_mvcc` could run afterward and read that log itself, any
    /// pending entries it needed are already gone. This constructor reads
    /// those entries *before* the reopen call, then feeds them into the
    /// same folding logic `with_mvcc` uses, closing that gap without any
    /// change to `GenericMmapStore::open` or to any non-MVCC domain.
    ///
    /// # Errors
    ///
    /// Returns [`DurabilityError`] if `<mmap_path>.mvcc` exists but can't
    /// be read/decoded, if the insert log can't be read, or if the
    /// underlying reopen (`open_memory_production_stack_portable`) fails.
    pub fn open_with_mvcc(path: &Path) -> Result<Self, DurabilityError> {
        // Read the pending log first — before the reopen below clears it.
        let log = insert_log::log_path(path);
        let pending_entries = insert_log::read_entries(&log, Memory::SCHEMA_TAG)?;
        Self::persist_pending_history(path, pending_entries)?;
        let stack = open_memory_production_stack_portable(path)?;
        Self::new(GenericProductionStore::new(stack)).attach_mvcc(path, Vec::new())
    }

    /// Shared by [`Self::with_mvcc`] and [`Self::open_with_mvcc`]: fold
    /// whatever insert-log entries weren't yet reflected in the
    /// persisted `.mvcc` store (`reconstructed.insert_log_entries_reflected`
    /// entries are skipped — already accounted for) into the
    /// reconstructed index, one freshly assigned txn id per entry.
    fn fold_pending_log_entries(
        reconstructed: &mvcc::Reconstructed,
        entries: Vec<LogEntry<Memory, RecordId>>,
    ) -> usize {
        let entries = entries
            .into_iter()
            .skip(reconstructed.insert_log_entries_reflected);
        let mut folded = 0;
        for entry in entries {
            folded += 1;
            let txn_id = reconstructed.state.counter().next();
            reconstructed.state.with_index_quiet(|index| match entry {
                LogEntry::Item(record) => {
                    let id = record.id;
                    index.record_write(
                        (id, mvcc::EXISTENCE_FIELD),
                        txn_id,
                        Some(ScanValue::Bool(true)),
                    );
                    for (tag, value) in Self::fields_of(record) {
                        index.record_write((id, tag), txn_id, Some(value));
                    }
                }
                LogEntry::Tombstone(id) => {
                    index.record_write((id, mvcc::EXISTENCE_FIELD), txn_id, None);
                }
            });
        }
        folded
    }

    /// The crash-atomic variant — see `DogConnectionStore::with_journal`
    /// for the contract; identical here.
    pub fn with_journal(
        store: GenericProductionStore<MemoryProductionStack>,
        journal_path: &Path,
    ) -> Result<Self, JournalError> {
        Self::with_journal_inner(store, journal_path, true)
    }

    /// `RGM-FR-009` (ADR-0121): [`Self::with_journal`] with the truncate
    /// held back — for [`Self::open_with_mvcc_journaled`], which folds
    /// the replay into the version index and flushes it *before* the
    /// journal is dropped, so a crash between the two replays again
    /// instead of losing the batches from the index.
    fn with_journal_inner(
        store: GenericProductionStore<MemoryProductionStack>,
        journal_path: &Path,
        truncate: bool,
    ) -> Result<Self, JournalError> {
        let (journal, batches) = CommitGroup::open(journal_path)?;
        let mut replayed = Vec::new();
        store.with_exclusive(|inner| -> Result<(), JournalError> {
            let schema = Self::schema();
            for (batch_index, batch) in batches.iter().enumerate() {
                match batch {
                    JournaledBatch::Transaction(ops) => {
                        // `RVL-FR-004` (ADR-0112): a record deleted after the
                        // batch was journaled (its tombstone folded from the
                        // insert log before this replay) makes the op moot,
                        // not the journal corrupt — skip it, replay the rest.
                        let mut applied = Vec::with_capacity(ops.len());
                        for (index, op) in ops.iter().enumerate() {
                            match Self::apply_batch(inner, std::slice::from_ref(op)) {
                                Ok(()) => applied.push(op.clone()),
                                Err((_, ErrorCode::RecordNotFound)) => {}
                                Err((_, code)) => {
                                    return Err(JournalError::Replay {
                                        batch: batch_index,
                                        index,
                                        code,
                                    })
                                }
                            }
                        }
                        if !applied.is_empty() {
                            replayed.push(ReplayedBatch::Transaction(applied));
                        }
                    }
                    JournaledBatch::Write(ops) => {
                        let results = Self::replay_write_batch(inner, &schema, ops).map_err(
                            |(index, code)| JournalError::Replay {
                                batch: batch_index,
                                index,
                                code,
                            },
                        )?;
                        replayed.push(ReplayedBatch::Write {
                            ops: ops.clone(),
                            results,
                        });
                    }
                }
            }
            inner.checkpoint_flush()?;
            if truncate {
                journal.truncate()
            } else {
                Ok(())
            }
        })?;
        Ok(Self {
            store,
            journal: Some(journal),
            backup_source: None,
            sync_updates: false,
            journal_updates: false,
            replayed,
            mvcc_reclaim_every: None,
            mvcc: None,
        })
    }

    /// `WBJ-FR-003` (ADR-0063): replay a journaled, already-validated
    /// `WriteOp` batch — the identical `prepare_write`/`apply_prepared`
    /// pair the live atomic path uses, in order, under the same
    /// exclusive section `with_journal`'s replay already holds. Safe to
    /// re-run a `ReplaceIf`'s guard here: a crash before this point means
    /// the store is exactly the state the guard was (or would have been)
    /// evaluated against originally, so replay reconstructs the
    /// identical decision. A prepare or apply failure here is a real
    /// anomaly (the crate's own crash-atomicity invariant broken, not an
    /// expected outcome) and aborts replay via `JournalError::Replay`
    /// rather than being silently skipped.
    fn replay_write_batch(
        inner: &mut MemoryProductionStack,
        schema: &DomainSchema,
        ops: &[WriteOp],
    ) -> Result<Vec<WriteResult>, (usize, ErrorCode)> {
        let mut results = Vec::with_capacity(ops.len());
        for (i, op) in ops.iter().enumerate() {
            let prepared = Self::prepare_write(schema, op).map_err(|code| (i, code))?;
            results.push(Self::apply_prepared(inner, prepared).map_err(|code| (i, code))?);
        }
        Ok(results)
    }

    /// This domain's `DomainSchema`, without an instance — needed at
    /// `with_journal`'s replay, before `Self` exists yet.
    /// `ConnectionStore::describe` delegates here.
    fn schema() -> DomainSchema {
        let read_only = FieldCapabilities {
            filter_eq: false,
            scan: false,
            update: false,
        };
        let field = |tag: FieldRef, name: &str, value_kind: ValueKind| FieldDescriptor {
            tag,
            name: name.into(),
            value_kind,
            capabilities: read_only,
        };
        DomainSchema {
            fields: vec![
                field(FIELD_CONTENT, "content", ValueKind::Str),
                FieldDescriptor {
                    tag: FIELD_CATEGORY,
                    name: "category".into(),
                    value_kind: ValueKind::Str,
                    capabilities: FieldCapabilities {
                        filter_eq: true,
                        scan: false,
                        update: false,
                    },
                },
                field(FIELD_TAGS, "tags", ValueKind::StrList),
                field(FIELD_SOURCE, "source", ValueKind::Str),
                field(FIELD_METADATA_JSON, "metadata_json", ValueKind::Str),
                field(FIELD_CREATED_AT, "created_at_unix_ms", ValueKind::I64),
                field(FIELD_UPDATED_AT, "updated_at_unix_ms", ValueKind::I64),
                field(FIELD_MEMORY_TYPE, "memory_type", ValueKind::Str),
                field(FIELD_STATUS, "status", ValueKind::Str),
                field(FIELD_SENSITIVE, "sensitive", ValueKind::Bool),
                FieldDescriptor {
                    tag: FIELD_ACCESS_COUNT,
                    name: "access_count".into(),
                    value_kind: ValueKind::I64,
                    capabilities: FieldCapabilities {
                        filter_eq: false,
                        scan: true,
                        update: true,
                    },
                },
                field(FIELD_DELETED_AT, "deleted_at_unix_ms", ValueKind::I64),
                field(FIELD_NODE_ID, "node_id", ValueKind::Str),
            ],
            relations: RelationCapabilities {
                parent_children: false,
                neighbors: true,
            },
        }
    }

    /// The validate-then-apply shape every adapter uses — the one
    /// mutable field is `access_count`, checked non-negative.
    fn validate_batch(
        updates: &[TransactionOp],
        exists: impl Fn(RecordId) -> bool,
    ) -> Result<(), (usize, ErrorCode)> {
        for (i, op) in updates.iter().enumerate() {
            match (op.field, &op.value) {
                (FIELD_ACCESS_COUNT, ScanValue::I64(count)) => {
                    if !valid_access_count(*count) {
                        return Err((i, ErrorCode::Malformed));
                    }
                    if !exists(op.id) {
                        return Err((i, ErrorCode::RecordNotFound));
                    }
                }
                (FIELD_ACCESS_COUNT, _) => return Err((i, ErrorCode::Malformed)),
                (field, _) if READ_ONLY_FIELDS.contains(&field) => {
                    return Err((i, ErrorCode::Unsupported))
                }
                _ => return Err((i, ErrorCode::UnknownField)),
            }
        }
        Ok(())
    }

    fn apply_batch(
        inner: &mut MemoryProductionStack,
        updates: &[TransactionOp],
    ) -> Result<(), (usize, ErrorCode)> {
        for (i, op) in updates.iter().enumerate() {
            if let ScanValue::I64(count) = op.value {
                UpdateField::<Memory, AccessCountField>::update(inner, op.id, count)
                    .map_err(|_| (i, ErrorCode::RecordNotFound))?;
            }
        }
        Ok(())
    }

    /// `INS-FR-006` (ADR-0046): the whole field list against this
    /// domain's schema, before any write — all thirteen tags exactly once
    /// with a value of its kind, `tags` as a `StrList`, `access_count`
    /// non-negative. `Malformed` for a missing, repeated, or wrong-kind
    /// field; `UnknownField` for a tag this domain doesn't have.
    fn memory_from_fields(
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<Memory, ErrorCode> {
        let mut content = None;
        let mut category = None;
        let mut tags = None;
        let mut source = None;
        let mut metadata_json = None;
        let mut created_at = None;
        let mut updated_at = None;
        let mut memory_type = None;
        let mut status = None;
        let mut sensitive = None;
        let mut access_count = None;
        let mut deleted_at = None;
        let mut node_id = None;
        for (tag, value) in fields {
            match (tag, value) {
                (FIELD_CONTENT, ScanValue::Str(v)) if content.is_none() => content = Some(v),
                (FIELD_CATEGORY, ScanValue::Str(v)) if category.is_none() => category = Some(v),
                (FIELD_TAGS, ScanValue::StrList(v)) if tags.is_none() => tags = Some(v),
                (FIELD_SOURCE, ScanValue::Str(v)) if source.is_none() => source = Some(v),
                (FIELD_METADATA_JSON, ScanValue::Str(v)) if metadata_json.is_none() => {
                    metadata_json = Some(v)
                }
                (FIELD_CREATED_AT, ScanValue::I64(v)) if created_at.is_none() => {
                    created_at = Some(v)
                }
                (FIELD_UPDATED_AT, ScanValue::I64(v)) if updated_at.is_none() => {
                    updated_at = Some(v)
                }
                (FIELD_MEMORY_TYPE, ScanValue::Str(v)) if memory_type.is_none() => {
                    memory_type = Some(v)
                }
                (FIELD_STATUS, ScanValue::Str(v)) if status.is_none() => status = Some(v),
                (FIELD_SENSITIVE, ScanValue::Bool(v)) if sensitive.is_none() => sensitive = Some(v),
                (FIELD_ACCESS_COUNT, ScanValue::I64(v))
                    if access_count.is_none() && valid_access_count(v) =>
                {
                    access_count = Some(v)
                }
                (FIELD_DELETED_AT, ScanValue::I64(v)) if deleted_at.is_none() && v >= 0 => {
                    deleted_at = Some(v)
                }
                (FIELD_NODE_ID, ScanValue::Str(v)) if node_id.is_none() => node_id = Some(v),
                (tag, _) if tag <= FIELD_NODE_ID => return Err(ErrorCode::Malformed),
                _ => return Err(ErrorCode::UnknownField),
            }
        }
        let (
            Some(content),
            Some(category),
            Some(tags),
            Some(source),
            Some(metadata_json),
            Some(created_at_unix_ms),
            Some(updated_at_unix_ms),
            Some(memory_type),
            Some(status),
            Some(sensitive),
            Some(access_count),
            Some(deleted_at_unix_ms),
            Some(node_id),
        ) = (
            content,
            category,
            tags,
            source,
            metadata_json,
            created_at,
            updated_at,
            memory_type,
            status,
            sensitive,
            access_count,
            deleted_at,
            node_id,
        )
        else {
            return Err(ErrorCode::Malformed);
        };
        Ok(Memory {
            id,
            content,
            category,
            tags,
            source,
            metadata_json,
            created_at_unix_ms,
            updated_at_unix_ms,
            memory_type,
            status,
            sensitive,
            access_count,
            deleted_at_unix_ms,
            node_id,
        })
    }

    /// `ISO-FR-002`/`ISO-FR-006` — see `DogConnectionStore::check_read_set`
    /// for the full contract; identical shape here.
    fn check_read_set(
        reads: &[(RecordId, FieldRef, ScanValue)],
        get: impl Fn(RecordId) -> Option<Memory>,
    ) -> Result<(), (usize, ErrorCode)> {
        for (id, field, value) in reads {
            let current = get(*id).and_then(|memory| {
                Self::fields_of(memory)
                    .into_iter()
                    .find(|(tag, _)| tag == field)
                    .map(|(_, v)| v)
            });
            if current.as_ref() != Some(value) {
                return Err((0, ErrorCode::Conflict));
            }
        }
        Ok(())
    }

    /// The wire shape of one memory, in tag order — `get`, `scan_all`,
    /// and the read-set check all go through here so they cannot drift.
    fn fields_of(memory: Memory) -> Vec<(FieldRef, ScanValue)> {
        vec![
            (FIELD_CONTENT, ScanValue::Str(memory.content)),
            (FIELD_CATEGORY, ScanValue::Str(memory.category)),
            (FIELD_TAGS, ScanValue::StrList(memory.tags)),
            (FIELD_SOURCE, ScanValue::Str(memory.source)),
            (FIELD_METADATA_JSON, ScanValue::Str(memory.metadata_json)),
            (FIELD_CREATED_AT, ScanValue::I64(memory.created_at_unix_ms)),
            (FIELD_UPDATED_AT, ScanValue::I64(memory.updated_at_unix_ms)),
            (FIELD_MEMORY_TYPE, ScanValue::Str(memory.memory_type)),
            (FIELD_STATUS, ScanValue::Str(memory.status)),
            (FIELD_SENSITIVE, ScanValue::Bool(memory.sensitive)),
            (FIELD_ACCESS_COUNT, ScanValue::I64(memory.access_count)),
            (FIELD_DELETED_AT, ScanValue::I64(memory.deleted_at_unix_ms)),
            (FIELD_NODE_ID, ScanValue::Str(memory.node_id)),
        ]
    }

    /// `ADR-0072`'s `MVCC2-FR-008`: record one whole-record write
    /// (`Insert`/`Replace`) into the MVCC version index — a fresh txn id
    /// (assigned *here*, inside whichever `with_exclusive` section the
    /// caller already holds — `MVCC2-FR-002`'s round-nine apply-time
    /// rule), the existence field plus every field `fields` carries. A
    /// no-op if this table was never given `with_mvcc` or never went
    /// MVCC-active (`MVCC2-FR-001`).
    fn mvcc_record_write(&self, fields: &[(FieldRef, ScanValue)], id: RecordId) {
        let Some(mvcc) = &self.mvcc else { return };
        if !mvcc.state.is_active() {
            return;
        }
        let txn_id = mvcc.state.counter().next();
        mvcc.state.with_index(|index| {
            index.record_write(
                (id, mvcc::EXISTENCE_FIELD),
                txn_id,
                Some(ScanValue::Bool(true)),
            );
            for (tag, value) in fields {
                index.record_write((id, *tag), txn_id, Some(value.clone()));
            }
        });
    }

    /// `ADR-0072`'s `MVCC2-FR-008`: record a delete — a tombstone on the
    /// existence field only; a snapshot that finds it absent never
    /// consults any individual field's own chain (`MvccIndex::read`'s own
    /// contract, checked in [`ConnectionStore::mvcc_get`] first). Same
    /// activation-gating as [`Self::mvcc_record_write`].
    fn mvcc_record_delete(&self, id: RecordId) {
        let Some(mvcc) = &self.mvcc else { return };
        if !mvcc.state.is_active() {
            return;
        }
        let txn_id = mvcc.state.counter().next();
        mvcc.state
            .with_index(|index| index.record_write((id, mvcc::EXISTENCE_FIELD), txn_id, None));
    }

    /// `ADR-0072`'s `MVCC2-FR-008`: record a session `Commit`'s whole
    /// staged batch — one txn id for every key it touches (`MVCC2-FR-002`).
    /// A no-op (no `fetch_add`, matching `mvcc_spike.rs`'s own proven
    /// empty-commit rule) if `updates` is empty or this table isn't
    /// MVCC-active. Must be called from inside the same `with_exclusive`
    /// section that already ran [`Self::apply_batch`] for this batch.
    fn mvcc_record_transaction(&self, updates: &[TransactionOp]) {
        let Some(mvcc) = &self.mvcc else { return };
        if !mvcc.state.is_active() || updates.is_empty() {
            return;
        }
        let txn_id = mvcc.state.counter().next();
        mvcc.state
            .with_index(|index| Self::record_transaction_into(index, txn_id, updates));
    }

    /// `JMC-FR-001` (ADR-0114): [`Self::mvcc_record_transaction`] for a
    /// batch `with_journal` replayed, at open — the same record, with
    /// the reclaim trigger held off and the append count reset after
    /// (`RVL-FR-002`).
    fn mvcc_record_transaction_quiet(&self, updates: &[TransactionOp]) {
        let Some(mvcc) = &self.mvcc else { return };
        if !mvcc.state.is_active() || updates.is_empty() {
            return;
        }
        let txn_id = mvcc.state.counter().next();
        mvcc.state
            .with_index_quiet(|index| Self::record_transaction_into(index, txn_id, updates));
    }

    fn record_transaction_into(index: &mut MvccIndex, txn_id: TxnId, updates: &[TransactionOp]) {
        for op in updates {
            index.record_write((op.id, op.field), txn_id, Some(op.value.clone()));
        }
    }

    /// `ADR-0072`'s `MVCC2-FR-002`/`008`: record an atomic `WriteBatch`'s
    /// whole set of writes — one shared txn id for every key any op in
    /// `ops` actually wrote, decided by `results` (a `Duplicate`/
    /// `NotFound`/`GuardFailed`/`Linked`/`AlreadyLinked`/`Failed` outcome
    /// wrote nothing, so it records nothing). `Link` touches no field
    /// [`ConnectionStore::mvcc_get`] ever reads, so it is never recorded.
    /// Must be called from inside the same `with_exclusive` section that
    /// already ran every op in `ops`. A no-op if `ops` is empty or
    /// nothing in it actually wrote (matching the empty-commit rule).
    fn mvcc_record_write_ops(&self, ops: &[WriteOp], results: &[WriteResult]) {
        let Some(mvcc) = &self.mvcc else { return };
        if !mvcc.state.is_active() {
            return;
        }
        let writes = Self::recorded_writes(ops, results);
        if writes.is_empty() {
            return;
        }
        let txn_id = mvcc.state.counter().next();
        mvcc.state
            .with_index(|index| Self::record_writes_into(index, txn_id, &writes));
    }

    /// `JMC-FR-001` (ADR-0114): [`Self::mvcc_record_write_ops`] for a
    /// batch `with_journal` replayed, at open — the same record, with
    /// the reclaim trigger held off and the append count reset after
    /// (`RVL-FR-002`).
    fn mvcc_record_write_ops_quiet(&self, ops: &[WriteOp], results: &[WriteResult]) {
        let Some(mvcc) = &self.mvcc else { return };
        if !mvcc.state.is_active() {
            return;
        }
        let writes = Self::recorded_writes(ops, results);
        if writes.is_empty() {
            return;
        }
        let txn_id = mvcc.state.counter().next();
        mvcc.state
            .with_index_quiet(|index| Self::record_writes_into(index, txn_id, &writes));
    }

    /// The writes an atomic batch actually made, by outcome: `None`
    /// fields for a record the batch deleted (every entry of that id,
    /// so a `Replace` before its own `Delete` records the deletion too).
    fn recorded_writes(ops: &[WriteOp], results: &[WriteResult]) -> Vec<RecordedWrite> {
        let deletes: std::collections::HashSet<RecordId> = ops
            .iter()
            .zip(results)
            .filter_map(|(op, result)| match (op, result) {
                (WriteOp::Delete { id }, WriteResult::Deleted) => Some(*id),
                _ => None,
            })
            .collect();
        ops.iter()
            .zip(results)
            .filter_map(|(op, result)| match (op, result) {
                (WriteOp::Insert { id, fields }, WriteResult::Inserted)
                | (WriteOp::Replace { id, fields }, WriteResult::Replaced)
                | (WriteOp::ReplaceIf { id, fields, .. }, WriteResult::Replaced) => {
                    Some((*id, (!deletes.contains(id)).then(|| fields.clone())))
                }
                (WriteOp::Delete { id }, WriteResult::Deleted) => Some((*id, None)),
                _ => None,
            })
            .collect()
    }

    fn record_writes_into(index: &mut MvccIndex, txn_id: TxnId, writes: &[RecordedWrite]) {
        for (id, fields) in writes {
            let Some(fields) = fields else {
                index.record_write((*id, mvcc::EXISTENCE_FIELD), txn_id, None);
                continue;
            };
            index.record_write(
                (*id, mvcc::EXISTENCE_FIELD),
                txn_id,
                Some(ScanValue::Bool(true)),
            );
            for (tag, value) in fields {
                index.record_write((*id, *tag), txn_id, Some(value.clone()));
            }
        }
    }

    /// `ADR-0072`'s `MVCC2-FR-010`: flush the current MVCC index to
    /// `<mmap_path>.mvcc`, recording the insert log's current entry count
    /// and `journal_entries` (the journal's own current count, from
    /// whichever boundary is flushing — a live checkpoint's own
    /// [`super::journal::Turn::journal_entries`], or the count `Self::
    /// mvcc_begin`'s own baseline flush measures itself). `true` if there
    /// was nothing to flush (MVCC never enabled or never activated — not
    /// a failure) or the flush succeeded; `false` only on a real I/O
    /// failure, in which case the caller must not reclaim the source log
    /// (`MVCC2-FR-010`'s own flush-before-reclaim gate).
    fn mvcc_flush_now(&self, journal_entries: u64) -> bool {
        let Some(mvcc) = &self.mvcc else { return true };
        if !mvcc.state.is_active() {
            return true;
        }
        let insert_log_count = insert_log::read_entries::<Memory, RecordId>(
            &insert_log::log_path(&mvcc.mmap_path),
            Memory::SCHEMA_TAG,
        )
        .map(|entries| entries.len())
        .unwrap_or(0);
        mvcc.state
            .flush(&mvcc.mmap_path, insert_log_count, journal_entries as usize)
            .is_ok()
    }
}

/// `WBT-FR-003` (ADR-0060): one parsed, pre-validated write of an atomic
/// [`WriteOp`] batch — the field lists already decoded to a [`Memory`]
/// and the guard/label already checked, so the exclusive section only
/// reads endpoints and applies.
enum PreparedWrite {
    Insert(Memory),
    Replace(Memory),
    ReplaceIf(Memory, Predicate),
    Delete(RecordId),
    Link {
        left: RecordId,
        right: RecordId,
        relation: String,
    },
}

impl MemoryConnectionStore {
    /// Parse and pre-validate one op with no lock held (`WBT-FR-003`):
    /// the field list to a `Memory`, the `ReplaceIf` guard as a `Query`
    /// predicate (`GRD-FR-004`), the `Link` label against this table's
    /// relations. Any failure aborts the atomic batch before a write.
    fn prepare_write(schema: &DomainSchema, op: &WriteOp) -> Result<PreparedWrite, ErrorCode> {
        Ok(match op {
            WriteOp::Insert { id, fields } => {
                PreparedWrite::Insert(Self::memory_from_fields(*id, fields.clone())?)
            }
            WriteOp::Replace { id, fields } => {
                PreparedWrite::Replace(Self::memory_from_fields(*id, fields.clone())?)
            }
            WriteOp::ReplaceIf { id, fields, guard } => {
                validate_predicate(schema, guard)?;
                PreparedWrite::ReplaceIf(
                    Self::memory_from_fields(*id, fields.clone())?,
                    guard.clone(),
                )
            }
            WriteOp::Delete { id } => PreparedWrite::Delete(*id),
            WriteOp::Link {
                left,
                right,
                relation,
            } => {
                if !MEMORY_RELATION_LABELS.contains(&relation.as_str()) {
                    return Err(ErrorCode::Malformed);
                }
                PreparedWrite::Link {
                    left: *left,
                    right: *right,
                    relation: relation.clone(),
                }
            }
        })
    }

    /// Apply one prepared write to the locked stack (`WBT-FR-003`).
    /// Every soft outcome is a [`WriteResult`]; only a storage I/O error
    /// (or a link to a missing own-table endpoint) is a hard `Err` — and
    /// the batch's own-endpoint existence was checked before this ran.
    fn apply_prepared(
        inner: &mut MemoryProductionStack,
        prepared: PreparedWrite,
    ) -> Result<WriteResult, ErrorCode> {
        Ok(match prepared {
            PreparedWrite::Insert(memory) => match Insert::insert(inner, memory) {
                Ok(()) => WriteResult::Inserted,
                Err(InsertError::Duplicate(_)) => WriteResult::Duplicate,
                Err(InsertError::Durability(_)) => return Err(ErrorCode::Storage),
            },
            PreparedWrite::Replace(memory) => match Replace::replace(inner, memory) {
                Ok(()) => WriteResult::Replaced,
                Err(ReplaceError::NotFound(_)) => WriteResult::NotFound,
                Err(ReplaceError::Durability(_)) => return Err(ErrorCode::Storage),
            },
            PreparedWrite::ReplaceIf(memory, guard) => {
                let id = memory.id;
                match GetById::<Memory>::get(inner, id) {
                    None => WriteResult::NotFound,
                    Some(stored) => {
                        if predicate_matches(&Self::fields_of(stored), &guard) {
                            match Replace::replace(inner, memory) {
                                Ok(()) => WriteResult::Replaced,
                                Err(ReplaceError::NotFound(_)) => WriteResult::NotFound,
                                Err(ReplaceError::Durability(_)) => return Err(ErrorCode::Storage),
                            }
                        } else {
                            WriteResult::GuardFailed
                        }
                    }
                }
            }
            PreparedWrite::Delete(id) => match Delete::<Memory>::delete(inner, id) {
                Ok(()) => WriteResult::Deleted,
                Err(DeleteError::NotFound(_)) => WriteResult::NotFound,
                Err(DeleteError::Durability(_)) => return Err(ErrorCode::Storage),
            },
            PreparedWrite::Link {
                left,
                right,
                relation,
            } => match MultiLink::link(inner, &relation, left, right) {
                Ok(crate::generic::LinkOutcome::Linked) => WriteResult::Linked,
                Ok(crate::generic::LinkOutcome::AlreadyLinked) => WriteResult::AlreadyLinked,
                Err(LinkError::UnknownRecord(_)) => return Err(ErrorCode::RecordNotFound),
                Err(LinkError::SelfLoop(_) | LinkError::InvalidLabel(_)) => {
                    return Err(ErrorCode::Malformed)
                }
                Err(LinkError::Durability(_)) => return Err(ErrorCode::Storage),
            },
        })
    }
}

impl ConnectionStore for MemoryConnectionStore {
    /// `JSM-FR-001` (ADR-0115): the journal's live figures, when there
    /// is one.
    fn journal_stats(&self) -> Option<JournalStats> {
        self.journal.as_ref().map(CommitGroup::stats)
    }

    fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>> {
        self.store.get::<Memory>(id).map(Self::fields_of)
    }

    /// `SQL-FR-004`/`SQL-FR-005` (ADR-0034): every id from `all_ids`,
    /// each mapped through this adapter's own `get`.
    fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
        self.store
            .all_ids::<Memory>()
            .into_iter()
            .filter_map(|id| self.get(id).map(|fields| (id, fields)))
            .collect()
    }

    /// `ORD-FR-005` (ADR-0059): a page ordered by `updated_at_unix_ms`
    /// is a range walk of the stack's sorted index — the page's cost,
    /// not the table's. Any other orderable field takes the scan path.
    /// `validate_page` has already matched the cursor's kind to the
    /// field's, so a cursor here is `I64` or absent.
    fn page(
        &self,
        order_by: FieldRef,
        after: Option<(ScanValue, RecordId)>,
        limit: usize,
    ) -> Result<Vec<PageRow>, ErrorCode> {
        if order_by != FIELD_UPDATED_AT {
            return Ok(page_by_scan(self, order_by, after, limit));
        }
        let cursor = match after {
            None => None,
            Some((ScanValue::I64(stamp), id)) => Some((stamp, id)),
            Some(_) => return Err(ErrorCode::Malformed),
        };
        Ok(self
            .store
            .page_by::<Memory, UpdatedAtOrder>(cursor, limit)
            .into_iter()
            .filter_map(|id| self.get(id).map(|fields| (id, fields)))
            .collect())
    }

    /// `PGD-FR-004` (ADR-0089): [`ConnectionStore::page`]'s twin walked
    /// backward — the sorted index from the cursor down, the page's cost.
    fn page_desc(
        &self,
        order_by: FieldRef,
        before: Option<(ScanValue, RecordId)>,
        limit: usize,
    ) -> Result<Vec<PageRow>, ErrorCode> {
        if order_by != FIELD_UPDATED_AT {
            return Ok(page_by_scan_desc(self, order_by, before, limit));
        }
        let cursor = match before {
            None => None,
            Some((ScanValue::I64(stamp), id)) => Some((stamp, id)),
            Some(_) => return Err(ErrorCode::Malformed),
        };
        Ok(self
            .store
            .page_by_desc::<Memory, UpdatedAtOrder>(cursor, limit)
            .into_iter()
            .filter_map(|id| self.get(id).map(|fields| (id, fields)))
            .collect())
    }

    /// `PAG-FR-002` (ADR-0055): the sort key of every record read
    /// straight off [`Memory`], so a page materializes only its own rows.
    /// A field this arm list does not name falls back to the wire shape
    /// — the same key [`page_key`] derives for the trait default.
    fn page_keys(&self, order_by: FieldRef) -> Vec<(RecordId, i128)> {
        self.store
            .all_ids::<Memory>()
            .into_iter()
            .filter_map(|id| {
                let record = self.store.get::<Memory>(id)?;
                let key = match order_by {
                    FIELD_CREATED_AT => Some(i128::from(record.created_at_unix_ms)),
                    FIELD_UPDATED_AT => Some(i128::from(record.updated_at_unix_ms)),
                    FIELD_ACCESS_COUNT => Some(i128::from(record.access_count)),
                    FIELD_DELETED_AT => Some(i128::from(record.deleted_at_unix_ms)),
                    _ => None,
                };
                let key = key.unwrap_or_else(|| page_key(&Self::fields_of(record), order_by, id).0);
                Some((id, key))
            })
            .collect()
    }

    /// `FPW-FR-001`–`003` (ADR-0076), `FPM-FR-001`–`003` (ADR-0077): a
    /// page ordered by `updated_at_unix_ms` is the bounded walk of the
    /// stack's sorted index — bounds on that field start and cut it,
    /// every other predicate is passed over until the page fills — the
    /// cost of the page for bounds alone, the page divided by the
    /// rejects' selectivity otherwise (`Page`'s own consistency class);
    /// a filter that plans the declared `category` index is the trait
    /// default's body, bucket first.
    fn filtered_page(
        &self,
        order_by: FieldRef,
        after: Option<(ScanValue, RecordId)>,
        limit: usize,
        filter: &[Predicate],
    ) -> Result<Vec<PageRow>, ErrorCode> {
        if !bounded_walk_applies(order_by, self.range_field(), &self.describe(), filter) {
            return Ok(filtered_page_by_candidates(
                self, order_by, after, limit, filter,
            ));
        }
        bounded_filtered_page(
            self,
            |start, limit| self.store.page_by::<Memory, UpdatedAtOrder>(start, limit),
            order_by,
            after,
            limit,
            filter,
        )
    }

    /// `QPR-FR-002` (ADR-0075): the field [`MemoryProductionStack`]'s
    /// sorted index is over (`ORD-FR-004`).
    fn range_field(&self) -> Option<FieldRef> {
        Some(FIELD_UPDATED_AT)
    }

    /// `QPR-FR-002` (ADR-0075): a `WHERE` range on `updated_at_unix_ms`
    /// is a walk of the stack's sorted index between the two bounds —
    /// the same set [`Self::page`] walks from a cursor, every key bound
    /// turned into a pair bound by [`uuid_pair_bounds`]. Any other field
    /// is `Unsupported`; a non-`I64` bound is `Malformed`.
    fn range_ids(
        &self,
        field: FieldRef,
        lower: Bound<ScanValue>,
        upper: Bound<ScanValue>,
    ) -> Result<Vec<RecordId>, ErrorCode> {
        if field != FIELD_UPDATED_AT {
            return Err(ErrorCode::Unsupported);
        }
        let (lower, upper) = uuid_pair_bounds(lower, upper)?;
        Ok(self.store.range_by::<Memory, UpdatedAtOrder>(lower, upper))
    }

    /// `QPB-FR-002` (ADR-0079): [`Self::range_ids`] with a budget — the
    /// stack's `range_by_limited`, `None` past `limit` ids.
    /// `QCW-FR-002` (ADR-0081): how many records a `WHERE` range on
    /// `field_updated_at` admits — the sorted index's own count between the
    /// bounds, no record read.
    fn range_count(
        &self,
        field: FieldRef,
        lower: Bound<ScanValue>,
        upper: Bound<ScanValue>,
    ) -> Result<u64, ErrorCode> {
        if field != FIELD_UPDATED_AT {
            return Err(ErrorCode::Unsupported);
        }
        let (lower, upper) = uuid_pair_bounds(lower, upper)?;
        Ok(self
            .store
            .range_count::<Memory, UpdatedAtOrder>(lower, upper) as u64)
    }

    /// `QKW-FR-002` (ADR-0082): the `field_updated_at` keys a `WHERE`
    /// range admits, ascending — the sorted index's own keys between
    /// the bounds, no record read.
    fn range_keys(
        &self,
        field: FieldRef,
        lower: Bound<ScanValue>,
        upper: Bound<ScanValue>,
    ) -> Result<Vec<i64>, ErrorCode> {
        if field != FIELD_UPDATED_AT {
            return Err(ErrorCode::Unsupported);
        }
        let (lower, upper) = uuid_pair_bounds(lower, upper)?;
        Ok(self
            .store
            .range_keys::<Memory, UpdatedAtOrder>(lower, upper))
    }

    /// `QRF-FR-002` (ADR-0087): the walk folded into [`KeyStats`] —
    /// `range_keys` with nothing materialized.
    fn range_stats(
        &self,
        field: FieldRef,
        lower: Bound<ScanValue>,
        upper: Bound<ScanValue>,
    ) -> Result<KeyStats, ErrorCode> {
        if field != FIELD_UPDATED_AT {
            return Err(ErrorCode::Unsupported);
        }
        let (lower, upper) = uuid_pair_bounds(lower, upper)?;
        Ok(self.store.range_fold::<Memory, UpdatedAtOrder, _, _>(
            lower,
            upper,
            KeyStats::default(),
            KeyStats::with,
        ))
    }

    fn range_ids_limited(
        &self,
        field: FieldRef,
        lower: Bound<ScanValue>,
        upper: Bound<ScanValue>,
        limit: usize,
    ) -> Result<Option<Vec<RecordId>>, ErrorCode> {
        if field != FIELD_UPDATED_AT {
            return Err(ErrorCode::Unsupported);
        }
        let (lower, upper) = uuid_pair_bounds(lower, upper)?;
        Ok(self
            .store
            .range_by_limited::<Memory, UpdatedAtOrder>(lower, upper, limit))
    }

    fn filter_eq(&self, field: FieldRef, value: &ScanValue) -> Result<Vec<RecordId>, ErrorCode> {
        match (field, value) {
            (FIELD_CATEGORY, ScanValue::Str(category)) => {
                Ok(self.store.filter_eq::<Memory, CategoryField>(category))
            }
            (FIELD_CATEGORY, _) => Err(ErrorCode::Malformed),
            (field, _) if field <= FIELD_NODE_ID => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    fn scan_field(&self, field: FieldRef) -> Result<Vec<ScanValue>, ErrorCode> {
        match field {
            FIELD_ACCESS_COUNT => Ok(self
                .store
                .scan::<Memory, AccessCountField>()
                .into_iter()
                .map(ScanValue::I64)
                .collect()),
            field if READ_ONLY_FIELDS.contains(&field) => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    fn update_field(
        &self,
        id: RecordId,
        field: FieldRef,
        value: ScanValue,
    ) -> Result<bool, ErrorCode> {
        match (field, value) {
            (FIELD_ACCESS_COUNT, ScanValue::I64(count)) => {
                if !valid_access_count(count) {
                    return Err(ErrorCode::Malformed);
                }
                if self.journals_updates() {
                    return self.journaled_update(TransactionOp {
                        id,
                        field: FIELD_ACCESS_COUNT,
                        value: ScanValue::I64(count),
                    });
                }
                // `MUR-FR-001` (ADR-0108): the in-place write and its MVCC
                // record in one exclusive section, as a batch's are.
                let written = self.store.with_exclusive(|inner| {
                    let written =
                        UpdateField::<Memory, AccessCountField>::update(inner, id, count).is_ok();
                    if written {
                        self.mvcc_record_transaction(&[TransactionOp {
                            id,
                            field: FIELD_ACCESS_COUNT,
                            value: ScanValue::I64(count),
                        }]);
                    }
                    written
                });
                if !written {
                    return Ok(false);
                }
                // `RVW-FR-001` (ADR-0110): no journal covers an in-place
                // update, so its MVCC record must reach `.mvcc` now —
                // exactly as the non-journaled batch arm flushes — or a
                // restart would reconstruct an index without it.
                let journal_count = self
                    .journal
                    .as_ref()
                    .map(CommitGroup::entries_since_checkpoint)
                    .unwrap_or(0);
                if !self.mvcc_flush_now(journal_count) {
                    return Err(ErrorCode::Storage);
                }
                self.sync_update_ack()?;
                Ok(true)
            }
            (FIELD_ACCESS_COUNT, _) => Err(ErrorCode::Malformed),
            (field, _) if READ_ONLY_FIELDS.contains(&field) => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    /// `INS-FR-006` (ADR-0046): validate, then one write under the
    /// store's own lock.
    fn insert_record(
        &self,
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<InsertOutcome, ErrorCode> {
        let memory = Self::memory_from_fields(id, fields)?;
        // `MVCC2-FR-002`/`008` (round nine): the MVCC record happens
        // inside the *same* `with_exclusive` section as the primary
        // write, not after `self.store.insert` has already released the
        // lock — that gap is exactly the cross-path ordering race round
        // nine's own research found and fixed by assigning `txn_id` at
        // apply time, under whichever lock the write already holds.
        self.store
            .with_exclusive(|inner| match Insert::insert(inner, memory) {
                Ok(()) => {
                    self.mvcc_record_write(
                        &Self::fields_of(GetById::<Memory>::get(inner, id).expect("just inserted")),
                        id,
                    );
                    // `ADR-0072`'s `MVCC2-FR-010`: a non-journaled table has
                    // no checkpoint boundary to piggyback on, so every
                    // MVCC-recording write flushes immediately — otherwise
                    // this write is invisible to `.mvcc` until the next
                    // `Compact`, and a restart in between silently loses it
                    // from MVCC's own bookkeeping (never a plain read).
                    if !self.mvcc_flush_now(0) {
                        return Err(ErrorCode::Storage);
                    }
                    Ok(InsertOutcome::Inserted)
                }
                Err(InsertError::Duplicate(_)) => Ok(InsertOutcome::Duplicate),
                Err(InsertError::Durability(_)) => Err(ErrorCode::Storage),
            })
    }

    /// `REP-FR-005` (ADR-0049): the same validation as `insert_record`,
    /// then one whole-record write under the store's own lock. An
    /// unknown id is the normal outcome, not an error.
    fn replace_record(
        &self,
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<ReplaceOutcome, ErrorCode> {
        let memory = Self::memory_from_fields(id, fields)?;
        self.store
            .with_exclusive(|inner| match Replace::replace(inner, memory) {
                Ok(()) => {
                    self.mvcc_record_write(
                        &Self::fields_of(GetById::<Memory>::get(inner, id).expect("just replaced")),
                        id,
                    );
                    // See `insert_record`: a non-journaled table must flush
                    // on every MVCC-recording write, not just at `Compact`.
                    if !self.mvcc_flush_now(0) {
                        return Err(ErrorCode::Storage);
                    }
                    Ok(ReplaceOutcome::Replaced)
                }
                Err(ReplaceError::NotFound(_)) => Ok(ReplaceOutcome::NotFound),
                Err(ReplaceError::Durability(_)) => Err(ErrorCode::Storage),
            })
    }

    /// `GRD-FR-003` (ADR-0054): `replace_record`'s validation, then the
    /// read, the guard over this adapter's own wire shape of the stored
    /// record, and the write under one acquisition of the store's lock.
    fn replace_record_if(
        &self,
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
        guard: &Predicate,
    ) -> Result<ReplaceIfOutcome, ErrorCode> {
        let memory = Self::memory_from_fields(id, fields)?;
        self.store.with_exclusive(|inner| {
            let Some(current) = GetById::<Memory>::get(inner, id) else {
                return Ok(ReplaceIfOutcome::NotFound);
            };
            if !predicate_matches(&Self::fields_of(current), guard) {
                return Ok(ReplaceIfOutcome::GuardFailed);
            }
            match Replace::replace(inner, memory) {
                Ok(()) => {
                    self.mvcc_record_write(
                        &Self::fields_of(GetById::<Memory>::get(inner, id).expect("just replaced")),
                        id,
                    );
                    // See `insert_record`: a non-journaled table must flush
                    // on every MVCC-recording write, not just at `Compact`.
                    if !self.mvcc_flush_now(0) {
                        return Err(ErrorCode::Storage);
                    }
                    Ok(ReplaceIfOutcome::Replaced)
                }
                Err(ReplaceError::NotFound(_)) => Ok(ReplaceIfOutcome::NotFound),
                Err(ReplaceError::Durability(_)) => Err(ErrorCode::Storage),
            }
        })
    }

    /// `DEL-FR-006` (ADR-0051): one whole-record delete under the store's
    /// own lock — the record and, within this table, every edge touching
    /// it. An unknown id is the normal outcome, not an error.
    fn delete_record(&self, id: RecordId) -> Result<DeleteOutcome, ErrorCode> {
        self.store
            .with_exclusive(|inner| match Delete::<Memory>::delete(inner, id) {
                Ok(()) => {
                    self.mvcc_record_delete(id);
                    // See `insert_record`: a non-journaled table must flush
                    // on every MVCC-recording write, not just at `Compact`.
                    if !self.mvcc_flush_now(0) {
                        return Err(ErrorCode::Storage);
                    }
                    Ok(DeleteOutcome::Deleted)
                }
                Err(DeleteError::NotFound(_)) => Ok(DeleteOutcome::NotFound),
                Err(DeleteError::Durability(_)) => Err(ErrorCode::Storage),
            })
    }

    /// `CMP-FR-006` (ADR-0052): the stack compacted under the store's own
    /// write lock; a file that could not be rewritten is `Storage`.
    fn compact(&self) -> Result<crate::generic::CompactionReport, ErrorCode> {
        self.store.with_exclusive(|inner| {
            // `ADR-0072`'s `MVCC2-FR-010` item 2: flush MVCC history
            // *before* `Compact`'s own insert-log clear, inside the same
            // lock `compact()` itself runs under — so nothing can write
            // (and thus append to the insert log) in between. If the
            // flush fails, refuse to compact at all rather than let the
            // clear proceed and lose whatever history was still only in
            // the log.
            let journal_entries = self
                .journal
                .as_ref()
                .map(CommitGroup::entries_since_checkpoint)
                .unwrap_or(0);
            // `HRC-FR-002` (ADR-0096): reclaim history no open snapshot
            // needs, then flush — the flushed store is the reclaimed one.
            if let Some(mvcc) = &self.mvcc {
                mvcc.state.reclaim();
            }
            if !self.mvcc_flush_now(journal_entries) {
                return Err(ErrorCode::Storage);
            }
            let report =
                crate::generic::query::Compact::compact(inner).map_err(|_| ErrorCode::Storage)?;
            // `RGM-FR-010` (ADR-0121): the clear emptied the insert log, so
            // the history's "entries reflected" must say so — left at the
            // pre-clear count, the next reopen skipped that many *new*
            // entries, and a journaled insert among them (a `Duplicate`
            // on replay, the store having folded the log first) never
            // reached the index.
            if !self.mvcc_flush_now(journal_entries) {
                return Err(ErrorCode::Storage);
            }
            Ok(report)
        })
    }

    /// `BAK-FR-002`/`006` (ADR-0065): copy every file under
    /// `self.backup_source`'s prefix, under the store's own write lock —
    /// `Unsupported` when this adapter was built with no known data
    /// directory (`with_backup_source` never called).
    fn backup(&self, target_dir: &Path) -> Result<BackupReport, ErrorCode> {
        let base = self.backup_source.as_ref().ok_or(ErrorCode::Unsupported)?;
        self.store
            .with_exclusive(|_| copy_table_files(base, target_dir))
            .map_err(|_| ErrorCode::Storage)
    }

    /// `RPL-FR-003` (ADR-0067): [`ConnectionStore::backup`]'s reading
    /// twin — see `DogConnectionStore::fetch_snapshot`.
    fn fetch_snapshot(&self) -> Result<Vec<(String, Vec<u8>)>, ErrorCode> {
        let base = self.backup_source.as_ref().ok_or(ErrorCode::Unsupported)?;
        self.store
            .with_exclusive(|_| read_table_files(base))
            .map_err(|e| match e {
                ReadTableFilesError::TooLarge => ErrorCode::TooLarge,
                ReadTableFilesError::Io => ErrorCode::Storage,
            })
    }

    /// `MEM-FR-005`: `Memory` has no `ChildOf` relation.
    fn parent(&self, _id: RecordId) -> Result<ParentLookup, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn children(&self, _id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    /// `TBL-FR-008` (ADR-0050): the entity ids a memory mentions — or,
    /// given an entity id, the memories that mention it, since the edge
    /// is stored in both directions. Every id on the far side is an
    /// `Entity` (another table's record); none is a `Memory`.
    fn neighbors(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        Ok(self.store.all_neighbors::<Memory>(id))
    }

    fn neighbors_by_relation(
        &self,
        id: RecordId,
        relation: &str,
    ) -> Result<Vec<RecordId>, ErrorCode> {
        match self.store.neighbors_by_relation::<Memory>(relation, id) {
            Some(records) => Ok(records),
            None => Err(ErrorCode::Malformed),
        }
    }

    /// `CNT-FR-002` (ADR-0057): one read under the store's lock; an
    /// unknown label is `Malformed`, as for `neighbors_by_relation`.
    /// `WBT-FR-002`/`003` (ADR-0060): pipelined is the trait default
    /// (each op through its single-shot method); atomic parses and
    /// pre-validates every op, then — under one exclusive section —
    /// checks each `Link`'s own-table endpoint and applies every op, so
    /// a precondition failure aborts with nothing applied and the batch
    /// is isolated from other connections. Not crash-atomic: a storage
    /// I/O error mid-apply is not rolled back (the named follow-on).
    fn write_batch(
        &self,
        ops: &[WriteOp],
        atomic: bool,
    ) -> Result<Vec<WriteResult>, (usize, ErrorCode)> {
        if !atomic {
            return Ok(ops.iter().map(|op| self.apply_write_op(op)).collect());
        }
        let schema = self.describe();
        let mut prepared = Vec::with_capacity(ops.len());
        for (i, op) in ops.iter().enumerate() {
            prepared.push(Self::prepare_write(&schema, op).map_err(|code| (i, code))?);
        }
        let apply = |inner: &mut MemoryProductionStack| {
            for (i, p) in prepared.iter().enumerate() {
                if let PreparedWrite::Link { left, .. } = p {
                    if GetById::<Memory>::get(inner, *left).is_none() {
                        return Err((i, ErrorCode::RecordNotFound));
                    }
                }
            }
            let mut results = Vec::with_capacity(prepared.len());
            for (i, p) in prepared.into_iter().enumerate() {
                results.push(Self::apply_prepared(inner, p).map_err(|code| (i, code))?);
            }
            // `ADR-0072`'s `MVCC2-FR-002`/`008`: an atomic `WriteBatch` is
            // one unit — one txn id for the whole batch, assigned here,
            // inside the same exclusive section, immediately after every
            // op in it has actually applied.
            self.mvcc_record_write_ops(ops, &results);
            Ok(results)
        };
        match &self.journal {
            // `WBJ-FR-004`: no journal, no change from before ADR-0063,
            // except `ADR-0072`'s `MVCC2-FR-010`: with no checkpoint
            // boundary to piggyback on, every MVCC-recording batch flushes
            // immediately after `apply` records it, still inside the same
            // exclusive section.
            None => self.store.with_exclusive(|inner| {
                let results = apply(inner)?;
                if !self.mvcc_flush_now(0) {
                    return Err((0, ErrorCode::Storage));
                }
                Ok(results)
            }),
            // `WBJ-FR-002` (ADR-0063): journal the raw, already-validated
            // `ops` before applying — a crash after the journal `fsync`
            // but before/during apply replays cleanly (`WBJ-FR-003`).
            // The apply closure's own results escape via `results_cell`
            // since `CommitGroup::commit_write`'s own contract only
            // reports whether a checkpoint happened, not arbitrary data.
            Some(journal) => {
                let results_cell: std::cell::RefCell<Option<Vec<WriteResult>>> =
                    std::cell::RefCell::new(None);
                journal
                    .commit_write(ops, |turn| {
                        self.store.with_exclusive(|inner| {
                            let results = apply(inner)?;
                            *results_cell.borrow_mut() = Some(results);
                            Ok(turn.checkpoint_due
                                && inner.checkpoint_flush().is_ok()
                                && self.mvcc_flush_now(turn.journal_entries))
                        })
                    })
                    .map_err(|e| match e {
                        CommitError::Journal(_) => (0, ErrorCode::Journal),
                        CommitError::Apply(e) => e,
                    })?;
                Ok(results_cell
                    .into_inner()
                    .expect("commit_write's apply closure always sets results_cell on Ok"))
            }
        }
    }

    fn count_edges(&self, relation: &str) -> Result<u64, ErrorCode> {
        match self.store.count_edges::<Memory>(relation) {
            Some(count) => Ok(count as u64),
            None => Err(ErrorCode::Malformed),
        }
    }

    fn list_relation_kinds(&self) -> Vec<String> {
        self.store.relation_kinds::<Memory>()
    }

    /// `TBL-FR-008`: the one relation, `mentions`, with its rows in the
    /// `entity` table — so a same-table `Join` over it is `Unsupported`
    /// and a cross-table one needs `right_table: Some("entity")`. The
    /// unfiltered `neighbors` is deliberately *not* listed: its far side
    /// is never this table's rows.
    fn describe_relations(&self) -> Vec<RelationDescriptor> {
        MEMORY_RELATION_LABELS
            .iter()
            .map(|label| RelationDescriptor {
                name: label.to_string(),
                kind: JoinRelation::Neighbors(Some(label.to_string())),
                target_table: Some(MEMORY_FOREIGN_TABLE.to_string()),
            })
            .collect()
    }

    /// `TBL-FR-008`: a fixed label set — `mentions` only; any other label
    /// is `Malformed`. `left` must be a memory (`RecordNotFound`); `right`
    /// is an entity id this adapter cannot see — the server checks it
    /// against the `entity` table before this is called (`TBL-FR-007`).
    fn link_records(
        &self,
        left: RecordId,
        right: RecordId,
        relation: &str,
    ) -> Result<LinkOutcome, ErrorCode> {
        if !MEMORY_RELATION_LABELS.contains(&relation) {
            return Err(ErrorCode::Malformed);
        }
        match self.store.link_by_relation::<Memory>(relation, left, right) {
            Ok(crate::generic::LinkOutcome::Linked) => Ok(LinkOutcome::Linked),
            Ok(crate::generic::LinkOutcome::AlreadyLinked) => Ok(LinkOutcome::AlreadyLinked),
            Err(LinkError::UnknownRecord(_)) => Err(ErrorCode::RecordNotFound),
            Err(LinkError::SelfLoop(_) | LinkError::InvalidLabel(_)) => Err(ErrorCode::Malformed),
            Err(LinkError::Durability(_)) => Err(ErrorCode::Storage),
        }
    }

    fn table_name(&self) -> &str {
        "memory"
    }

    /// `DEL-FR-005` (ADR-0051): the `entity` table deleted `id` — every
    /// `mentions` edge to it goes (the consumer's `DELETE FROM
    /// memory_entities WHERE entity_id = ?`). A label this domain lacks
    /// is `Malformed`.
    fn detach_record(&self, relation: &str, id: RecordId) -> Result<usize, ErrorCode> {
        if !MEMORY_RELATION_LABELS.contains(&relation) {
            return Err(ErrorCode::Malformed);
        }
        match self.store.detach::<Memory>(relation, id) {
            Ok(n) => Ok(n),
            Err(DeleteError::NotFound(_)) => Err(ErrorCode::Malformed),
            Err(DeleteError::Durability(_)) => Err(ErrorCode::Storage),
        }
    }

    /// `STV-FR-002`: `validate_batch` on this one operation.
    fn validate_op(&self, op: &TransactionOp) -> Result<(), ErrorCode> {
        Self::validate_batch(std::slice::from_ref(op), |id| {
            self.store.get::<Memory>(id).is_some()
        })
        .map_err(|(_, code)| code)
    }

    fn describe(&self) -> DomainSchema {
        Self::schema()
    }

    fn apply_transaction(
        &self,
        updates: &[TransactionOp],
        read_set: &[(RecordId, FieldRef, ScanValue)],
    ) -> Result<(), (usize, ErrorCode)> {
        // See `DogConnectionStore::apply_transaction` for the two paths
        // (`GRP-FR-001`–`005`) and where the read-set check runs in each.
        // `ADR-0072`'s `MVCC2-FR-008`: the MVCC record runs inside the
        // same exclusive section, immediately after `apply_batch`
        // succeeds — this ordinary (no-session) path has no
        // `mvcc_snapshot` to conflict-check against, so it records
        // unconditionally, exactly as a validated session write does.
        match &self.journal {
            None => self.store.with_exclusive(|inner| {
                Self::validate_batch(updates, |id| GetById::<Memory>::get(inner, id).is_some())?;
                Self::check_read_set(read_set, |id| GetById::<Memory>::get(inner, id))?;
                Self::apply_batch(inner, updates)?;
                // `SYU-FR-003` (ADR-0097): no journal covers this batch,
                // so force the slots to disk before acknowledging.
                if self.sync_updates && inner.checkpoint_flush().is_err() {
                    return Err((0, ErrorCode::Storage));
                }
                self.mvcc_record_transaction(updates);
                // `ADR-0072`'s `MVCC2-FR-010`: no journal means no
                // checkpoint boundary, so a committed transaction flushes
                // MVCC history immediately — otherwise it is invisible to
                // `.mvcc` until the next `Compact`, and a restart in
                // between silently loses it from MVCC's own bookkeeping.
                if !self.mvcc_flush_now(0) {
                    return Err((0, ErrorCode::Storage));
                }
                Ok(())
            }),
            Some(journal) => {
                Self::validate_batch(updates, |id| self.store.get::<Memory>(id).is_some())?;
                journal
                    .commit(updates, |turn| {
                        self.store.with_exclusive(|inner| {
                            Self::check_read_set(read_set, |id| GetById::<Memory>::get(inner, id))?;
                            Self::apply_batch(inner, updates)?;
                            self.mvcc_record_transaction(updates);
                            Ok(turn.checkpoint_due
                                && inner.checkpoint_flush().is_ok()
                                && self.mvcc_flush_now(turn.journal_entries))
                        })
                    })
                    .map_err(|e| match e {
                        CommitError::Journal(_) => (0, ErrorCode::Journal),
                        CommitError::Apply(e) => e,
                    })
            }
        }
    }

    /// `ADR-0072`'s `MVCC2-FR-007`: [`Self::apply_transaction`]'s exact
    /// contract, plus a write-write conflict check against
    /// `mvcc_snapshot` — every key `updates` targets whose current
    /// last-write txn exceeds `mvcc_snapshot` refuses the whole commit,
    /// applying nothing, before the MVCC record. Both the check and the
    /// record run inside the same exclusive section as `apply_batch` —
    /// `check_read_set`'s own precedent.
    fn apply_transaction_mvcc(
        &self,
        updates: &[TransactionOp],
        read_set: &[(RecordId, FieldRef, ScanValue)],
        mvcc_snapshot: u64,
    ) -> Result<(), (usize, ErrorCode)> {
        let Some(mvcc) = &self.mvcc else {
            return Err((0, ErrorCode::Unsupported));
        };
        let conflict_check = || -> Result<(), (usize, ErrorCode)> {
            let conflicts = mvcc.state.with_index(|index| {
                index.conflicts(updates.iter().map(|op| (op.id, op.field)), mvcc_snapshot)
            });
            if conflicts {
                return Err((0, ErrorCode::Conflict));
            }
            Ok(())
        };
        match &self.journal {
            None => self.store.with_exclusive(|inner| {
                Self::validate_batch(updates, |id| GetById::<Memory>::get(inner, id).is_some())?;
                Self::check_read_set(read_set, |id| GetById::<Memory>::get(inner, id))?;
                conflict_check()?;
                Self::apply_batch(inner, updates)?;
                // `SYU-FR-003` (ADR-0097): no journal covers this batch,
                // so force the slots to disk before acknowledging.
                if self.sync_updates && inner.checkpoint_flush().is_err() {
                    return Err((0, ErrorCode::Storage));
                }
                self.mvcc_record_transaction(updates);
                // See `apply_transaction`: no journal means no checkpoint
                // boundary, so this commit flushes MVCC history now.
                if !self.mvcc_flush_now(0) {
                    return Err((0, ErrorCode::Storage));
                }
                Ok(())
            }),
            Some(journal) => {
                Self::validate_batch(updates, |id| self.store.get::<Memory>(id).is_some())?;
                journal
                    .commit(updates, |turn| {
                        self.store.with_exclusive(|inner| {
                            Self::check_read_set(read_set, |id| GetById::<Memory>::get(inner, id))?;
                            conflict_check()?;
                            Self::apply_batch(inner, updates)?;
                            self.mvcc_record_transaction(updates);
                            Ok(turn.checkpoint_due
                                && inner.checkpoint_flush().is_ok()
                                && self.mvcc_flush_now(turn.journal_entries))
                        })
                    })
                    .map_err(|e| match e {
                        CommitError::Journal(_) => (0, ErrorCode::Journal),
                        CommitError::Apply(e) => e,
                    })
            }
        }
    }

    /// `ADR-0072`'s `MVCC2-FR-012`: `Memory` implements real MVCC.
    fn mvcc_supported(&self) -> bool {
        self.mvcc.is_some()
    }

    /// `ADR-0072`'s `MVCC2-FR-001`/`005`: activate (baseline-seed every
    /// currently-live record, once) and open a new snapshot.
    fn mvcc_begin(&self) -> u64 {
        let Some(mvcc) = &self.mvcc else { return 0 };
        if !mvcc.state.is_active() {
            self.store.with_exclusive(|inner| {
                mvcc.state.activate();
                for id in AllIds::<Memory>::all_ids(inner) {
                    if let Some(record) = GetById::<Memory>::get(inner, id) {
                        let fields = Self::fields_of(record);
                        mvcc.state.with_index_quiet(|index| {
                            index.record_write(
                                (id, mvcc::EXISTENCE_FIELD),
                                mvcc::BASELINE_TXN,
                                Some(ScanValue::Bool(true)),
                            );
                            for (tag, value) in &fields {
                                index.record_write(
                                    (id, *tag),
                                    mvcc::BASELINE_TXN,
                                    Some(value.clone()),
                                );
                            }
                        });
                    }
                }
            });
            // `MVCC2-FR-001`: durably record the baseline now, so this
            // table's "has ever gone MVCC-active" signal survives a
            // restart even if nothing is ever written again. Reflects
            // everything currently in the insert log/journal too — a
            // baseline scan reads the *current* live state, which
            // already includes every insert-log entry folded in (the
            // stack was opened through `GenericMmapStore::open`, which
            // folds the log before this ever runs).
            let journal_count = self
                .journal
                .as_ref()
                .map(CommitGroup::entries_since_checkpoint)
                .unwrap_or(0);
            let _ = self.mvcc_flush_now(journal_count);
        }
        // `RVW-FR-002` (ADR-0110): read and register under one lock.
        mvcc.state.open_snapshot()
    }

    fn mvcc_release(&self, snapshot_txn: u64) {
        if let Some(mvcc) = &self.mvcc {
            mvcc.state.open_snapshots().deregister(snapshot_txn);
        }
    }

    /// `MHE-FR-001` (ADR-0109): the version-index entries this table
    /// holds — `None` before `with_mvcc`/`open_with_mvcc`.
    fn mvcc_history_entries(&self) -> Option<u64> {
        self.mvcc
            .as_ref()
            .map(|mvcc| mvcc.state.with_index(|index| index.history_len()) as u64)
    }

    /// `ADR-0072`'s `MVCC2-FR-006`: `id`'s value as of `snapshot_txn` —
    /// the existence field decides absence outright (a delete's
    /// tombstone, or a snapshot before this table's own baseline covered
    /// `id`); each schema field is then read from its own chain.
    /// [`ErrorCode::Conflict`] is reused as the typed "history reclaimed"
    /// signal — the same code a write-write conflict already uses,
    /// distinguished by the caller never treating `Commit`'s own
    /// `TransactionFailed { index: 0, .. }` shape as this method's return.
    fn mvcc_get(
        &self,
        id: RecordId,
        snapshot_txn: u64,
    ) -> Result<Option<Vec<(FieldRef, ScanValue)>>, ErrorCode> {
        let Some(mvcc) = &self.mvcc else {
            return Ok(self.get(id));
        };
        if !mvcc.state.is_active() {
            return Ok(self.get(id));
        }
        let existed = mvcc
            .state
            .with_index(|index| index.read(&(id, mvcc::EXISTENCE_FIELD), snapshot_txn));
        match existed {
            Err(mvcc::HistoryReclaimed) => return Err(ErrorCode::Conflict),
            Ok(None) => return Ok(None),
            Ok(Some(_)) => {}
        }
        let mut fields = Vec::with_capacity(Self::schema().fields.len());
        for field in Self::schema().fields {
            let value = mvcc
                .state
                .with_index(|index| index.read(&(id, field.tag), snapshot_txn));
            match value {
                Err(mvcc::HistoryReclaimed) => return Err(ErrorCode::Conflict),
                Ok(Some(v)) => fields.push((field.tag, v)),
                // Existence says present but this one field has no entry
                // at or below the snapshot — cannot happen once the
                // baseline scan has run (it seeds every field for every
                // live record atomically with existence), so this is
                // defensive, not an expected path.
                Ok(None) => {}
            }
        }
        Ok(Some(fields))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generic::memory::{
        create_memory_production_stack, open_memory_production_stack_portable,
    };
    use crate::test_support::fresh_temp_dir;
    use uuid::Uuid;

    fn memory(n: u128, category: &str, sensitive: bool) -> Memory {
        Memory {
            id: Uuid::from_u128(n),
            content: format!("memory {n}"),
            category: category.into(),
            tags: vec!["t".into()],
            source: "manual".into(),
            metadata_json: "{}".into(),
            created_at_unix_ms: 1_000 * n as i64,
            updated_at_unix_ms: 1_000 * n as i64,
            memory_type: "unclassified".into(),
            status: "active".into(),
            sensitive,
            access_count: 0,
            deleted_at_unix_ms: 0,
            node_id: String::new(),
        }
    }

    fn sample_adapter() -> MemoryConnectionStore {
        let dir = fresh_temp_dir("server_memory_adapter").unwrap();
        let path = dir.join("memories.mmap");
        let stack = create_memory_production_stack(
            vec![
                memory(1, "general", false),
                memory(2, "preference", false),
                memory(3, "general", true),
            ],
            &[],
            &path,
        )
        .unwrap();
        MemoryConnectionStore::new(GenericProductionStore::new(stack))
    }

    fn full_fields(n: u128) -> Vec<(FieldRef, ScanValue)> {
        MemoryConnectionStore::fields_of(memory(n, "decision", false))
    }

    /// `FPW-FR-004`/`FPW-FR-005` (ADR-0076) and `FPM-FR-004` (ADR-0077),
    /// acceptance criterion 3: for every eligible shape — each
    /// comparator, two-sided, `=`, a client cursor combined with a lower
    /// bound in both orders, a cursor past every row, an empty filter,
    /// every `limit` relation to the match count, an inverted range,
    /// `i64::MIN`/`MAX` literals, and (since ADR-0077) `Ne` on the walked
    /// field and predicates on an unindexed second field the walk must
    /// pass rejects for — the override returns the identical sequence
    /// [`filtered_page_by_candidates`] (the trait default's own body)
    /// returns; and every ineligible shape (an equality on the declared
    /// index, another `order_by`) too, because it *is* the default there.
    #[test]
    fn filtered_page_bounded_walk_returns_the_default_body_s_exact_sequence() {
        use crate::server::protocol::{CompareOp, Predicate};
        let dir = fresh_temp_dir("server_memory_bounded_walk").unwrap();
        let path = dir.join("memories.mmap");
        // Stamps 1_000 * n, with 3 sharing 2's stamp for the id tie-break
        // and 6 at the extremes' neighbours.
        let mut seeded: Vec<Memory> = (1..=6)
            .map(|n| memory(n, if n % 2 == 0 { "general" } else { "preference" }, false))
            .collect();
        seeded[2].updated_at_unix_ms = 2_000;
        seeded[5].updated_at_unix_ms = 2_000;
        let stack = create_memory_production_stack(seeded, &[], &path).unwrap();
        let adapter = MemoryConnectionStore::new(GenericProductionStore::new(stack));
        let p = |field: FieldRef, op, value: ScanValue| Predicate { field, op, value };
        let i = |op, k: i64| p(FIELD_UPDATED_AT, op, ScanValue::I64(k));
        use CompareOp::{Eq, Ge, Gt, Le, Lt, Ne};
        let id = Uuid::from_u128;
        let cursor = |k: i64, n: u128| Some((ScanValue::I64(k), id(n)));

        type Shape = (
            FieldRef,
            Option<(ScanValue, RecordId)>,
            usize,
            Vec<Predicate>,
        );
        let shapes: Vec<Shape> = vec![
            (FIELD_UPDATED_AT, None, 10, vec![]),
            (FIELD_UPDATED_AT, None, 2, vec![]),
            (FIELD_UPDATED_AT, None, 10, vec![i(Gt, 2_000)]),
            (FIELD_UPDATED_AT, None, 10, vec![i(Ge, 2_000)]),
            (FIELD_UPDATED_AT, None, 10, vec![i(Lt, 2_000)]),
            (FIELD_UPDATED_AT, None, 10, vec![i(Le, 2_000)]),
            (FIELD_UPDATED_AT, None, 10, vec![i(Eq, 2_000)]),
            (FIELD_UPDATED_AT, None, 2, vec![i(Eq, 2_000)]),
            (FIELD_UPDATED_AT, None, 10, vec![i(Ge, 2_000), i(Lt, 5_000)]),
            (FIELD_UPDATED_AT, None, 2, vec![i(Ge, 2_000), i(Lt, 5_000)]),
            (FIELD_UPDATED_AT, None, 3, vec![i(Ge, 2_000), i(Lt, 5_000)]),
            (FIELD_UPDATED_AT, None, 10, vec![i(Gt, 4_000), i(Lt, 2_000)]),
            (FIELD_UPDATED_AT, None, 10, vec![i(Gt, 2_000), i(Lt, 2_000)]),
            (FIELD_UPDATED_AT, None, 10, vec![i(Gt, 1_000), i(Gt, 3_000)]),
            (
                FIELD_UPDATED_AT,
                None,
                10,
                vec![i(Ge, i64::MIN), i(Le, i64::MAX)],
            ),
            (FIELD_UPDATED_AT, None, 10, vec![i(Gt, i64::MAX)]),
            (FIELD_UPDATED_AT, cursor(2_000, 2), 10, vec![]),
            (FIELD_UPDATED_AT, cursor(2_000, 2), 10, vec![i(Gt, 1_000)]),
            (FIELD_UPDATED_AT, cursor(1_000, 1), 10, vec![i(Ge, 4_000)]),
            (FIELD_UPDATED_AT, cursor(2_000, 3), 1, vec![i(Le, 4_000)]),
            (FIELD_UPDATED_AT, cursor(9_000, 1), 10, vec![i(Ge, 1_000)]),
            (FIELD_UPDATED_AT, cursor(2_000, 2), 10, vec![i(Eq, 2_000)]),
            // `FPM-FR-002`/`003` (ADR-0077): rejects the walk passes —
            // `Ne` on the walked field, a predicate on an unindexed
            // field (every record's `source` is "manual"; `content` is
            // unique per record), with and without a cursor, a page the
            // rejects leave short or empty.
            (FIELD_UPDATED_AT, None, 10, vec![i(Ne, 2_000)]),
            (
                FIELD_UPDATED_AT,
                None,
                1,
                vec![i(Ge, 1_000), i(Ne, 1_000), i(Ne, 2_000)],
            ),
            (
                FIELD_UPDATED_AT,
                None,
                2,
                vec![
                    i(Gt, 1_000),
                    p(FIELD_CONTENT, Eq, ScanValue::Str("memory 4".into())),
                ],
            ),
            (
                FIELD_UPDATED_AT,
                None,
                3,
                vec![
                    i(Le, 4_000),
                    p(FIELD_CONTENT, Ne, ScanValue::Str("memory 2".into())),
                ],
            ),
            (
                FIELD_UPDATED_AT,
                cursor(2_000, 2),
                2,
                vec![p(FIELD_SOURCE, Ne, ScanValue::Str("manual".into()))],
            ),
            (
                FIELD_UPDATED_AT,
                cursor(2_000, 2),
                2,
                vec![
                    i(Lt, 5_000),
                    p(FIELD_SOURCE, Eq, ScanValue::Str("manual".into())),
                ],
            ),
            // Ineligible: the default, through the override — an
            // equality on the declared `category` index (equality-first,
            // `FPM-FR-001`), another `order_by`.
            (
                FIELD_UPDATED_AT,
                None,
                10,
                vec![
                    i(Ge, 2_000),
                    p(FIELD_CATEGORY, Eq, ScanValue::Str("general".into())),
                ],
            ),
            (FIELD_CREATED_AT, None, 10, vec![i(Ge, 2_000)]),
            (FIELD_ACCESS_COUNT, cursor(2, 2), 2, vec![i(Gt, 1_000)]),
        ];
        for (order_by, after, limit, filter) in shapes {
            let expected =
                filtered_page_by_candidates(&adapter, order_by, after.clone(), limit, &filter);
            let actual = adapter
                .filtered_page(order_by, after.clone(), limit, &filter)
                .unwrap();
            assert_eq!(
                actual, expected,
                "order_by {order_by}, after {after:?}, limit {limit}, filter {filter:?}"
            );
        }

        // A few pinned answers, so the oracle itself is not trusted blindly.
        let ids = |rows: Vec<PageRow>| rows.into_iter().map(|(id, _)| id).collect::<Vec<_>>();
        assert_eq!(
            ids(adapter
                .filtered_page(FIELD_UPDATED_AT, None, 10, &[i(Eq, 2_000)])
                .unwrap()),
            vec![id(2), id(3), id(6)],
            "key 2_000 in id order"
        );
        assert_eq!(
            ids(adapter
                .filtered_page(FIELD_UPDATED_AT, cursor(2_000, 3), 10, &[i(Le, 4_000)])
                .unwrap()),
            vec![id(6), id(4)],
            "strictly after (2_000, 3): 6 at 2_000, then 4 at 4_000; 5 is cut"
        );
        assert_eq!(
            ids(adapter
                .filtered_page(FIELD_UPDATED_AT, None, 10, &[i(Gt, 4_000), i(Lt, 2_000)])
                .unwrap()),
            Vec::<Uuid>::new(),
            "inverted: empty, no error"
        );
        assert_eq!(
            ids(adapter
                .filtered_page(FIELD_UPDATED_AT, None, 10, &[i(Ne, 2_000)])
                .unwrap()),
            vec![id(1), id(4), id(5)],
            "`Ne` on the walked field: the three at other stamps, walked past 2, 3, 6"
        );
        assert_eq!(
            ids(adapter
                .filtered_page(
                    FIELD_UPDATED_AT,
                    None,
                    2,
                    &[
                        i(Gt, 1_000),
                        p(FIELD_CONTENT, Eq, ScanValue::Str("memory 4".into())),
                    ],
                )
                .unwrap()),
            vec![id(4)],
            "walked past 2, 3, 6 (rejected) to 4; 5 rejected; the index ends"
        );
    }

    /// `QPB-FR-002` (ADR-0079): the adapter's budgeted walk — the whole
    /// range within the budget, `None` past it, `Unsupported` for any
    /// field but `updated_at_unix_ms`, `Malformed` for a non-`I64` bound.
    #[test]
    fn range_ids_limited_walks_within_the_budget_and_yields_past_it() {
        let adapter = sample_adapter();
        let id = Uuid::from_u128;
        let lower = || Bound::Included(ScanValue::I64(1_000));
        assert_eq!(
            adapter.range_ids_limited(FIELD_UPDATED_AT, lower(), Bound::Unbounded, 3),
            Ok(Some(vec![id(1), id(2), id(3)])),
            "exactly the budget"
        );
        assert_eq!(
            adapter.range_ids_limited(FIELD_UPDATED_AT, lower(), Bound::Unbounded, 2),
            Ok(None)
        );
        assert_eq!(
            adapter.range_ids_limited(FIELD_CREATED_AT, lower(), Bound::Unbounded, 9),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            adapter.range_ids_limited(
                FIELD_UPDATED_AT,
                Bound::Included(ScanValue::Str("x".into())),
                Bound::Unbounded,
                9
            ),
            Err(ErrorCode::Malformed)
        );
    }

    /// `ADR-0072`: a table with two pre-existing records, built with
    /// [`MemoryConnectionStore::with_mvcc`] so `SESSION_MVCC_ISOLATION`
    /// is available — but not yet activated (`mvcc_begin` does that).
    fn sample_adapter_with_mvcc() -> MemoryConnectionStore {
        let dir = fresh_temp_dir("server_memory_mvcc_adapter").unwrap();
        let path = dir.join("memories.mmap");
        let stack = create_memory_production_stack(
            vec![memory(1, "general", false), memory(2, "preference", false)],
            &[],
            &path,
        )
        .unwrap();
        MemoryConnectionStore::new(GenericProductionStore::new(stack))
            .with_mvcc(&path)
            .unwrap()
    }

    /// `MVCC2-FR-001`/`013`: activation baseline-seeds a pre-existing
    /// record, so a snapshot opened right at activation still sees it —
    /// not absence.
    #[test]
    fn mvcc_begin_baseline_seeds_pre_existing_records() {
        let adapter = sample_adapter_with_mvcc();
        assert!(adapter.mvcc_supported());
        let id = Uuid::from_u128(1);
        let snapshot = adapter.mvcc_begin();
        assert_eq!(
            adapter.mvcc_get(id, snapshot).unwrap(),
            Some(full_fields_of(1, "general", false))
        );
        adapter.mvcc_release(snapshot);
    }

    /// `MVCC2-FR-006`/`008`, acceptance criterion 4: a real-MVCC
    /// snapshot survives a *concurrent ordinary* (non-session) write —
    /// not only a conflicting session commit — and a fresh snapshot
    /// taken afterward sees the new value.
    #[test]
    fn mvcc_snapshot_survives_a_concurrent_ordinary_replace() {
        let adapter = sample_adapter_with_mvcc();
        let id = Uuid::from_u128(1);
        let before = adapter.mvcc_begin();

        let mut edited = full_fields(1);
        edited[0] = (FIELD_CONTENT, ScanValue::Str("edited".into()));
        assert_eq!(
            adapter.replace_record(id, edited.clone()),
            Ok(ReplaceOutcome::Replaced)
        );

        assert_eq!(
            adapter.mvcc_get(id, before).unwrap(),
            Some(full_fields_of(1, "general", false)),
            "the snapshot still sees the pre-replace value"
        );
        let after = adapter.mvcc_begin();
        assert_eq!(
            adapter.mvcc_get(id, after).unwrap(),
            Some(edited),
            "a fresh snapshot sees the ordinary write"
        );
        adapter.mvcc_release(before);
        adapter.mvcc_release(after);
    }

    /// Acceptance criterion 5: a key created by an ordinary `Insert`
    /// after a snapshot began is invisible to that snapshot.
    #[test]
    fn mvcc_snapshot_does_not_see_a_key_created_after_it_began() {
        let adapter = sample_adapter_with_mvcc();
        let before = adapter.mvcc_begin();
        let new_id = Uuid::from_u128(99);
        assert_eq!(
            adapter.insert_record(new_id, full_fields(99)),
            Ok(InsertOutcome::Inserted)
        );
        assert_eq!(adapter.mvcc_get(new_id, before).unwrap(), None);
        let after = adapter.mvcc_begin();
        assert_eq!(
            adapter.mvcc_get(new_id, after).unwrap(),
            Some(full_fields(99))
        );
        adapter.mvcc_release(before);
        adapter.mvcc_release(after);
    }

    /// Acceptance criterion 3, adapted to a single-adapter test: a
    /// session's staged `UpdateField` batch commits cleanly against its
    /// own unchanged snapshot, but is refused `Conflict` — nothing
    /// applied — once a concurrent write has touched the same key.
    #[test]
    fn apply_transaction_mvcc_detects_a_write_write_conflict() {
        let adapter = sample_adapter_with_mvcc();
        let id = Uuid::from_u128(1);
        let snapshot = adapter.mvcc_begin();
        let update = TransactionOp {
            id,
            field: FIELD_ACCESS_COUNT,
            value: ScanValue::I64(5),
        };

        // Uncontested: applies and is visible afterward.
        assert_eq!(
            adapter.apply_transaction_mvcc(std::slice::from_ref(&update), &[], snapshot),
            Ok(())
        );
        assert_eq!(
            adapter.get(id).unwrap()[10],
            (FIELD_ACCESS_COUNT, ScanValue::I64(5))
        );

        // The same (now stale) snapshot's own second attempt conflicts
        // with the write it just made.
        assert_eq!(
            adapter.apply_transaction_mvcc(&[update], &[], snapshot),
            Err((0, ErrorCode::Conflict))
        );
        adapter.mvcc_release(snapshot);
    }

    /// Acceptance criterion 8: a `Compact` flushes MVCC history *before*
    /// clearing anything, and a reopen afterward still reconstructs the
    /// full, correct history from before that `Compact` — a snapshot
    /// taken before a write still sees the pre-write value, and a fresh
    /// snapshot after reopen sees the write.
    /// `HRC-FR-002`/`003` (ADR-0096): `Compact` reclaims every history
    /// entry below the oldest open snapshot and nothing that snapshot
    /// still needs; with no snapshot open, one entry per chain remains
    /// and the current value still reads.
    #[test]
    fn compact_reclaims_history_below_the_oldest_open_snapshot() {
        let adapter = sample_adapter_with_mvcc();
        let id = Uuid::from_u128(1);
        let history_len = || {
            adapter
                .mvcc
                .as_ref()
                .unwrap()
                .state
                .with_index(|index| index.history_len())
        };
        let write = |value: i64| {
            let snapshot = adapter.mvcc_begin();
            adapter
                .apply_transaction_mvcc(
                    &[TransactionOp {
                        id,
                        field: FIELD_ACCESS_COUNT,
                        value: ScanValue::I64(value),
                    }],
                    &[],
                    snapshot,
                )
                .unwrap();
            adapter.mvcc_release(snapshot);
        };
        write(1);
        let held = adapter.mvcc_begin();
        write(2);
        write(3);
        let before = history_len();

        adapter.compact().unwrap();
        let with_snapshot_open = history_len();
        assert!(
            with_snapshot_open < before,
            "with a snapshot open at {held} only the entries below it are reclaimed: {before} -> {with_snapshot_open}"
        );
        assert_eq!(
            adapter.mvcc_get(id, held).unwrap().unwrap()[FIELD_ACCESS_COUNT as usize].1,
            ScanValue::I64(1),
            "the open snapshot still reads the value it was opened on"
        );

        adapter.mvcc_release(held);
        adapter.compact().unwrap();
        let after = history_len();
        assert!(
            after < with_snapshot_open,
            "releasing the snapshot must let Compact reclaim more: {with_snapshot_open} -> {after}"
        );
        assert_eq!(
            adapter.get(id).unwrap()[FIELD_ACCESS_COUNT as usize].1,
            ScanValue::I64(3),
            "the current value is never reclaimed"
        );
    }

    /// `ART-FR-002`/`005` (ADR-0105): through the adapter, a threshold
    /// reclaims without any `Compact` — the held snapshot still reads
    /// its value, the current value reads, and a reopen reconstructs
    /// a consistent index.
    #[test]
    fn a_reclaim_threshold_bounds_history_without_a_compact() {
        let adapter = sample_adapter_with_mvcc().with_mvcc_reclaim_every(Some(2));
        let id = Uuid::from_u128(1);
        let state = || &adapter.mvcc.as_ref().unwrap().state;
        let history_len = || state().with_index(|index| index.history_len());
        let write = |value: i64| {
            let snapshot = adapter.mvcc_begin();
            adapter
                .apply_transaction_mvcc(
                    &[TransactionOp {
                        id,
                        field: FIELD_ACCESS_COUNT,
                        value: ScanValue::I64(value),
                    }],
                    &[],
                    snapshot,
                )
                .unwrap();
            adapter.mvcc_release(snapshot);
        };
        assert_eq!(state().reclaim_every(), Some(2));
        write(1);
        let held = adapter.mvcc_begin();
        let before = history_len();
        for value in 2..=9 {
            write(value);
        }
        let reclaims_while_held = state().auto_reclaims();
        assert!(
            reclaims_while_held >= 1,
            "eight appends past a threshold of two must have reclaimed"
        );
        // Every entry after the held snapshot is one it may still need,
        // so history grows while it is open — the trigger fires and
        // keeps them, exactly as `Compact` would.
        let while_held = history_len();
        assert!(while_held > before, "{before} -> {while_held}");
        assert_eq!(
            adapter.mvcc_get(id, held).unwrap().unwrap()[FIELD_ACCESS_COUNT as usize].1,
            ScanValue::I64(1),
            "the open snapshot still reads the value it was opened on"
        );
        adapter.mvcc_release(held);
        write(10);
        write(11);
        assert!(
            state().auto_reclaims() > reclaims_while_held,
            "the next threshold after the release reclaims again"
        );
        // The reclaim runs inside the committing session, whose own
        // snapshot (the previous commit) is still registered — so the
        // written chain keeps that one entry beside the current.
        assert!(
            history_len() <= before + 1,
            "nothing held: the history is back to a bounded size, {while_held} -> {}",
            history_len()
        );
        assert_eq!(
            adapter.get(id).unwrap()[FIELD_ACCESS_COUNT as usize].1,
            ScanValue::I64(11),
            "the current value is never reclaimed"
        );
    }

    /// `SYU-FR-001`–`003` (ADR-0097): with synced updates on, every
    /// answer is the same as without — `UpdateField` and a non-journaled
    /// `Transaction` batch acknowledge and read back — and a reopen from
    /// the files alone sees the values without any explicit `Flush`.
    /// What `msync` adds (durability past a power loss) is not
    /// observable in-process; the test pins the contract's shape.
    #[test]
    fn synced_updates_answer_exactly_as_unsynced_and_a_reopen_sees_them() {
        let dir = fresh_temp_dir("server_memory_synced_updates").unwrap();
        let path = dir.join("memories.mmap");
        let id = Uuid::from_u128(1);
        {
            let stack =
                create_memory_production_stack(vec![memory(1, "general", false)], &[], &path)
                    .unwrap();
            let adapter = MemoryConnectionStore::new(GenericProductionStore::new(stack))
                .with_synced_updates(true);
            assert_eq!(
                adapter.update_field(id, FIELD_ACCESS_COUNT, ScanValue::I64(7)),
                Ok(true)
            );
            assert_eq!(
                adapter.update_field(Uuid::from_u128(404), FIELD_ACCESS_COUNT, ScanValue::I64(1)),
                Ok(false)
            );
            assert_eq!(
                adapter.apply_transaction(
                    &[TransactionOp {
                        id,
                        field: FIELD_ACCESS_COUNT,
                        value: ScanValue::I64(8),
                    }],
                    &[],
                ),
                Ok(())
            );
            assert_eq!(
                adapter.get(id).unwrap()[FIELD_ACCESS_COUNT as usize].1,
                ScanValue::I64(8)
            );
        }
        let stack = crate::generic::memory::open_memory_production_stack_portable(&path).unwrap();
        let reopened = MemoryConnectionStore::new(GenericProductionStore::new(stack));
        assert_eq!(
            reopened.get(id).unwrap()[FIELD_ACCESS_COUNT as usize].1,
            ScanValue::I64(8)
        );
    }

    /// `JUF-FR-001`/`002` (ADR-0107): with a journal and the setting,
    /// `UpdateField` is a journaled one-operation batch — the entry is
    /// in the journal before the acknowledgement, the answers are the
    /// same as the in-place path's (`Ok(true)`, `Ok(false)` for a
    /// missing record, `Malformed` for a bad value), and a reopen
    /// through the journal sees the value. Without the setting the
    /// journaled adapter still writes in place, nothing journaled.
    #[test]
    fn journaled_synced_updates_go_through_the_journal_and_survive_a_reopen() {
        let dir = fresh_temp_dir("server_memory_journaled_updates").unwrap();
        let path = dir.join("memories.mmap");
        let journal = dir.join("memories.journal");
        let id = Uuid::from_u128(1);
        {
            let stack =
                create_memory_production_stack(vec![memory(1, "general", false)], &[], &path)
                    .unwrap();
            let unsynced =
                MemoryConnectionStore::with_journal(GenericProductionStore::new(stack), &journal)
                    .unwrap();
            assert!(!unsynced.journals_updates());
            let unsynced = unsynced.with_synced_updates(true);
            assert!(
                !unsynced.journals_updates(),
                "synced alone is msync, not the journal"
            );
            assert_eq!(
                unsynced.update_field(id, FIELD_ACCESS_COUNT, ScanValue::I64(5)),
                Ok(true)
            );
            assert_eq!(
                unsynced
                    .journal
                    .as_ref()
                    .unwrap()
                    .entries_since_checkpoint(),
                0,
                "unsynced: in place, nothing journaled"
            );
            let adapter = unsynced.with_journaled_updates(true);
            assert!(adapter.journals_updates());
            assert_eq!(
                adapter.update_field(id, FIELD_ACCESS_COUNT, ScanValue::I64(7)),
                Ok(true)
            );
            assert_eq!(
                adapter.journal.as_ref().unwrap().entries_since_checkpoint(),
                1,
                "synced: one journal entry per update"
            );
            assert_eq!(
                adapter.update_field(Uuid::from_u128(404), FIELD_ACCESS_COUNT, ScanValue::I64(1)),
                Ok(false)
            );
            assert_eq!(
                adapter.update_field(id, FIELD_ACCESS_COUNT, ScanValue::I64(-1)),
                Err(ErrorCode::Malformed)
            );
            assert_eq!(
                adapter.journal.as_ref().unwrap().entries_since_checkpoint(),
                1,
                "a refused update journals nothing"
            );
            assert_eq!(
                adapter.get(id).unwrap()[FIELD_ACCESS_COUNT as usize].1,
                ScanValue::I64(7)
            );
        }
        let stack = crate::generic::memory::open_memory_production_stack_portable(&path).unwrap();
        let reopened =
            MemoryConnectionStore::with_journal(GenericProductionStore::new(stack), &journal)
                .unwrap();
        assert_eq!(
            reopened.get(id).unwrap()[FIELD_ACCESS_COUNT as usize].1,
            ScanValue::I64(7)
        );
    }

    /// `MUR-FR-001`/`002` (ADR-0108): an in-place `UpdateField` on an
    /// MVCC-active table records its write in the version index like a
    /// batch does — a snapshot opened before it still reads the old
    /// value, the current value reads the new one, and the history
    /// grew by the one entry. Before this, the in-place path wrote the
    /// slot and told the index nothing.
    #[test]
    fn an_in_place_update_is_recorded_in_the_mvcc_index() {
        let adapter = sample_adapter_with_mvcc();
        let id = Uuid::from_u128(1);
        let state = || &adapter.mvcc.as_ref().unwrap().state;
        let held = adapter.mvcc_begin();
        let before = state().with_index(|index| index.history_len());
        let old = adapter.mvcc_get(id, held).unwrap().unwrap()[FIELD_ACCESS_COUNT as usize]
            .1
            .clone();
        assert_eq!(
            adapter.update_field(id, FIELD_ACCESS_COUNT, ScanValue::I64(41)),
            Ok(true)
        );
        assert_eq!(
            state().with_index(|index| index.history_len()),
            before + 1,
            "one entry for the one in-place write"
        );
        assert_eq!(
            adapter.mvcc_get(id, held).unwrap().unwrap()[FIELD_ACCESS_COUNT as usize].1,
            old,
            "the snapshot opened before the update still reads the old value"
        );
        assert_eq!(
            adapter.get(id).unwrap()[FIELD_ACCESS_COUNT as usize].1,
            ScanValue::I64(41)
        );
        adapter.mvcc_release(held);
        assert_eq!(
            adapter.update_field(Uuid::from_u128(404), FIELD_ACCESS_COUNT, ScanValue::I64(1)),
            Ok(false)
        );
        assert_eq!(
            state().with_index(|index| index.history_len()),
            before + 1,
            "a missing record records nothing"
        );
    }

    /// `MHE-FR-001` (ADR-0109): the history-size accessor behind the
    /// metric — `None` without MVCC state, the index's entry count with.
    #[test]
    fn mvcc_history_entries_is_none_without_mvcc_and_the_count_with() {
        assert_eq!(sample_adapter().mvcc_history_entries(), None);
        let adapter = sample_adapter_with_mvcc();
        assert_eq!(
            adapter.mvcc_history_entries(),
            Some(0),
            "state, not yet activated"
        );
        let _snapshot = adapter.mvcc_begin();
        let expected = adapter
            .mvcc
            .as_ref()
            .unwrap()
            .state
            .with_index(|index| index.history_len()) as u64;
        assert!(expected > 0, "activation seeds a baseline");
        assert_eq!(adapter.mvcc_history_entries(), Some(expected));
    }

    /// `RVL-FR-004` (ADR-0112): a journaled update whose record is deleted
    /// before the journal is checkpointed is moot at replay, not corrupt —
    /// the reopen through `with_journal` succeeds and the record stays
    /// deleted.
    #[test]
    fn replay_skips_a_journaled_update_to_a_record_deleted_afterwards() {
        let dir = fresh_temp_dir("server_memory_replay_deleted").unwrap();
        let path = dir.join("memories.mmap");
        let journal = dir.join("memories.journal");
        let id = Uuid::from_u128(1);
        {
            let stack = create_memory_production_stack(
                vec![memory(1, "general", false), memory(2, "general", false)],
                &[],
                &path,
            )
            .unwrap();
            let adapter =
                MemoryConnectionStore::with_journal(GenericProductionStore::new(stack), &journal)
                    .unwrap()
                    .with_journaled_updates(true);
            assert_eq!(
                adapter.update_field(id, FIELD_ACCESS_COUNT, ScanValue::I64(9)),
                Ok(true)
            );
            assert_eq!(
                adapter.journal.as_ref().unwrap().entries_since_checkpoint(),
                1
            );
            assert_eq!(adapter.delete_record(id), Ok(DeleteOutcome::Deleted));
        }
        let stack = crate::generic::memory::open_memory_production_stack_portable(&path).unwrap();
        let reopened =
            MemoryConnectionStore::with_journal(GenericProductionStore::new(stack), &journal)
                .unwrap();
        assert_eq!(reopened.get(id), None, "deleted stays deleted");
        assert!(reopened.get(Uuid::from_u128(2)).is_some());
    }

    #[test]
    fn compact_flushes_mvcc_history_and_a_reopen_reconstructs_it() {
        let dir = fresh_temp_dir("server_memory_mvcc_compact").unwrap();
        let path = dir.join("memories.mmap");
        let id = Uuid::from_u128(1);
        let stack =
            create_memory_production_stack(vec![memory(1, "general", false)], &[], &path).unwrap();
        let adapter = MemoryConnectionStore::new(GenericProductionStore::new(stack))
            .with_mvcc(&path)
            .unwrap();

        let before = adapter.mvcc_begin();
        assert_eq!(
            adapter.apply_transaction_mvcc(
                &[TransactionOp {
                    id,
                    field: FIELD_ACCESS_COUNT,
                    value: ScanValue::I64(9),
                }],
                &[],
                before,
            ),
            Ok(())
        );
        adapter.compact().unwrap();
        drop(adapter);

        let stack = open_memory_production_stack_portable(&path).unwrap();
        let reopened = MemoryConnectionStore::new(GenericProductionStore::new(stack))
            .with_mvcc(&path)
            .unwrap();
        assert!(reopened.mvcc_supported());
        let fields = reopened.mvcc_get(id, before).unwrap().unwrap();
        assert_eq!(
            fields[10],
            (FIELD_ACCESS_COUNT, ScanValue::I64(0)),
            "the pre-write snapshot still sees the pre-write value after reopen"
        );
        let after = reopened.mvcc_begin();
        assert_eq!(
            reopened.mvcc_get(id, after).unwrap().unwrap()[10],
            (FIELD_ACCESS_COUNT, ScanValue::I64(9)),
            "a fresh snapshot after reopen sees the write made before compact"
        );
    }

    /// `docs/design/MVCC-OPEN-HOOK-PROPOSAL.md` acceptance criterion 1:
    /// a table with a genuinely pending, unflushed insert-log entry
    /// (an ordinary `replace_record` made after activation, with no
    /// `Compact` and no explicit flush in between) at the moment of a
    /// simulated restart — `open_with_mvcc` on the reopened table still
    /// answers correctly, both for a snapshot taken before the write
    /// (sees the original value) and a fresh one after reopen (sees the
    /// write). `with_mvcc` alone cannot handle this: by the time it would
    /// run after the normal portable-open path, `GenericMmapStore::open`
    /// has already cleared the insert log this test relies on.
    /// `RVW-FR-001` (ADR-0110): an in-place `UpdateField` on an
    /// MVCC-active table survives a restart in the version index — no
    /// `Compact`, no session, no journal: the update's record is flushed
    /// to `.mvcc` before the acknowledgement, so `open_with_mvcc` rebuilds
    /// an index whose newest entry is the update, and a fresh snapshot
    /// reads the new value through `mvcc_get` as `get` does.
    #[test]
    fn an_in_place_update_survives_a_restart_in_the_mvcc_index() {
        let dir = fresh_temp_dir("server_memory_in_place_restart").unwrap();
        let path = dir.join("memories.mmap");
        let id = Uuid::from_u128(1);
        {
            let stack =
                create_memory_production_stack(vec![memory(1, "general", false)], &[], &path)
                    .unwrap();
            let adapter = MemoryConnectionStore::new(GenericProductionStore::new(stack))
                .with_mvcc(&path)
                .unwrap();
            let activated = adapter.mvcc_begin();
            adapter.mvcc_release(activated);
            assert_eq!(
                adapter.update_field(id, FIELD_ACCESS_COUNT, ScanValue::I64(41)),
                Ok(true)
            );
        }
        let reopened = MemoryConnectionStore::open_with_mvcc(&path).unwrap();
        let fresh = reopened.mvcc_begin();
        assert_eq!(
            reopened.mvcc_get(id, fresh).unwrap().unwrap()[FIELD_ACCESS_COUNT as usize].1,
            ScanValue::I64(41),
            "the index rebuilt from .mvcc holds the in-place update"
        );
        assert_eq!(
            reopened.get(id).unwrap()[FIELD_ACCESS_COUNT as usize].1,
            ScanValue::I64(41)
        );
    }

    #[test]
    fn open_with_mvcc_reconstructs_a_pending_unflushed_insert_log_entry() {
        let dir = fresh_temp_dir("server_memory_mvcc_open_hook").unwrap();
        let path = dir.join("memories.mmap");
        let id = Uuid::from_u128(1);
        let stack =
            create_memory_production_stack(vec![memory(1, "general", false)], &[], &path).unwrap();
        let adapter = MemoryConnectionStore::new(GenericProductionStore::new(stack))
            .with_mvcc(&path)
            .unwrap();

        let before = adapter.mvcc_begin();
        let original = adapter.get(id).unwrap();
        let mut edited = original.clone();
        edited[0] = (
            FIELD_CONTENT,
            ScanValue::Str("edited, never flushed".into()),
        );
        assert_eq!(
            adapter.replace_record(id, edited.clone()),
            Ok(ReplaceOutcome::Replaced),
            "an ordinary write — appends to the insert log, no Compact/flush follows"
        );
        // No `compact()`, no further `mvcc_begin()` — the replace above
        // is the *only* thing that touched the insert log, and it is
        // still sitting there, unflushed, when we drop and reopen.
        drop(adapter);

        let reopened = MemoryConnectionStore::open_with_mvcc(&path).unwrap();
        assert!(reopened.mvcc_supported());
        assert_eq!(
            reopened.mvcc_get(id, before).unwrap(),
            Some(original),
            "the pre-write snapshot still sees the pre-write value after reopen"
        );
        let after = reopened.mvcc_begin();
        assert_eq!(
            reopened.mvcc_get(id, after).unwrap(),
            Some(edited),
            "a fresh snapshot after reopen sees the pending write open_with_mvcc recovered"
        );
    }

    /// `MVCC2-FR-008`: an atomic `WriteBatch`'s `Insert`/`Replace`/
    /// `Delete` are recorded too, not only the single-shot paths — a
    /// snapshot taken before it still sees the pre-batch state.
    #[test]
    fn mvcc_snapshot_survives_a_concurrent_atomic_write_batch() {
        let adapter = sample_adapter_with_mvcc();
        let (existing, deleted) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let new_id = Uuid::from_u128(50);
        let before = adapter.mvcc_begin();

        let mut edited = full_fields(1);
        edited[0] = (FIELD_CONTENT, ScanValue::Str("batched edit".into()));
        let ops = vec![
            WriteOp::Replace {
                id: existing,
                fields: edited.clone(),
            },
            WriteOp::Delete { id: deleted },
            WriteOp::Insert {
                id: new_id,
                fields: full_fields(50),
            },
        ];
        assert_eq!(
            adapter.write_batch(&ops, true),
            Ok(vec![
                WriteResult::Replaced,
                WriteResult::Deleted,
                WriteResult::Inserted
            ])
        );

        assert_eq!(
            adapter.mvcc_get(existing, before).unwrap(),
            Some(full_fields_of(1, "general", false)),
            "the snapshot still sees the pre-batch value"
        );
        assert_eq!(
            adapter.mvcc_get(deleted, before).unwrap(),
            Some(full_fields_of(2, "preference", false)),
            "the snapshot still sees the not-yet-deleted record"
        );
        assert_eq!(
            adapter.mvcc_get(new_id, before).unwrap(),
            None,
            "the snapshot never sees a record inserted after it began"
        );

        let after = adapter.mvcc_begin();
        assert_eq!(adapter.mvcc_get(existing, after).unwrap(), Some(edited));
        assert_eq!(adapter.mvcc_get(deleted, after).unwrap(), None);
        assert!(adapter.mvcc_get(new_id, after).unwrap().is_some());
        adapter.mvcc_release(before);
        adapter.mvcc_release(after);
    }

    /// The round-ten fix: on a non-journaled table, `insert_record`/
    /// `replace_record`/`delete_record` each used to record into the
    /// in-memory MVCC index but never flush `<mmap_path>.mvcc` — only
    /// `Compact` did. Checked directly against the on-disk file via an
    /// independent `MvccState::open`, not via `open_with_mvcc`'s own
    /// insert-log fold: none of these three ever clear the insert log
    /// themselves (only `GenericMmapStore::open`/`Compact` do), so a
    /// restart-based test would reconstruct the same answer from the
    /// still-pending log regardless of whether the flush actually ran.
    #[test]
    fn insert_replace_delete_each_flush_mvcc_history_immediately_with_no_compact() {
        let dir = fresh_temp_dir("server_memory_mvcc_write_flush").unwrap();
        let path = dir.join("memories.mmap");
        let stack = create_memory_production_stack(
            vec![memory(1, "general", false), memory(2, "preference", false)],
            &[],
            &path,
        )
        .unwrap();
        let adapter = MemoryConnectionStore::new(GenericProductionStore::new(stack))
            .with_mvcc(&path)
            .unwrap();
        adapter.mvcc_begin();

        let read_field = |id: Uuid, field: FieldRef| {
            MvccState::open(&path)
                .unwrap()
                .state
                .with_index(|index| index.read(&(id, field), u64::MAX))
                .unwrap()
        };

        let new_id = Uuid::from_u128(50);
        assert_eq!(
            adapter.insert_record(new_id, full_fields(50)),
            Ok(InsertOutcome::Inserted)
        );
        assert_eq!(
            read_field(new_id, mvcc::EXISTENCE_FIELD),
            Some(ScanValue::Bool(true)),
            "insert_record must flush to disk immediately, not only at Compact"
        );

        let mut edited = full_fields(1);
        edited[0] = (FIELD_CONTENT, ScanValue::Str("edited".into()));
        assert_eq!(
            adapter.replace_record(Uuid::from_u128(1), edited),
            Ok(ReplaceOutcome::Replaced)
        );
        assert_eq!(
            read_field(Uuid::from_u128(1), FIELD_CONTENT),
            Some(ScanValue::Str("edited".into())),
            "replace_record must flush to disk immediately, not only at Compact"
        );

        assert_eq!(
            adapter.delete_record(Uuid::from_u128(2)),
            Ok(DeleteOutcome::Deleted)
        );
        assert_eq!(
            read_field(Uuid::from_u128(2), mvcc::EXISTENCE_FIELD),
            None,
            "delete_record's tombstone must flush to disk immediately, not only at Compact"
        );
    }

    /// See `insert_replace_delete_each_flush_mvcc_history_immediately_
    /// with_no_compact`: a non-journaled atomic `WriteBatch` must flush
    /// too, checked the same direct way.
    #[test]
    fn write_batch_atomic_non_journaled_flushes_mvcc_history_immediately() {
        let dir = fresh_temp_dir("server_memory_mvcc_batch_flush").unwrap();
        let path = dir.join("memories.mmap");
        let stack =
            create_memory_production_stack(vec![memory(1, "general", false)], &[], &path).unwrap();
        let adapter = MemoryConnectionStore::new(GenericProductionStore::new(stack))
            .with_mvcc(&path)
            .unwrap();
        adapter.mvcc_begin();

        let new_id = Uuid::from_u128(50);
        let ops = vec![WriteOp::Insert {
            id: new_id,
            fields: full_fields(50),
        }];
        assert_eq!(
            adapter.write_batch(&ops, true),
            Ok(vec![WriteResult::Inserted])
        );

        let value = MvccState::open(&path)
            .unwrap()
            .state
            .with_index(|index| index.read(&(new_id, mvcc::EXISTENCE_FIELD), u64::MAX))
            .unwrap();
        assert_eq!(
            value,
            Some(ScanValue::Bool(true)),
            "a non-journaled atomic write_batch must flush MVCC history \
             immediately, not only at Compact"
        );
    }

    /// The round-ten fix's core case: on a non-journaled table, an
    /// ordinary (non-session) `apply_transaction` commit — the only
    /// mutation path that never touches the insert log at all — used to
    /// be recorded in the in-memory index but never flushed to
    /// `<mmap_path>.mvcc` until the next `Compact`. A restart in between
    /// silently lost it from MVCC's own bookkeeping, even though a plain
    /// `GetById` read showed the correct current value. Unlike
    /// `open_with_mvcc_reconstructs_a_pending_unflushed_insert_log_entry`,
    /// there is no insert log here to fall back on — this restart only
    /// reconstructs correctly if `apply_transaction` itself flushed.
    #[test]
    fn apply_transaction_non_journaled_flushes_mvcc_history_and_a_restart_reconstructs_it() {
        let dir = fresh_temp_dir("server_memory_mvcc_txn_flush").unwrap();
        let path = dir.join("memories.mmap");
        let id = Uuid::from_u128(1);
        let stack =
            create_memory_production_stack(vec![memory(1, "general", false)], &[], &path).unwrap();
        let adapter = MemoryConnectionStore::new(GenericProductionStore::new(stack))
            .with_mvcc(&path)
            .unwrap();

        let before = adapter.mvcc_begin();
        assert_eq!(
            adapter.apply_transaction(
                &[TransactionOp {
                    id,
                    field: FIELD_ACCESS_COUNT,
                    value: ScanValue::I64(7),
                }],
                &[],
            ),
            Ok(())
        );
        // No `Compact`, no journal — the only thing that could have
        // flushed this commit to disk is `apply_transaction` itself.
        drop(adapter);

        let reopened = MemoryConnectionStore::open_with_mvcc(&path).unwrap();
        assert!(reopened.mvcc_supported());
        assert_eq!(
            reopened.mvcc_get(id, before).unwrap().unwrap()[10],
            (FIELD_ACCESS_COUNT, ScanValue::I64(0)),
            "the pre-commit snapshot still sees the pre-commit value after reopen"
        );
        let after = reopened.mvcc_begin();
        assert_eq!(
            reopened.mvcc_get(id, after).unwrap().unwrap()[10],
            (FIELD_ACCESS_COUNT, ScanValue::I64(7)),
            "a fresh snapshot after reopen sees the commit apply_transaction flushed"
        );
    }

    /// See `apply_transaction_non_journaled_flushes_mvcc_history_and_a_
    /// restart_reconstructs_it`: `apply_transaction_mvcc`'s own
    /// non-journaled path has the identical gap and fix.
    #[test]
    fn apply_transaction_mvcc_non_journaled_flushes_mvcc_history_and_a_restart_reconstructs_it() {
        let dir = fresh_temp_dir("server_memory_mvcc_txn_mvcc_flush").unwrap();
        let path = dir.join("memories.mmap");
        let id = Uuid::from_u128(1);
        let stack =
            create_memory_production_stack(vec![memory(1, "general", false)], &[], &path).unwrap();
        let adapter = MemoryConnectionStore::new(GenericProductionStore::new(stack))
            .with_mvcc(&path)
            .unwrap();

        let before = adapter.mvcc_begin();
        assert_eq!(
            adapter.apply_transaction_mvcc(
                &[TransactionOp {
                    id,
                    field: FIELD_ACCESS_COUNT,
                    value: ScanValue::I64(11),
                }],
                &[],
                before,
            ),
            Ok(())
        );
        drop(adapter);

        let reopened = MemoryConnectionStore::open_with_mvcc(&path).unwrap();
        assert_eq!(
            reopened.mvcc_get(id, before).unwrap().unwrap()[10],
            (FIELD_ACCESS_COUNT, ScanValue::I64(0)),
            "the pre-commit snapshot still sees the pre-commit value after reopen"
        );
        let after = reopened.mvcc_begin();
        assert_eq!(
            reopened.mvcc_get(id, after).unwrap().unwrap()[10],
            (FIELD_ACCESS_COUNT, ScanValue::I64(11)),
            "a fresh snapshot after reopen sees the commit apply_transaction_mvcc flushed"
        );
    }

    /// `JMR-FR-001` (ADR-0113): a journaled batch commits to the index
    /// in memory but only the journal reaches disk before the
    /// acknowledgement, so before this ADR a restart that replayed the
    /// journal into the store left the MVCC index without the batch —
    /// a fresh snapshot after reopen read the pre-commit value while a
    /// plain `GetById` read the committed one. The reopen now folds
    /// what `with_journal` replayed into the reconstructed index.
    #[test]
    fn a_journaled_batch_reaches_the_mvcc_index_after_a_restart() {
        let dir = fresh_temp_dir("server_memory_journaled_batch_mvcc_restart").unwrap();
        let path = dir.join("memories.mmap");
        let journal = dir.join("memories.journal");
        let id = Uuid::from_u128(1);
        {
            let stack =
                create_memory_production_stack(vec![memory(1, "general", false)], &[], &path)
                    .unwrap();
            let adapter =
                MemoryConnectionStore::with_journal(GenericProductionStore::new(stack), &journal)
                    .unwrap()
                    .with_mvcc(&path)
                    .unwrap();
            // Activate the index and flush its baseline, as the first
            // `Begin` of a served table does.
            let baseline = adapter.mvcc_begin();
            adapter.mvcc_release(baseline);
            assert_eq!(
                adapter.apply_transaction(
                    &[TransactionOp {
                        id,
                        field: FIELD_ACCESS_COUNT,
                        value: ScanValue::I64(5),
                    }],
                    &[],
                ),
                Ok(())
            );
            // No checkpoint: the batch is in the journal and nowhere else.
            drop(adapter);
        }

        let stack = open_memory_production_stack_portable(&path).unwrap();
        let reopened =
            MemoryConnectionStore::with_journal(GenericProductionStore::new(stack), &journal)
                .unwrap()
                .with_mvcc(&path)
                .unwrap();
        assert_eq!(
            reopened.get(id).unwrap()[FIELD_ACCESS_COUNT as usize].1,
            ScanValue::I64(5),
            "the journal replay reached the store"
        );
        let after = reopened.mvcc_begin();
        assert_eq!(
            reopened.mvcc_get(id, after).unwrap().unwrap()[10],
            (FIELD_ACCESS_COUNT, ScanValue::I64(5)),
            "a fresh snapshot after reopen sees the journaled batch"
        );
    }

    /// `JMC-FR-001`/`002` (ADR-0114): the journaled reopen. Every kind of
    /// journaled batch — an atomic `WriteBatch` (insert, replace, delete)
    /// and a `Transaction` — committed on an active index with no
    /// checkpoint in between reaches a fresh snapshot after the reopen,
    /// in journal order: the transaction's value on the replaced record
    /// wins over the replace's, the deleted record is gone, the inserted
    /// one is there. Removing the replay fold fails this test.
    #[test]
    fn open_with_mvcc_journaled_folds_every_replayed_batch_in_journal_order() {
        let dir = fresh_temp_dir("server_memory_journaled_mvcc_reopen").unwrap();
        let path = dir.join("memories.mmap");
        let journal = dir.join("memories.journal");
        let (replaced, deleted, inserted) =
            (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(10));
        let mut edited = full_fields(1);
        edited[0] = (
            FIELD_CONTENT,
            ScanValue::Str("replaced by the batch".into()),
        );
        {
            let stack = create_memory_production_stack(
                vec![memory(1, "general", false), memory(2, "general", false)],
                &[],
                &path,
            )
            .unwrap();
            let adapter =
                MemoryConnectionStore::with_journal(GenericProductionStore::new(stack), &journal)
                    .unwrap()
                    .with_mvcc(&path)
                    .unwrap();
            let baseline = adapter.mvcc_begin();
            adapter.mvcc_release(baseline);
            let ops = vec![
                WriteOp::Insert {
                    id: inserted,
                    fields: full_fields(10),
                },
                WriteOp::Replace {
                    id: replaced,
                    fields: edited.clone(),
                },
                WriteOp::Delete { id: deleted },
            ];
            assert_eq!(
                adapter.write_batch(&ops, true).unwrap(),
                vec![
                    WriteResult::Inserted,
                    WriteResult::Replaced,
                    WriteResult::Deleted
                ]
            );
            assert_eq!(
                adapter.apply_transaction(
                    &[TransactionOp {
                        id: replaced,
                        field: FIELD_ACCESS_COUNT,
                        value: ScanValue::I64(5),
                    }],
                    &[],
                ),
                Ok(())
            );
            // No checkpoint: both batches are in the journal and nowhere
            // else the index reads.
            drop(adapter);
        }

        let reopened = MemoryConnectionStore::open_with_mvcc_journaled(&path, &journal).unwrap();
        assert!(reopened.mvcc_supported());
        let after = reopened.mvcc_begin();
        assert_eq!(
            reopened.mvcc_get(inserted, after).unwrap(),
            Some(full_fields(10)),
            "the journaled insert reached the index"
        );
        assert_eq!(
            reopened.mvcc_get(deleted, after).unwrap(),
            None,
            "the journaled delete reached the index"
        );
        let mut expected = edited;
        expected[10] = (FIELD_ACCESS_COUNT, ScanValue::I64(5));
        assert_eq!(
            reopened.mvcc_get(replaced, after).unwrap(),
            Some(expected),
            "the replace and then the transaction, in journal order"
        );
        assert_eq!(
            reopened.get(replaced).unwrap()[FIELD_ACCESS_COUNT as usize].1,
            ScanValue::I64(5),
            "the store agrees with the index"
        );
    }

    /// `RGF-FR-001` (ADR-0120, the growth-line review's High 1): enabling
    /// MVCC on a table that was never MVCC-active. An ordinary insert
    /// leaves a pending insert-log entry; an in-place update then changes
    /// the record without touching the log (and, the index inactive,
    /// flushes nothing). The reopen must fold nothing into the inactive
    /// index: the first `Begin` seeds the baseline from the live store,
    /// and a fresh snapshot reads the updated value. Before the gate the
    /// pending entry was folded at txn 1, above the later baseline at
    /// txn 0, and the snapshot read the stale insert.
    #[test]
    fn enabling_mvcc_on_a_never_active_table_reads_the_live_store_not_the_pending_log() {
        let dir = fresh_temp_dir("server_memory_mvcc_first_activation").unwrap();
        let path = dir.join("memories.mmap");
        let id = Uuid::from_u128(7);
        {
            let stack = create_memory_production_stack(vec![], &[], &path).unwrap();
            let adapter = MemoryConnectionStore::new(GenericProductionStore::new(stack))
                .with_mvcc(&path)
                .unwrap();
            assert_eq!(
                adapter.insert_record(id, full_fields(7)),
                Ok(InsertOutcome::Inserted)
            );
            assert_eq!(
                adapter.update_field(id, FIELD_ACCESS_COUNT, ScanValue::I64(7)),
                Ok(true)
            );
            // Never activated: no `.mvcc`, the insert pending in the log.
            drop(adapter);
        }
        let reopened = MemoryConnectionStore::open_with_mvcc(&path).unwrap();
        let snapshot = reopened.mvcc_begin();
        assert_eq!(
            reopened.mvcc_get(id, snapshot).unwrap().unwrap()[10],
            (FIELD_ACCESS_COUNT, ScanValue::I64(7)),
            "the first snapshot after activation sees the live store's value"
        );
        assert_eq!(
            reopened.get(id).unwrap()[10],
            (FIELD_ACCESS_COUNT, ScanValue::I64(7))
        );
    }

    /// `RGM-FR-010` (ADR-0121, the review's Medium 11): an ordinary
    /// insert flushes the history with the insert log at one entry;
    /// `Compact` clears the log; a journaled `WriteBatch` insert then
    /// appends one entry. Before this round `.mvcc` still said one entry
    /// was reflected, so the reopen's pending fold skipped the new one,
    /// and the replay could not rescue it either: the reopen had folded
    /// the log into the store first, so the replayed insert was a
    /// `Duplicate` that records nothing. `Compact` now flushes again
    /// after the clear, with the count at zero; this test fails without
    /// that second flush.
    #[test]
    fn a_journaled_write_batch_after_a_compact_reaches_the_index_after_a_restart() {
        let dir = fresh_temp_dir("server_memory_journaled_write_after_compact").unwrap();
        let path = dir.join("memories.mmap");
        let journal = dir.join("memories.journal");
        let (early, late) = (Uuid::from_u128(20), Uuid::from_u128(30));
        {
            let stack =
                create_memory_production_stack(vec![memory(1, "general", false)], &[], &path)
                    .unwrap();
            let adapter =
                MemoryConnectionStore::with_journal(GenericProductionStore::new(stack), &journal)
                    .unwrap()
                    .with_mvcc(&path)
                    .unwrap();
            let baseline = adapter.mvcc_begin();
            adapter.mvcc_release(baseline);
            assert_eq!(
                adapter.insert_record(early, full_fields(20)),
                Ok(InsertOutcome::Inserted)
            );
            adapter.compact().unwrap();
            assert_eq!(
                adapter
                    .write_batch(
                        &[WriteOp::Insert {
                            id: late,
                            fields: full_fields(30),
                        }],
                        true,
                    )
                    .unwrap(),
                vec![WriteResult::Inserted]
            );
            drop(adapter);
        }
        let reopened = MemoryConnectionStore::open_with_mvcc_journaled(&path, &journal).unwrap();
        let after = reopened.mvcc_begin();
        assert_eq!(
            reopened.mvcc_get(late, after).unwrap(),
            Some(full_fields(30)),
            "the journaled insert after the compact reached the index through the replay"
        );
        assert_eq!(
            reopened.mvcc_get(early, after).unwrap(),
            Some(full_fields(20))
        );
    }

    fn full_fields_of(n: u128, category: &str, sensitive: bool) -> Vec<(FieldRef, ScanValue)> {
        MemoryConnectionStore::fields_of(memory(n, category, sensitive))
    }

    #[test]
    fn get_returns_every_field_in_tag_order_and_describe_matches() {
        let adapter = sample_adapter();
        let fields = adapter.get(Uuid::from_u128(3)).unwrap();
        assert_eq!(fields.len(), 13);
        assert_eq!(
            fields[2],
            (FIELD_TAGS, ScanValue::StrList(vec!["t".into()]))
        );
        assert_eq!(fields[9], (FIELD_SENSITIVE, ScanValue::Bool(true)));
        let schema = adapter.describe();
        assert_eq!(schema.fields.len(), 13);
        for (i, f) in schema.fields.iter().enumerate() {
            assert_eq!(f.tag as usize, i, "tags are dense and in order");
            assert_eq!(fields[i].0, f.tag);
        }
        let category = &schema.fields[FIELD_CATEGORY as usize];
        assert!(category.capabilities.filter_eq && !category.capabilities.update);
        let count = &schema.fields[FIELD_ACCESS_COUNT as usize];
        assert!(
            count.capabilities.scan && count.capabilities.update && !count.capabilities.filter_eq
        );
        assert_eq!(
            schema.fields[FIELD_CONTENT as usize].capabilities,
            FieldCapabilities {
                filter_eq: false,
                scan: false,
                update: false
            }
        );
        assert!(schema.relations.neighbors && !schema.relations.parent_children);
        assert!(adapter.get(Uuid::from_u128(99)).is_none());
    }

    #[test]
    fn filter_by_category_scan_and_update_access_count_with_the_domain_rule() {
        let adapter = sample_adapter();
        let mut general = adapter
            .filter_eq(FIELD_CATEGORY, &ScanValue::Str("general".into()))
            .unwrap();
        general.sort();
        assert_eq!(general, vec![Uuid::from_u128(1), Uuid::from_u128(3)]);
        assert_eq!(
            adapter.filter_eq(FIELD_CONTENT, &ScanValue::Str("x".into())),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            adapter.filter_eq(FIELD_CATEGORY, &ScanValue::I64(1)),
            Err(ErrorCode::Malformed)
        );
        assert_eq!(
            adapter.filter_eq(99, &ScanValue::I64(1)),
            Err(ErrorCode::UnknownField)
        );

        assert_eq!(
            adapter.update_field(Uuid::from_u128(1), FIELD_ACCESS_COUNT, ScanValue::I64(5)),
            Ok(true)
        );
        assert_eq!(
            adapter.get(Uuid::from_u128(1)).unwrap()[10],
            (FIELD_ACCESS_COUNT, ScanValue::I64(5))
        );
        assert_eq!(
            adapter.update_field(Uuid::from_u128(1), FIELD_ACCESS_COUNT, ScanValue::I64(-1)),
            Err(ErrorCode::Malformed),
            "a counter is never negative"
        );
        assert_eq!(
            adapter.update_field(Uuid::from_u128(99), FIELD_ACCESS_COUNT, ScanValue::I64(1)),
            Ok(false)
        );
        assert_eq!(
            adapter.update_field(
                Uuid::from_u128(1),
                FIELD_CONTENT,
                ScanValue::Str("x".into())
            ),
            Err(ErrorCode::Unsupported)
        );
        let mut counts = adapter.scan_field(FIELD_ACCESS_COUNT).unwrap();
        counts.sort_by_key(|v| if let ScanValue::I64(n) = v { *n } else { 0 });
        assert_eq!(
            counts,
            vec![ScanValue::I64(0), ScanValue::I64(0), ScanValue::I64(5)]
        );
        assert_eq!(adapter.scan_field(FIELD_TAGS), Err(ErrorCode::Unsupported));
    }

    #[test]
    fn insert_record_takes_all_thirteen_fields_and_refuses_every_malformed_list() {
        let adapter = sample_adapter();
        let id = Uuid::from_u128(4);
        assert_eq!(
            adapter.insert_record(id, full_fields(4)),
            Ok(InsertOutcome::Inserted)
        );
        assert_eq!(adapter.get(id).unwrap(), full_fields(4));
        assert_eq!(
            adapter.insert_record(id, full_fields(4)),
            Ok(InsertOutcome::Duplicate)
        );
        let mut negative = full_fields(5);
        negative[10] = (FIELD_ACCESS_COUNT, ScanValue::I64(-3));
        assert_eq!(
            adapter.insert_record(Uuid::from_u128(5), negative),
            Err(ErrorCode::Malformed)
        );
        let mut wrong_tags = full_fields(5);
        wrong_tags[2] = (FIELD_TAGS, ScanValue::Str("t".into()));
        assert_eq!(
            adapter.insert_record(Uuid::from_u128(5), wrong_tags),
            Err(ErrorCode::Malformed)
        );
        assert_eq!(
            adapter.insert_record(Uuid::from_u128(5), full_fields(5)[..12].to_vec()),
            Err(ErrorCode::Malformed)
        );
        let mut extra = full_fields(5);
        extra.push((42, ScanValue::U32(0)));
        assert_eq!(
            adapter.insert_record(Uuid::from_u128(5), extra),
            Err(ErrorCode::UnknownField)
        );
        assert!(
            adapter.get(Uuid::from_u128(5)).is_none(),
            "nothing inserted"
        );
    }

    #[test]
    fn mentions_is_the_one_relation_foreign_to_entity_and_links_without_seeing_the_far_end() {
        let adapter = sample_adapter();
        let (one, ada) = (Uuid::from_u128(1), Uuid::from_u128(0xada));
        assert_eq!(adapter.parent(one), Err(ErrorCode::Unsupported));
        assert_eq!(adapter.children(one), Err(ErrorCode::Unsupported));
        assert!(adapter.describe().relations.neighbors);
        assert_eq!(adapter.table_name(), "memory");
        assert_eq!(
            adapter.describe_relations(),
            vec![RelationDescriptor {
                name: "mentions".into(),
                kind: JoinRelation::Neighbors(Some("mentions".into())),
                target_table: Some("entity".into()),
            }]
        );
        assert_eq!(adapter.list_relation_kinds(), vec!["mentions".to_string()]);
        assert_eq!(adapter.neighbors(one), Ok(vec![]));
        // The far end is an entity this adapter never sees (`TBL-FR-007`).
        assert_eq!(
            adapter.link_records(one, ada, "mentions"),
            Ok(LinkOutcome::Linked)
        );
        assert_eq!(
            adapter.link_records(one, ada, "mentions"),
            Ok(LinkOutcome::AlreadyLinked)
        );
        assert_eq!(
            adapter.neighbors_by_relation(one, "mentions"),
            Ok(vec![ada])
        );
        assert_eq!(
            adapter.neighbors(ada),
            Ok(vec![one]),
            "read from the entity's side"
        );
        assert_eq!(
            adapter.link_records(one, ada, "x"),
            Err(ErrorCode::Malformed),
            "a fixed label set"
        );
        assert_eq!(
            adapter.link_records(Uuid::from_u128(77), ada, "mentions"),
            Err(ErrorCode::RecordNotFound),
            "the near end must be a memory"
        );
        assert_eq!(
            adapter.neighbors_by_relation(one, "x"),
            Err(ErrorCode::Malformed)
        );
    }

    /// `REP-FR-005` (ADR-0049): the same validation as `insert_record`,
    /// then the whole record replaced — every read sees the new version
    /// at once, the old category bucket no longer lists the id; an
    /// unknown id is `NotFound` with nothing written; a malformed list
    /// is refused before any write.
    #[test]
    fn replace_record_validates_like_insert_then_swaps_the_whole_record() {
        let adapter = sample_adapter();
        let id = Uuid::from_u128(1);
        let mut edited = full_fields(1);
        edited[0] = (FIELD_CONTENT, ScanValue::Str("memory 1, revised".into()));
        edited[1] = (FIELD_CATEGORY, ScanValue::Str("decision".into()));
        edited[10] = (FIELD_ACCESS_COUNT, ScanValue::I64(5));
        assert_eq!(
            adapter.replace_record(id, edited.clone()),
            Ok(ReplaceOutcome::Replaced)
        );
        assert_eq!(adapter.get(id).unwrap(), edited);
        assert_eq!(
            adapter.filter_eq(FIELD_CATEGORY, &ScanValue::Str("decision".into())),
            Ok(vec![id])
        );
        assert!(!adapter
            .filter_eq(FIELD_CATEGORY, &ScanValue::Str("general".into()))
            .unwrap()
            .contains(&id));
        assert_eq!(
            adapter.replace_record(Uuid::from_u128(9), full_fields(9)),
            Ok(ReplaceOutcome::NotFound)
        );
        assert!(adapter.get(Uuid::from_u128(9)).is_none());
        let mut negative = full_fields(1);
        negative[10] = (FIELD_ACCESS_COUNT, ScanValue::I64(-1));
        assert_eq!(
            adapter.replace_record(id, negative),
            Err(ErrorCode::Malformed)
        );
        assert_eq!(
            adapter.replace_record(id, full_fields(1)[..12].to_vec()),
            Err(ErrorCode::Malformed)
        );
        assert_eq!(
            adapter.get(id).unwrap(),
            edited,
            "nothing written on refusal"
        );
        assert_eq!(adapter.scan_all().len(), 3, "no new record");
    }

    /// `DEL-FR-006`/`005` (ADR-0051): a deleted memory is gone from every
    /// read and its `mentions` edge with it; a repeat is `NotFound`;
    /// `detach_record` for an entity id drops every memory's edge to it
    /// and refuses a label the domain lacks.
    #[test]
    fn delete_record_and_detach_record_drop_the_memory_and_its_edges() {
        let adapter = sample_adapter();
        let (one, two, ada) = (
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(0xada),
        );
        adapter.link_records(one, ada, "mentions").unwrap();
        adapter.link_records(two, ada, "mentions").unwrap();
        assert_eq!(adapter.delete_record(one), Ok(DeleteOutcome::Deleted));
        assert!(adapter.get(one).is_none());
        assert_eq!(adapter.scan_all().len(), 2);
        assert!(!adapter
            .filter_eq(FIELD_CATEGORY, &ScanValue::Str("general".into()))
            .unwrap()
            .contains(&one));
        assert_eq!(
            adapter.neighbors_by_relation(ada, "mentions"),
            Ok(vec![two])
        );
        assert_eq!(adapter.delete_record(one), Ok(DeleteOutcome::NotFound));
        assert_eq!(adapter.detach_record("mentions", ada), Ok(1));
        assert_eq!(adapter.neighbors_by_relation(two, "mentions"), Ok(vec![]));
        assert!(adapter.get(two).is_some());
        assert_eq!(adapter.detach_record("mentions", ada), Ok(0));
        assert_eq!(adapter.detach_record("x", ada), Err(ErrorCode::Malformed));
    }

    /// `CMP-FR-006` (ADR-0052): the adapter compacts its stack and reports
    /// it; every read afterwards is what it was.
    #[test]
    fn compact_reports_what_it_reclaimed_and_changes_no_read() {
        let adapter = sample_adapter();
        let (one, ada) = (Uuid::from_u128(1), Uuid::from_u128(0xada));
        adapter.link_records(one, ada, "mentions").unwrap();
        assert_eq!(
            adapter.delete_record(Uuid::from_u128(3)),
            Ok(DeleteOutcome::Deleted)
        );
        let before = adapter.scan_all();
        let report = adapter.compact().unwrap();
        assert_eq!(report.records, 2);
        assert_eq!(report.slots_reclaimed, 1);
        assert_eq!(report.log_entries_folded, 1);
        assert_eq!(report.edge_logs_folded, 1);
        assert_eq!(adapter.scan_all(), before);
        assert_eq!(
            adapter.neighbors_by_relation(one, "mentions"),
            Ok(vec![ada])
        );
    }

    /// `GRD-FR-003` (ADR-0054): the guard is evaluated over this adapter's
    /// own wire shape of the stored record — `updated_at < mine` holds
    /// for a newer version and replaces, fails for an older one with
    /// nothing written; an unknown id is `NotFound`; `replace_record`'s
    /// validation still runs first.
    #[test]
    fn replace_record_if_is_last_writer_wins_over_the_stored_updated_at() {
        use crate::server::protocol::{CompareOp, Predicate};
        let adapter = sample_adapter();
        let id = Uuid::from_u128(1);
        let stored_updated_at = 1_000;
        let mut newer = full_fields(1);
        newer[0] = (FIELD_CONTENT, ScanValue::Str("newer".into()));
        newer[6] = (FIELD_UPDATED_AT, ScanValue::I64(5_000));
        let guard = |mine: i64| Predicate {
            field: FIELD_UPDATED_AT,
            op: CompareOp::Lt,
            value: ScanValue::I64(mine),
        };
        assert_eq!(
            adapter.replace_record_if(id, newer.clone(), &guard(5_000)),
            Ok(ReplaceIfOutcome::Replaced)
        );
        assert_eq!(adapter.get(id).unwrap(), newer);

        let mut older = newer.clone();
        older[0] = (FIELD_CONTENT, ScanValue::Str("older".into()));
        older[6] = (FIELD_UPDATED_AT, ScanValue::I64(stored_updated_at));
        assert_eq!(
            adapter.replace_record_if(id, older, &guard(stored_updated_at)),
            Ok(ReplaceIfOutcome::GuardFailed)
        );
        assert_eq!(adapter.get(id).unwrap(), newer, "nothing written");

        assert_eq!(
            adapter.replace_record_if(Uuid::from_u128(99), newer.clone(), &guard(9_000)),
            Ok(ReplaceIfOutcome::NotFound)
        );
        let mut negative = newer.clone();
        negative[10] = (FIELD_ACCESS_COUNT, ScanValue::I64(-1));
        assert_eq!(
            adapter.replace_record_if(id, negative, &guard(9_000)),
            Err(ErrorCode::Malformed)
        );
        assert_eq!(adapter.get(id).unwrap(), newer);
    }

    /// `WBJ-FR-005` (ADR-0063): a journaled adapter's atomic `write_batch`
    /// survives a crash between the journal `fsync` and the apply —
    /// simulated the same way `dog.rs`'s own journal crash-recovery test
    /// is: a genuinely committed batch's durable journal, replayed onto
    /// a fresh copy of the pre-batch store files (the state a crash
    /// right after the `fsync` but before any apply would leave behind).
    #[test]
    fn write_batch_atomic_is_crash_atomic_via_the_journal() {
        let dir = fresh_temp_dir("server_memory_write_batch_journal").unwrap();
        let seed = || vec![memory(1, "general", false), memory(2, "preference", false)];
        let journal = dir.join("wbt.journal");
        let header_len = 12;

        let path_a = dir.join("a.mmap");
        let stack_a = create_memory_production_stack(seed(), &[], &path_a).unwrap();
        let adapter =
            MemoryConnectionStore::with_journal(GenericProductionStore::new(stack_a), &journal)
                .unwrap();
        assert_eq!(std::fs::metadata(&journal).unwrap().len(), header_len);

        let ops = vec![
            WriteOp::Insert {
                id: Uuid::from_u128(10),
                fields: full_fields(10),
            },
            WriteOp::Replace {
                id: Uuid::from_u128(1),
                fields: full_fields(1),
            },
        ];
        let results = adapter.write_batch(&ops, true).unwrap();
        assert_eq!(results, vec![WriteResult::Inserted, WriteResult::Replaced]);
        let after_ok = std::fs::metadata(&journal).unwrap().len();
        assert!(after_ok > header_len, "the batch is journaled");
        drop(adapter);

        // Fresh, pre-batch store files + the durable journal: replay
        // applies both ops, exactly as a real crash-then-restart would.
        let path_b = dir.join("b.mmap");
        let stack_b = create_memory_production_stack(seed(), &[], &path_b).unwrap();
        let replayed =
            MemoryConnectionStore::with_journal(GenericProductionStore::new(stack_b), &journal)
                .unwrap();
        assert!(
            replayed.get(Uuid::from_u128(10)).is_some(),
            "the insert landed"
        );
        assert_eq!(
            replayed.get(Uuid::from_u128(1)).unwrap(),
            full_fields(1),
            "the replace landed"
        );
        assert_eq!(
            std::fs::metadata(&journal).unwrap().len(),
            header_len,
            "checkpointed after replay"
        );
    }
}
