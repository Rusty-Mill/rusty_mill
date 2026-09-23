//! [`ConnectionStore`] adapter wrapping
//! [`crate::generic::production::GenericProductionStore<EntityProductionStack>`]
//! for `Entity` v2 — `ENT2-FR-006`/`007`, ADR-0039, `server`-gated
//! alone, matching `Reminder`'s own front-door precedent.
//!
//! # Two relation labels, not one
//!
//! `neighbors` (unfiltered) answers the union of both `relates_to`/
//! `mentioned_with`; `neighbors_by_relation`/`list_relation_kinds` are
//! real for the first time in this crate — see
//! [`crate::generic::store::MultiSymmetric`]'s own doc comment for the
//! mechanism.
//!
//! # `kind`, not a plain number, is the equality-filterable field
//!
//! `filter_eq` on `kind` accepts any string now (open-ended, `ENT2-FR-001`)
//! — no discriminant validation, unlike v1's fixed-enum `kind_from_u32`
//! check. `kind` is **not** durably updatable over the wire (unlike v1) —
//! see `crate::generic::entity`'s own module doc for why (`ScannableField::
//! ScanValue: Copy`, and `String` is neither `Copy` nor mmap-fixed-width).
//! `mention_count` fills that role instead, kept unchanged from v1.
//!
//! # `label` is equality-filterable too — through a different index
//!
//! `ENT3-FR-005`/`006` (ADR-0040): `filter_eq` on `label` resolves the
//! query against the stack's `NameIndex` layer, not `GenericMmapStore`'s
//! own `IndexedField` slot (`kind` holds that). It matches `label` *or
//! any alias*, case- and whitespace-insensitively — normalization is the
//! store's, so the raw wire string is passed straight through. Zero, one,
//! or many ids; a miss is `Ok(vec![])`, never an error. Reuses
//! `Request::FilterEq`/`ScanValue::Str`/`Response::RecordList` exactly as
//! they are — **no `PROTOCOL_VERSION` change**; the only thing a client
//! sees differently is `DomainSchema` now reporting `filter_eq: true` for
//! `label`, a data value, not a shape.
//!
//! # `aliases` is readable — protocol 11 — and nothing else
//!
//! `ENT4-FR-002` (ADR-0041): `aliases` has `FIELD_ALIASES = 3`, a
//! `FieldDescriptor` with every capability flag `false`, and rides in
//! `get`/`scan_all` as `ScanValue::StrList` — the raw stored `Vec<String>`
//! in stored order, un-normalized (the `NameIndex` keys are derived from
//! it, never the reverse). Every write/filter/scan path on it is
//! `Unsupported` — a known field that supports nothing, not `UnknownField`.
//! A connection negotiated below 11 never sees the field at all:
//! `downgrade_for_version` in `super` strips the pair from `Record`/`Rows`
//! and the descriptor from `Schema` (rule 3, `ENT4-FR-003`), leaving
//! exactly the three-field shape `FR-042` returned.

use super::journal::{
    CheckpointFlush, CommitError, CommitGroup, JournalError, JournaledBatch, ReplayedBatch,
};
use super::mvcc::{self, MvccIndex, MvccState, TxnId};
use super::protocol::{
    DomainSchema, ErrorCode, FieldCapabilities, FieldDescriptor, FieldRef, ParentLookup, Predicate,
    RecordId, RelationCapabilities, ScanValue, TransactionOp, ValueKind, WriteOp, WriteResult,
};
use super::{
    copy_table_files, page_key, predicate_matches, read_table_files, validate_predicate,
    BackupReport, ConnectionStore, DeleteOutcome, InsertOutcome, LinkOutcome, ReadTableFilesError,
    ReplaceIfOutcome, ReplaceOutcome,
};
use crate::durability::DurabilityError;
use crate::generic::entity::{
    open_entity_production_stack_portable, Entity, EntityProductionStack, KindField,
    MentionCountField,
};
use crate::generic::insert_log::{self, LogEntry};
use crate::generic::production::GenericProductionStore;
use crate::generic::query::{AllIds, Delete, GetById, Insert, MultiLink, Replace, UpdateField};
use crate::generic::store::valid_relation_label;
use crate::generic::traits::SchemaTag;
use crate::generic::{DeleteError, InsertError, LinkError, ReplaceError};
use std::path::{Path, PathBuf};

pub const FIELD_LABEL: FieldRef = 0;
pub const FIELD_KIND: FieldRef = 1;
pub const FIELD_MENTION_COUNT: FieldRef = 2;
/// `ENT4-FR-002` (ADR-0041, protocol 11): read-only; see module docs.
pub const FIELD_ALIASES: FieldRef = 3;

/// `ADR-0072`'s `MVCC2-FR-001`/`010`: an active table's MVCC state plus
/// the `mmap_path` [`MvccState::flush`] needs to name `<mmap_path>.mvcc`.
struct MvccHandle {
    state: MvccState,
    mmap_path: PathBuf,
}

/// One write an atomic batch made, for the MVCC record: `None` fields is
/// a deletion.
type RecordedWrite = (RecordId, Option<Vec<(FieldRef, ScanValue)>>);

pub struct EntityConnectionStore {
    store: GenericProductionStore<EntityProductionStack>,
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
    /// `ADR-0072`'s `MVCC2-FR-001` — see `MemoryConnectionStore`'s own
    /// `mvcc` field for the full contract; identical here.
    mvcc: Option<MvccHandle>,
}

impl EntityConnectionStore {
    pub fn new(store: GenericProductionStore<EntityProductionStack>) -> Self {
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

    /// `ADR-0072`'s `MVCC2-FR-001`/`003` — see
    /// `MemoryConnectionStore::with_mvcc` for the full contract
    /// (including its own documented restart-recovery gap); identical
    /// here.
    pub fn with_mvcc(self, mmap_path: &Path) -> Result<Self, DurabilityError> {
        let log = insert_log::log_path(mmap_path);
        let entries = insert_log::read_entries(&log, Entity::SCHEMA_TAG)?;
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
        let pending_entries = insert_log::read_entries(&log, Entity::SCHEMA_TAG)?;
        let stack = open_entity_production_stack_portable(path)?;
        Self::with_journal(GenericProductionStore::new(stack), journal_path)?
            .attach_mvcc(path, pending_entries)
            .map_err(JournalError::from)
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
        pending: Vec<LogEntry<Entity, RecordId>>,
    ) -> Result<Self, DurabilityError> {
        let reconstructed = MvccState::open(mmap_path)?;
        let folded_pending = Self::fold_pending_log_entries(&reconstructed, pending);
        reconstructed
            .state
            .set_reclaim_every(self.mvcc_reclaim_every);
        let replayed = std::mem::take(&mut self.replayed);
        let fold_replayed = reconstructed.state.is_active() && !replayed.is_empty();
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
        if folded_pending > 0 || fold_replayed {
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
    /// the *reopen* counterpart to [`Self::with_mvcc`] — see
    /// `MemoryConnectionStore::open_with_mvcc` for the full contract;
    /// identical here.
    ///
    /// # Errors
    ///
    /// Returns [`DurabilityError`] if `<mmap_path>.mvcc` exists but can't
    /// be read/decoded, if the insert log can't be read, or if the
    /// underlying reopen (`open_entity_production_stack_portable`) fails.
    pub fn open_with_mvcc(path: &Path) -> Result<Self, DurabilityError> {
        // Read the pending log first — before the reopen below clears it.
        let log = insert_log::log_path(path);
        let pending_entries = insert_log::read_entries(&log, Entity::SCHEMA_TAG)?;
        let stack = open_entity_production_stack_portable(path)?;
        Self::new(GenericProductionStore::new(stack)).attach_mvcc(path, pending_entries)
    }

    /// Shared by [`Self::with_mvcc`] and [`Self::open_with_mvcc`] — see
    /// `MemoryConnectionStore::fold_pending_log_entries` for the full
    /// contract; identical here.
    fn fold_pending_log_entries(
        reconstructed: &mvcc::Reconstructed,
        entries: Vec<LogEntry<Entity, RecordId>>,
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
        store: GenericProductionStore<EntityProductionStack>,
        journal_path: &Path,
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
            journal.truncate()
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

    /// `WBJ-FR-003` (ADR-0063) — see `MemoryConnectionStore::
    /// replay_write_batch` for the full contract; identical here.
    fn replay_write_batch(
        inner: &mut EntityProductionStack,
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
    /// `with_journal`'s replay. `ConnectionStore::describe` delegates
    /// here.
    fn schema() -> DomainSchema {
        DomainSchema {
            fields: vec![
                FieldDescriptor {
                    tag: FIELD_LABEL,
                    name: "label".into(),
                    value_kind: ValueKind::Str,
                    capabilities: FieldCapabilities {
                        filter_eq: true,
                        scan: false,
                        update: false,
                    },
                },
                FieldDescriptor {
                    tag: FIELD_KIND,
                    name: "kind".into(),
                    value_kind: ValueKind::Str,
                    capabilities: FieldCapabilities {
                        filter_eq: true,
                        scan: false,
                        update: false,
                    },
                },
                FieldDescriptor {
                    tag: FIELD_MENTION_COUNT,
                    name: "mention_count".into(),
                    value_kind: ValueKind::I64,
                    capabilities: FieldCapabilities {
                        filter_eq: false,
                        scan: true,
                        update: true,
                    },
                },
                FieldDescriptor {
                    tag: FIELD_ALIASES,
                    name: "aliases".into(),
                    value_kind: ValueKind::StrList,
                    capabilities: FieldCapabilities {
                        filter_eq: false,
                        scan: false,
                        update: false,
                    },
                },
            ],
            relations: RelationCapabilities {
                parent_children: false,
                neighbors: true,
            },
        }
    }

    /// `Entity`'s only mutable field over this protocol is
    /// `mention_count` — `kind` moved to read-only in v2 (see module
    /// docs).
    fn validate_batch(
        updates: &[TransactionOp],
        exists: impl Fn(RecordId) -> bool,
    ) -> Result<(), (usize, ErrorCode)> {
        for (i, op) in updates.iter().enumerate() {
            match (op.field, &op.value) {
                (FIELD_MENTION_COUNT, ScanValue::I64(_)) => {
                    if !exists(op.id) {
                        return Err((i, ErrorCode::RecordNotFound));
                    }
                }
                (FIELD_MENTION_COUNT, _) => return Err((i, ErrorCode::Malformed)),
                (FIELD_LABEL | FIELD_KIND | FIELD_ALIASES, _) => {
                    return Err((i, ErrorCode::Unsupported))
                }
                _ => return Err((i, ErrorCode::UnknownField)),
            }
        }
        Ok(())
    }

    fn apply_batch(
        inner: &mut EntityProductionStack,
        updates: &[TransactionOp],
    ) -> Result<(), (usize, ErrorCode)> {
        for (i, op) in updates.iter().enumerate() {
            if let ScanValue::I64(mention_count) = op.value {
                UpdateField::<Entity, MentionCountField>::update(inner, op.id, mention_count)
                    .map_err(|_| (i, ErrorCode::RecordNotFound))?;
            }
        }
        Ok(())
    }

    /// `INS-FR-006` (ADR-0046): the whole field list against this
    /// domain's schema, before any write — all four tags exactly once
    /// with a value of its kind, `aliases` as a `StrList` (the first
    /// *write* of one: `ENT4-FR-003`'s read-only rule describes
    /// `UpdateField`, not a whole record set at creation — the blob and
    /// the insert log carry `Vec<Entity>` whole, so aliases are durable
    /// on the same terms as at `create`). `Malformed` for a missing,
    /// repeated, or wrong-kind field; `UnknownField` for a tag this
    /// domain doesn't have. Relations: none — an inserted entity has no
    /// neighbors under either label until the relation-insertion round.
    fn entity_from_fields(
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<Entity, ErrorCode> {
        let mut label = None;
        let mut kind = None;
        let mut mention_count = None;
        let mut aliases = None;
        for (tag, value) in fields {
            match (tag, value) {
                (FIELD_LABEL, ScanValue::Str(v)) if label.is_none() => label = Some(v),
                (FIELD_KIND, ScanValue::Str(v)) if kind.is_none() => kind = Some(v),
                (FIELD_MENTION_COUNT, ScanValue::I64(v)) if mention_count.is_none() => {
                    mention_count = Some(v)
                }
                (FIELD_ALIASES, ScanValue::StrList(v)) if aliases.is_none() => aliases = Some(v),
                (FIELD_LABEL | FIELD_KIND | FIELD_MENTION_COUNT | FIELD_ALIASES, _) => {
                    return Err(ErrorCode::Malformed)
                }
                _ => return Err(ErrorCode::UnknownField),
            }
        }
        let (Some(label), Some(kind), Some(mention_count), Some(aliases)) =
            (label, kind, mention_count, aliases)
        else {
            return Err(ErrorCode::Malformed);
        };
        Ok(Entity {
            id,
            label,
            kind,
            mention_count,
            aliases,
        })
    }

    /// `ISO-FR-002`/`ISO-FR-006` — see `DogConnectionStore::check_read_set`
    /// for the full contract; identical shape here.
    fn check_read_set(
        reads: &[(RecordId, FieldRef, ScanValue)],
        get: impl Fn(RecordId) -> Option<Entity>,
    ) -> Result<(), (usize, ErrorCode)> {
        for (id, field, value) in reads {
            let current = get(*id).and_then(|entity| match *field {
                FIELD_LABEL => Some(ScanValue::Str(entity.label)),
                FIELD_KIND => Some(ScanValue::Str(entity.kind)),
                FIELD_MENTION_COUNT => Some(ScanValue::I64(entity.mention_count)),
                FIELD_ALIASES => Some(ScanValue::StrList(entity.aliases)),
                _ => None,
            });
            if current.as_ref() != Some(value) {
                return Err((0, ErrorCode::Conflict));
            }
        }
        Ok(())
    }

    /// `ADR-0072`'s `MVCC2-FR-008` — see `MemoryConnectionStore::
    /// mvcc_record_write` for the full contract; identical here.
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

    /// `ADR-0072`'s `MVCC2-FR-008` — see `MemoryConnectionStore::
    /// mvcc_record_delete` for the full contract; identical here.
    fn mvcc_record_delete(&self, id: RecordId) {
        let Some(mvcc) = &self.mvcc else { return };
        if !mvcc.state.is_active() {
            return;
        }
        let txn_id = mvcc.state.counter().next();
        mvcc.state
            .with_index(|index| index.record_write((id, mvcc::EXISTENCE_FIELD), txn_id, None));
    }

    /// `ADR-0072`'s `MVCC2-FR-008` — see `MemoryConnectionStore::
    /// mvcc_record_transaction` for the full contract; identical here.
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

    /// `ADR-0072`'s `MVCC2-FR-002`/`008` — see `MemoryConnectionStore::
    /// mvcc_record_write_ops` for the full contract; identical here.
    /// `Link` touches no field [`ConnectionStore::mvcc_get`] ever reads,
    /// so it is never recorded.
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

    /// `ADR-0072`'s `MVCC2-FR-010` — see `MemoryConnectionStore::
    /// mvcc_flush_now` for the full contract; identical here.
    fn mvcc_flush_now(&self, journal_entries: u64) -> bool {
        let Some(mvcc) = &self.mvcc else { return true };
        if !mvcc.state.is_active() {
            return true;
        }
        let insert_log_count = insert_log::read_entries::<Entity, RecordId>(
            &insert_log::log_path(&mvcc.mmap_path),
            Entity::SCHEMA_TAG,
        )
        .map(|entries| entries.len())
        .unwrap_or(0);
        mvcc.state
            .flush(&mvcc.mmap_path, insert_log_count, journal_entries as usize)
            .is_ok()
    }
}

impl EntityConnectionStore {
    /// The wire shape of one entity, in tag order — `get` and a
    /// `ReplaceIf` guard's evaluation both go through here.
    fn fields_of(entity: Entity) -> Vec<(FieldRef, ScanValue)> {
        vec![
            (FIELD_LABEL, ScanValue::Str(entity.label)),
            (FIELD_KIND, ScanValue::Str(entity.kind)),
            (FIELD_MENTION_COUNT, ScanValue::I64(entity.mention_count)),
            // `ENT4-FR-002`: raw, stored order, un-normalized.
            (FIELD_ALIASES, ScanValue::StrList(entity.aliases)),
        ]
    }
}

/// `WBT-FR-003` (ADR-0060): a parsed, pre-validated write of an atomic
/// [`WriteOp`] batch on `Entity`.
enum PreparedWrite {
    Insert(Entity),
    Replace(Entity),
    ReplaceIf(Entity, Predicate),
    Delete(RecordId),
    Link {
        left: RecordId,
        right: RecordId,
        relation: String,
    },
}

impl EntityConnectionStore {
    fn prepare_write(schema: &DomainSchema, op: &WriteOp) -> Result<PreparedWrite, ErrorCode> {
        Ok(match op {
            WriteOp::Insert { id, fields } => {
                PreparedWrite::Insert(Self::entity_from_fields(*id, fields.clone())?)
            }
            WriteOp::Replace { id, fields } => {
                PreparedWrite::Replace(Self::entity_from_fields(*id, fields.clone())?)
            }
            WriteOp::ReplaceIf { id, fields, guard } => {
                validate_predicate(schema, guard)?;
                PreparedWrite::ReplaceIf(
                    Self::entity_from_fields(*id, fields.clone())?,
                    guard.clone(),
                )
            }
            WriteOp::Delete { id } => PreparedWrite::Delete(*id),
            WriteOp::Link {
                left,
                right,
                relation,
            } => {
                if !valid_relation_label(relation) {
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

    fn apply_prepared(
        inner: &mut EntityProductionStack,
        prepared: PreparedWrite,
    ) -> Result<WriteResult, ErrorCode> {
        Ok(match prepared {
            PreparedWrite::Insert(entity) => match Insert::insert(inner, entity) {
                Ok(()) => WriteResult::Inserted,
                Err(InsertError::Duplicate(_)) => WriteResult::Duplicate,
                Err(InsertError::Durability(_)) => return Err(ErrorCode::Storage),
            },
            PreparedWrite::Replace(entity) => match Replace::replace(inner, entity) {
                Ok(()) => WriteResult::Replaced,
                Err(ReplaceError::NotFound(_)) => WriteResult::NotFound,
                Err(ReplaceError::Durability(_)) => return Err(ErrorCode::Storage),
            },
            PreparedWrite::ReplaceIf(entity, guard) => {
                let id = entity.id;
                match GetById::<Entity>::get(inner, id) {
                    None => WriteResult::NotFound,
                    Some(stored) => {
                        if predicate_matches(&Self::fields_of(stored), &guard) {
                            match Replace::replace(inner, entity) {
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
            PreparedWrite::Delete(id) => match Delete::<Entity>::delete(inner, id) {
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

impl ConnectionStore for EntityConnectionStore {
    fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>> {
        self.store.get::<Entity>(id).map(Self::fields_of)
    }

    /// `SQL-FR-004`/`SQL-FR-005` (ADR-0034): every id from `all_ids`,
    /// each mapped through this adapter's own `get`.
    fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
        self.store
            .all_ids::<Entity>()
            .into_iter()
            .filter_map(|id| self.get(id).map(|fields| (id, fields)))
            .collect()
    }

    /// `PAG-FR-002` (ADR-0055): the sort key of every record read
    /// straight off [`Entity`], so a page materializes only its own rows.
    /// A field this arm list does not name falls back to the wire shape
    /// — the same key [`page_key`] derives for the trait default.
    fn page_keys(&self, order_by: FieldRef) -> Vec<(RecordId, i128)> {
        self.store
            .all_ids::<Entity>()
            .into_iter()
            .filter_map(|id| {
                let record = self.store.get::<Entity>(id)?;
                let key = match order_by {
                    FIELD_MENTION_COUNT => Some(i128::from(record.mention_count)),
                    _ => None,
                };
                let key = key.unwrap_or_else(|| page_key(&Self::fields_of(record), order_by, id).0);
                Some((id, key))
            })
            .collect()
    }

    fn filter_eq(&self, field: FieldRef, value: &ScanValue) -> Result<Vec<RecordId>, ErrorCode> {
        match (field, value) {
            (FIELD_KIND, ScanValue::Str(kind)) => {
                Ok(self.store.filter_eq::<Entity, KindField>(kind))
            }
            (FIELD_KIND, _) => Err(ErrorCode::Malformed),
            // `ENT3-FR-005`: `label` or any alias, normalized by the store.
            (FIELD_LABEL, ScanValue::Str(name)) => Ok(self.store.find_by_name::<Entity>(name)),
            (FIELD_LABEL, _) => Err(ErrorCode::Malformed),
            (FIELD_MENTION_COUNT | FIELD_ALIASES, _) => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    fn scan_field(&self, field: FieldRef) -> Result<Vec<ScanValue>, ErrorCode> {
        match field {
            FIELD_MENTION_COUNT => Ok(self
                .store
                .scan::<Entity, MentionCountField>()
                .into_iter()
                .map(ScanValue::I64)
                .collect()),
            FIELD_LABEL | FIELD_KIND | FIELD_ALIASES => Err(ErrorCode::Unsupported),
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
            (FIELD_MENTION_COUNT, ScanValue::I64(mention_count)) => {
                if self.journals_updates() {
                    return self.journaled_update(TransactionOp {
                        id,
                        field: FIELD_MENTION_COUNT,
                        value: ScanValue::I64(mention_count),
                    });
                }
                // `MUR-FR-001` (ADR-0108): the in-place write and its MVCC
                // record in one exclusive section, as a batch's are.
                let written = self.store.with_exclusive(|inner| {
                    let written =
                        UpdateField::<Entity, MentionCountField>::update(inner, id, mention_count)
                            .is_ok();
                    if written {
                        self.mvcc_record_transaction(&[TransactionOp {
                            id,
                            field: FIELD_MENTION_COUNT,
                            value: ScanValue::I64(mention_count),
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
            (FIELD_MENTION_COUNT, _) => Err(ErrorCode::Malformed),
            (FIELD_LABEL | FIELD_KIND | FIELD_ALIASES, _) => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    /// `INS-FR-006` (ADR-0046): validate, then one write under the
    /// store's own lock — through `NameIndex` (its keys added) and
    /// `MultiSymmetric` (no edges) down to the durable core. A duplicate
    /// is the normal outcome; a durability failure is `Storage`.
    fn insert_record(
        &self,
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<InsertOutcome, ErrorCode> {
        let entity = Self::entity_from_fields(id, fields)?;
        self.store
            .with_exclusive(|inner| match Insert::insert(inner, entity) {
                Ok(()) => {
                    self.mvcc_record_write(
                        &Self::fields_of(GetById::<Entity>::get(inner, id).expect("just inserted")),
                        id,
                    );
                    // `ADR-0072`'s `MVCC2-FR-010`: a non-journaled table has
                    // no checkpoint boundary, so every MVCC-recording write
                    // flushes immediately — see `MemoryConnectionStore::
                    // insert_record`.
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
    /// then one whole-record write under the store's own lock — the
    /// name index follows a changed `label`/`aliases`, every relation
    /// edge survives. An unknown id is the normal outcome, not an error.
    fn replace_record(
        &self,
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<ReplaceOutcome, ErrorCode> {
        let entity = Self::entity_from_fields(id, fields)?;
        self.store
            .with_exclusive(|inner| match Replace::replace(inner, entity) {
                Ok(()) => {
                    self.mvcc_record_write(
                        &Self::fields_of(GetById::<Entity>::get(inner, id).expect("just replaced")),
                        id,
                    );
                    // See `insert_record`: flush on every MVCC-recording
                    // write, not just at `Compact`.
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
        let entity = Self::entity_from_fields(id, fields)?;
        self.store.with_exclusive(|inner| {
            let Some(current) = GetById::<Entity>::get(inner, id) else {
                return Ok(ReplaceIfOutcome::NotFound);
            };
            if !predicate_matches(&Self::fields_of(current), guard) {
                return Ok(ReplaceIfOutcome::GuardFailed);
            }
            match Replace::replace(inner, entity) {
                Ok(()) => {
                    self.mvcc_record_write(
                        &Self::fields_of(GetById::<Entity>::get(inner, id).expect("just replaced")),
                        id,
                    );
                    // See `insert_record`: flush on every MVCC-recording
                    // write, not just at `Compact`.
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
            .with_exclusive(|inner| match Delete::<Entity>::delete(inner, id) {
                Ok(()) => {
                    self.mvcc_record_delete(id);
                    // See `insert_record`: flush on every MVCC-recording
                    // write, not just at `Compact`.
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
            // `ADR-0072`'s `MVCC2-FR-010` item 2 — see
            // `MemoryConnectionStore::compact` for the full contract;
            // identical here.
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
            crate::generic::query::Compact::compact(inner).map_err(|_| ErrorCode::Storage)
        })
    }

    /// `BAK-FR-002`/`006` (ADR-0065) — see
    /// `MemoryConnectionStore::backup` for the full contract; identical
    /// here.
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

    /// `LNK-FR-009` (ADR-0047): open labels — any valid label is
    /// accepted and created at first use by the `MultiSymmetric` layer
    /// beneath; `ListRelationKinds`/`DescribeRelations` list it at once.
    fn link_records(
        &self,
        left: RecordId,
        right: RecordId,
        relation: &str,
    ) -> Result<LinkOutcome, ErrorCode> {
        if !valid_relation_label(relation) {
            return Err(ErrorCode::Malformed);
        }
        match self.store.link_by_relation::<Entity>(relation, left, right) {
            Ok(crate::generic::LinkOutcome::Linked) => Ok(LinkOutcome::Linked),
            Ok(crate::generic::LinkOutcome::AlreadyLinked) => Ok(LinkOutcome::AlreadyLinked),
            Err(LinkError::UnknownRecord(_)) => Err(ErrorCode::RecordNotFound),
            Err(LinkError::SelfLoop(_) | LinkError::InvalidLabel(_)) => Err(ErrorCode::Malformed),
            Err(LinkError::Durability(_)) => Err(ErrorCode::Storage),
        }
    }

    /// `ENT2-FR-006`: `Entity` has no `ChildOf` relation.
    fn parent(&self, _id: RecordId) -> Result<ParentLookup, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn children(&self, _id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn neighbors(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        Ok(self.store.all_neighbors::<Entity>(id))
    }

    fn neighbors_by_relation(
        &self,
        id: RecordId,
        relation: &str,
    ) -> Result<Vec<RecordId>, ErrorCode> {
        match self.store.neighbors_by_relation::<Entity>(relation, id) {
            Some(records) => Ok(records),
            None => Err(ErrorCode::Malformed),
        }
    }

    /// `CNT-FR-002` (ADR-0057): one read under the store's lock; an
    /// unknown label is `Malformed`, as for `neighbors_by_relation`.
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
        let apply = |inner: &mut EntityProductionStack| {
            for (i, p) in prepared.iter().enumerate() {
                if let PreparedWrite::Link { left, .. } = p {
                    if GetById::<Entity>::get(inner, *left).is_none() {
                        return Err((i, ErrorCode::RecordNotFound));
                    }
                }
            }
            let mut results = Vec::with_capacity(prepared.len());
            for (i, p) in prepared.into_iter().enumerate() {
                results.push(Self::apply_prepared(inner, p).map_err(|code| (i, code))?);
            }
            self.mvcc_record_write_ops(ops, &results);
            Ok(results)
        };
        match &self.journal {
            // See `MemoryConnectionStore::write_batch`: no journal means no
            // checkpoint boundary, so this atomic batch flushes MVCC
            // history immediately.
            None => self.store.with_exclusive(|inner| {
                let results = apply(inner)?;
                if !self.mvcc_flush_now(0) {
                    return Err((0, ErrorCode::Storage));
                }
                Ok(results)
            }),
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
        match self.store.count_edges::<Entity>(relation) {
            Some(count) => Ok(count as u64),
            None => Err(ErrorCode::Malformed),
        }
    }

    fn list_relation_kinds(&self) -> Vec<String> {
        self.store.relation_kinds::<Entity>()
    }

    /// `STV-FR-002`: `validate_batch` on this one operation, with the
    /// same per-call existence read the journaled path uses.
    fn validate_op(&self, op: &TransactionOp) -> Result<(), ErrorCode> {
        Self::validate_batch(std::slice::from_ref(op), |id| {
            self.store.get::<Entity>(id).is_some()
        })
        .map_err(|(_, code)| code)
    }

    fn table_name(&self) -> &str {
        "entity"
    }

    fn describe(&self) -> DomainSchema {
        DomainSchema {
            fields: vec![
                FieldDescriptor {
                    tag: FIELD_LABEL,
                    name: "label".into(),
                    value_kind: ValueKind::Str,
                    // `ENT3-FR-006`: `filter_eq` real since ADR-0040 (via
                    // `NameIndex`, matching aliases too); still read-only.
                    capabilities: FieldCapabilities {
                        filter_eq: true,
                        scan: false,
                        update: false,
                    },
                },
                FieldDescriptor {
                    tag: FIELD_KIND,
                    name: "kind".into(),
                    value_kind: ValueKind::Str,
                    capabilities: FieldCapabilities {
                        filter_eq: true,
                        scan: false,
                        update: false,
                    },
                },
                FieldDescriptor {
                    tag: FIELD_MENTION_COUNT,
                    name: "mention_count".into(),
                    value_kind: ValueKind::I64,
                    capabilities: FieldCapabilities {
                        filter_eq: false,
                        scan: true,
                        update: true,
                    },
                },
                // `ENT4-FR-002` (ADR-0041): read-only — stripped from this
                // schema by `downgrade_for_version` for a connection
                // negotiated below 11.
                FieldDescriptor {
                    tag: FIELD_ALIASES,
                    name: "aliases".into(),
                    value_kind: ValueKind::StrList,
                    capabilities: FieldCapabilities {
                        filter_eq: false,
                        scan: false,
                        update: false,
                    },
                },
            ],
            // `Dog`'s own shape: neighbors only, no parent/children.
            relations: RelationCapabilities {
                parent_children: false,
                neighbors: true,
            },
        }
    }

    fn apply_transaction(
        &self,
        updates: &[TransactionOp],
        read_set: &[(RecordId, FieldRef, ScanValue)],
    ) -> Result<(), (usize, ErrorCode)> {
        // See `DogConnectionStore::apply_transaction` for the two paths
        // (`GRP-FR-001`–`005`) and where the read-set check runs in each;
        // identical here.
        match &self.journal {
            None => self.store.with_exclusive(|inner| {
                Self::validate_batch(updates, |id| GetById::<Entity>::get(inner, id).is_some())?;
                Self::check_read_set(read_set, |id| GetById::<Entity>::get(inner, id))?;
                Self::apply_batch(inner, updates)?;
                // `SYU-FR-003` (ADR-0097): no journal covers this batch,
                // so force the slots to disk before acknowledging.
                if self.sync_updates && inner.checkpoint_flush().is_err() {
                    return Err((0, ErrorCode::Storage));
                }
                self.mvcc_record_transaction(updates);
                // See `MemoryConnectionStore::apply_transaction`: no
                // journal means no checkpoint boundary, so this commit
                // flushes MVCC history now.
                if !self.mvcc_flush_now(0) {
                    return Err((0, ErrorCode::Storage));
                }
                Ok(())
            }),
            Some(journal) => {
                Self::validate_batch(updates, |id| self.store.get::<Entity>(id).is_some())?;
                journal
                    .commit(updates, |turn| {
                        self.store.with_exclusive(|inner| {
                            Self::check_read_set(read_set, |id| GetById::<Entity>::get(inner, id))?;
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

    /// `ADR-0072`'s `MVCC2-FR-007` — see `MemoryConnectionStore::
    /// apply_transaction_mvcc` for the full contract; identical here.
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
                Self::validate_batch(updates, |id| GetById::<Entity>::get(inner, id).is_some())?;
                Self::check_read_set(read_set, |id| GetById::<Entity>::get(inner, id))?;
                conflict_check()?;
                Self::apply_batch(inner, updates)?;
                // `SYU-FR-003` (ADR-0097): no journal covers this batch,
                // so force the slots to disk before acknowledging.
                if self.sync_updates && inner.checkpoint_flush().is_err() {
                    return Err((0, ErrorCode::Storage));
                }
                self.mvcc_record_transaction(updates);
                // See `MemoryConnectionStore::apply_transaction`: no
                // journal means no checkpoint boundary, so this commit
                // flushes MVCC history now.
                if !self.mvcc_flush_now(0) {
                    return Err((0, ErrorCode::Storage));
                }
                Ok(())
            }),
            Some(journal) => {
                Self::validate_batch(updates, |id| self.store.get::<Entity>(id).is_some())?;
                journal
                    .commit(updates, |turn| {
                        self.store.with_exclusive(|inner| {
                            Self::check_read_set(read_set, |id| GetById::<Entity>::get(inner, id))?;
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

    /// `ADR-0072`'s `MVCC2-FR-012`: `Entity` implements real MVCC.
    fn mvcc_supported(&self) -> bool {
        self.mvcc.is_some()
    }

    /// `ADR-0072`'s `MVCC2-FR-001`/`005` — see `MemoryConnectionStore::
    /// mvcc_begin` for the full contract; identical here.
    fn mvcc_begin(&self) -> u64 {
        let Some(mvcc) = &self.mvcc else { return 0 };
        if !mvcc.state.is_active() {
            self.store.with_exclusive(|inner| {
                mvcc.state.activate();
                for id in AllIds::<Entity>::all_ids(inner) {
                    if let Some(record) = GetById::<Entity>::get(inner, id) {
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

    /// `ADR-0072`'s `MVCC2-FR-006` — see `MemoryConnectionStore::
    /// mvcc_get` for the full contract; identical here.
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
                Ok(None) => {}
            }
        }
        Ok(Some(fields))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generic::entity::{
        create_entity_production_stack, open_entity_production_stack_portable,
    };
    use crate::test_support::fresh_temp_dir;
    use uuid::Uuid;

    fn sample_entities() -> Vec<Entity> {
        vec![
            Entity {
                id: Uuid::from_u128(1),
                label: "Ada Lovelace".into(),
                kind: "person".into(),
                mention_count: 3,
                aliases: vec!["Ada".into(), "Countess of Lovelace".into()],
            },
            Entity {
                id: Uuid::from_u128(2),
                label: "Analytical Engine".into(),
                kind: "concept".into(),
                mention_count: 5,
                aliases: vec![],
            },
            Entity {
                id: Uuid::from_u128(3),
                label: "London".into(),
                kind: "place".into(),
                mention_count: 1,
                aliases: vec!["Londinium".into()],
            },
        ]
    }

    fn sample_adapter() -> EntityConnectionStore {
        let dir = fresh_temp_dir("server_entity_v2_adapter").unwrap();
        let path = dir.join("entities.mmap");
        let relates_to = vec![(Uuid::from_u128(1), Uuid::from_u128(2))];
        let mentioned_with = vec![(Uuid::from_u128(1), Uuid::from_u128(3))];
        let stack =
            create_entity_production_stack(sample_entities(), &relates_to, &mentioned_with, &path)
                .unwrap();
        EntityConnectionStore::new(GenericProductionStore::new(stack))
    }

    /// `MUR-FR-001`/`002` (ADR-0108) on `Entity`, `RVL-FR-010` (ADR-0112):
    /// the in-place `UpdateField` records into the MVCC index — a held
    /// snapshot still reads the old value, the current reads the new.
    #[test]
    fn an_in_place_update_is_recorded_in_the_mvcc_index() {
        let (adapter, _path) = sample_adapter_with_mvcc();
        let id = Uuid::from_u128(1);
        let held = adapter.mvcc_begin();
        let old = adapter.mvcc_get(id, held).unwrap().unwrap();
        let before = adapter.mvcc_history_entries().unwrap();
        assert_eq!(
            adapter.update_field(id, FIELD_MENTION_COUNT, ScanValue::I64(77)),
            Ok(true)
        );
        assert_eq!(adapter.mvcc_history_entries(), Some(before + 1));
        assert_eq!(adapter.mvcc_get(id, held).unwrap().unwrap(), old);
        assert_eq!(
            adapter.get(id).unwrap()[FIELD_MENTION_COUNT as usize].1,
            ScanValue::I64(77)
        );
        adapter.mvcc_release(held);
    }

    fn sample_adapter_with_mvcc() -> (EntityConnectionStore, std::path::PathBuf) {
        let dir = fresh_temp_dir("server_entity_mvcc_adapter").unwrap();
        let path = dir.join("entities.mmap");
        let stack = create_entity_production_stack(sample_entities(), &[], &[], &path).unwrap();
        let adapter = EntityConnectionStore::new(GenericProductionStore::new(stack))
            .with_mvcc(&path)
            .unwrap();
        (adapter, path)
    }

    /// `ADR-0072`: activation baseline, a snapshot surviving a concurrent
    /// ordinary write and a concurrent atomic `WriteBatch`, invisibility
    /// of a post-snapshot insert, and write-write conflict detection.
    /// Mirrors `MemoryConnectionStore`'s own MVCC test suite.
    #[test]
    fn mvcc_activation_snapshot_isolation_and_conflict_detection() {
        let (adapter, _path) = sample_adapter_with_mvcc();
        assert!(adapter.mvcc_supported());
        let id = Uuid::from_u128(1);
        let before = adapter.mvcc_begin();
        assert_eq!(adapter.mvcc_get(id, before).unwrap(), adapter.get(id));

        let edited = vec![
            (FIELD_LABEL, ScanValue::Str("Ada King".into())),
            (FIELD_KIND, ScanValue::Str("person".into())),
            (FIELD_MENTION_COUNT, ScanValue::I64(99)),
            (
                FIELD_ALIASES,
                ScanValue::StrList(vec!["Enchantress".into()]),
            ),
        ];
        assert_eq!(
            adapter.replace_record(id, edited.clone()),
            Ok(ReplaceOutcome::Replaced)
        );
        assert_eq!(
            adapter.mvcc_get(id, before).unwrap(),
            Some(vec![
                (FIELD_LABEL, ScanValue::Str("Ada Lovelace".into())),
                (FIELD_KIND, ScanValue::Str("person".into())),
                (FIELD_MENTION_COUNT, ScanValue::I64(3)),
                (
                    FIELD_ALIASES,
                    ScanValue::StrList(vec!["Ada".into(), "Countess of Lovelace".into()])
                ),
            ]),
            "the snapshot still sees the pre-replace value"
        );

        let new_id = Uuid::from_u128(77);
        let new_fields = vec![
            (FIELD_LABEL, ScanValue::Str("Grace Hopper".into())),
            (FIELD_KIND, ScanValue::Str("person".into())),
            (FIELD_MENTION_COUNT, ScanValue::I64(1)),
            (FIELD_ALIASES, ScanValue::StrList(vec![])),
        ];
        assert_eq!(
            adapter.insert_record(new_id, new_fields.clone()),
            Ok(InsertOutcome::Inserted)
        );
        assert_eq!(adapter.mvcc_get(new_id, before).unwrap(), None);

        let after = adapter.mvcc_begin();
        assert_eq!(adapter.mvcc_get(id, after).unwrap(), Some(edited));
        assert_eq!(adapter.mvcc_get(new_id, after).unwrap(), Some(new_fields));

        assert_eq!(
            adapter.apply_transaction_mvcc(
                &[TransactionOp {
                    id,
                    field: FIELD_MENTION_COUNT,
                    value: ScanValue::I64(5),
                }],
                &[],
                before,
            ),
            Err((0, ErrorCode::Conflict)),
            "stale snapshot conflicts with the ordinary replace above"
        );
        assert_eq!(
            adapter.apply_transaction_mvcc(
                &[TransactionOp {
                    id,
                    field: FIELD_MENTION_COUNT,
                    value: ScanValue::I64(5),
                }],
                &[],
                after,
            ),
            Ok(())
        );
        adapter.mvcc_release(before);
        adapter.mvcc_release(after);
    }

    /// `MVCC2-FR-008`: an atomic `WriteBatch`'s `Insert`/`Replace`/
    /// `Delete`/`Link` — `Link` never gets an MVCC entry, since it
    /// touches no field `mvcc_get` reads.
    #[test]
    fn mvcc_snapshot_survives_a_concurrent_atomic_write_batch() {
        let (adapter, _path) = sample_adapter_with_mvcc();
        let (kept, deleted) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let before = adapter.mvcc_begin();
        let before_kept = adapter.get(kept).unwrap();
        let before_deleted = adapter.get(deleted).unwrap();

        let mut edited = before_kept.clone();
        edited[2] = (FIELD_MENTION_COUNT, ScanValue::I64(42));
        let ops = vec![
            WriteOp::Replace {
                id: kept,
                fields: edited.clone(),
            },
            WriteOp::Delete { id: deleted },
        ];
        assert_eq!(
            adapter.write_batch(&ops, true),
            Ok(vec![WriteResult::Replaced, WriteResult::Deleted])
        );
        assert_eq!(adapter.mvcc_get(kept, before).unwrap(), Some(before_kept));
        assert_eq!(
            adapter.mvcc_get(deleted, before).unwrap(),
            Some(before_deleted)
        );

        let after = adapter.mvcc_begin();
        assert_eq!(adapter.mvcc_get(kept, after).unwrap(), Some(edited));
        assert_eq!(adapter.mvcc_get(deleted, after).unwrap(), None);
        adapter.mvcc_release(before);
        adapter.mvcc_release(after);
    }

    /// Acceptance criterion 8: `Compact` flushes MVCC history before
    /// clearing, and a reopen afterward reconstructs it correctly.
    #[test]
    fn compact_flushes_mvcc_history_and_a_reopen_reconstructs_it() {
        let (adapter, path) = sample_adapter_with_mvcc();
        let id = Uuid::from_u128(1);
        let before = adapter.mvcc_begin();
        let original = adapter.get(id).unwrap();
        let mut edited = original.clone();
        edited[2] = (FIELD_MENTION_COUNT, ScanValue::I64(42));
        assert_eq!(
            adapter.replace_record(id, edited.clone()),
            Ok(ReplaceOutcome::Replaced)
        );
        adapter.compact().unwrap();
        drop(adapter);

        let stack = open_entity_production_stack_portable(&path).unwrap();
        let reopened = EntityConnectionStore::new(GenericProductionStore::new(stack))
            .with_mvcc(&path)
            .unwrap();
        assert_eq!(reopened.mvcc_get(id, before).unwrap(), Some(original));
        let after = reopened.mvcc_begin();
        assert_eq!(reopened.mvcc_get(id, after).unwrap(), Some(edited));
    }

    /// `JMC-FR-001`/`002` (ADR-0114): the journaled reopen — a journaled
    /// atomic `WriteBatch` (a replace) then a journaled `Transaction` on
    /// the same record, no checkpoint, reach a fresh snapshot after
    /// `open_with_mvcc_journaled`, in journal order. See
    /// `MemoryConnectionStore`'s test for the insert-and-delete arms.
    #[test]
    fn open_with_mvcc_journaled_folds_the_replayed_batches_in_journal_order() {
        let dir = fresh_temp_dir("server_entity_journaled_mvcc_reopen").unwrap();
        let path = dir.join("entitys.mmap");
        let journal = dir.join("entitys.journal");
        let id = Uuid::from_u128(1);
        let mut edited;
        {
            let stack = create_entity_production_stack(sample_entities(), &[], &[], &path).unwrap();
            let adapter =
                EntityConnectionStore::with_journal(GenericProductionStore::new(stack), &journal)
                    .unwrap()
                    .with_mvcc(&path)
                    .unwrap();
            let baseline = adapter.mvcc_begin();
            adapter.mvcc_release(baseline);
            edited = adapter.get(id).unwrap();
            edited[2] = (FIELD_MENTION_COUNT, ScanValue::I64(42));
            assert_eq!(
                adapter
                    .write_batch(
                        &[WriteOp::Replace {
                            id,
                            fields: edited.clone(),
                        }],
                        true,
                    )
                    .unwrap(),
                vec![WriteResult::Replaced]
            );
            drop(adapter);
        }

        let reopened = EntityConnectionStore::open_with_mvcc_journaled(&path, &journal).unwrap();
        let after = reopened.mvcc_begin();
        assert_eq!(
            reopened.mvcc_get(id, after).unwrap(),
            Some(edited),
            "a fresh snapshot after the journaled reopen sees the journaled batch"
        );
    }

    /// `docs/design/MVCC-OPEN-HOOK-PROPOSAL.md` acceptance criterion 1 —
    /// see `MemoryConnectionStore`'s own identical test for the full
    /// rationale: a genuinely pending, unflushed insert-log entry (an
    /// ordinary `replace_record`, no `Compact` in between) at the moment
    /// of a simulated restart is still correctly reconstructed by
    /// `open_with_mvcc`, which `with_mvcc` alone cannot do.
    #[test]
    fn open_with_mvcc_reconstructs_a_pending_unflushed_insert_log_entry() {
        let (adapter, path) = sample_adapter_with_mvcc();
        let id = Uuid::from_u128(1);
        let before = adapter.mvcc_begin();
        let original = adapter.get(id).unwrap();
        let mut edited = original.clone();
        edited[2] = (FIELD_MENTION_COUNT, ScanValue::I64(42));
        assert_eq!(
            adapter.replace_record(id, edited.clone()),
            Ok(ReplaceOutcome::Replaced),
            "an ordinary write — appends to the insert log, no Compact/flush follows"
        );
        drop(adapter);

        let reopened = EntityConnectionStore::open_with_mvcc(&path).unwrap();
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

    /// The round-ten fix: on a non-journaled table, `insert_record`/
    /// `replace_record`/`delete_record` each used to record into the
    /// in-memory MVCC index but never flush `<mmap_path>.mvcc` — only
    /// `Compact` did. Checked directly against the on-disk file via an
    /// independent `MvccState::open`, not via `open_with_mvcc`'s own
    /// insert-log fold — see `MemoryConnectionStore`'s identical test for
    /// why a restart-based test wouldn't discriminate this fix.
    #[test]
    fn insert_replace_delete_each_flush_mvcc_history_immediately_with_no_compact() {
        let (adapter, path) = sample_adapter_with_mvcc();
        adapter.mvcc_begin();

        let read_field = |id: Uuid, field: FieldRef| {
            MvccState::open(&path)
                .unwrap()
                .state
                .with_index(|index| index.read(&(id, field), u64::MAX))
                .unwrap()
        };

        let new_id = Uuid::from_u128(77);
        let new_fields = vec![
            (FIELD_LABEL, ScanValue::Str("Grace Hopper".into())),
            (FIELD_KIND, ScanValue::Str("person".into())),
            (FIELD_MENTION_COUNT, ScanValue::I64(1)),
            (FIELD_ALIASES, ScanValue::StrList(vec![])),
        ];
        assert_eq!(
            adapter.insert_record(new_id, new_fields),
            Ok(InsertOutcome::Inserted)
        );
        assert_eq!(
            read_field(new_id, mvcc::EXISTENCE_FIELD),
            Some(ScanValue::Bool(true)),
            "insert_record must flush to disk immediately, not only at Compact"
        );

        let id = Uuid::from_u128(1);
        let mut edited = adapter.get(id).unwrap();
        edited[2] = (FIELD_MENTION_COUNT, ScanValue::I64(42));
        assert_eq!(
            adapter.replace_record(id, edited),
            Ok(ReplaceOutcome::Replaced)
        );
        assert_eq!(
            read_field(id, FIELD_MENTION_COUNT),
            Some(ScanValue::I64(42)),
            "replace_record must flush to disk immediately, not only at Compact"
        );

        let deleted = Uuid::from_u128(2);
        assert_eq!(adapter.delete_record(deleted), Ok(DeleteOutcome::Deleted));
        assert_eq!(
            read_field(deleted, mvcc::EXISTENCE_FIELD),
            None,
            "delete_record's tombstone must flush to disk immediately, not only at Compact"
        );
    }

    /// See `insert_replace_delete_each_flush_mvcc_history_immediately_
    /// with_no_compact`: a non-journaled atomic `WriteBatch` must flush
    /// too, checked the same direct way.
    #[test]
    fn write_batch_atomic_non_journaled_flushes_mvcc_history_immediately() {
        let (adapter, path) = sample_adapter_with_mvcc();
        adapter.mvcc_begin();

        let new_id = Uuid::from_u128(77);
        let new_fields = vec![
            (FIELD_LABEL, ScanValue::Str("Grace Hopper".into())),
            (FIELD_KIND, ScanValue::Str("person".into())),
            (FIELD_MENTION_COUNT, ScanValue::I64(1)),
            (FIELD_ALIASES, ScanValue::StrList(vec![])),
        ];
        let ops = vec![WriteOp::Insert {
            id: new_id,
            fields: new_fields,
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

    /// The round-ten fix's core case — see `MemoryConnectionStore`'s
    /// identical test for the full rationale: `apply_transaction` never
    /// touches the insert log, so this restart only reconstructs
    /// correctly if `apply_transaction` itself flushed.
    #[test]
    fn apply_transaction_non_journaled_flushes_mvcc_history_and_a_restart_reconstructs_it() {
        let (adapter, path) = sample_adapter_with_mvcc();
        let id = Uuid::from_u128(1);

        let before = adapter.mvcc_begin();
        assert_eq!(
            adapter.apply_transaction(
                &[TransactionOp {
                    id,
                    field: FIELD_MENTION_COUNT,
                    value: ScanValue::I64(42),
                }],
                &[],
            ),
            Ok(())
        );
        drop(adapter);

        let reopened = EntityConnectionStore::open_with_mvcc(&path).unwrap();
        assert!(reopened.mvcc_supported());
        assert_eq!(
            reopened.mvcc_get(id, before).unwrap().unwrap()[2],
            (FIELD_MENTION_COUNT, ScanValue::I64(3)),
            "the pre-commit snapshot still sees the pre-commit value after reopen"
        );
        let after = reopened.mvcc_begin();
        assert_eq!(
            reopened.mvcc_get(id, after).unwrap().unwrap()[2],
            (FIELD_MENTION_COUNT, ScanValue::I64(42)),
            "a fresh snapshot after reopen sees the commit apply_transaction flushed"
        );
    }

    /// See `apply_transaction_non_journaled_flushes_mvcc_history_and_a_
    /// restart_reconstructs_it`: `apply_transaction_mvcc`'s own
    /// non-journaled path has the identical gap and fix.
    #[test]
    fn apply_transaction_mvcc_non_journaled_flushes_mvcc_history_and_a_restart_reconstructs_it() {
        let (adapter, path) = sample_adapter_with_mvcc();
        let id = Uuid::from_u128(1);

        let before = adapter.mvcc_begin();
        assert_eq!(
            adapter.apply_transaction_mvcc(
                &[TransactionOp {
                    id,
                    field: FIELD_MENTION_COUNT,
                    value: ScanValue::I64(42),
                }],
                &[],
                before,
            ),
            Ok(())
        );
        drop(adapter);

        let reopened = EntityConnectionStore::open_with_mvcc(&path).unwrap();
        assert_eq!(
            reopened.mvcc_get(id, before).unwrap().unwrap()[2],
            (FIELD_MENTION_COUNT, ScanValue::I64(3)),
            "the pre-commit snapshot still sees the pre-commit value after reopen"
        );
        let after = reopened.mvcc_begin();
        assert_eq!(
            reopened.mvcc_get(id, after).unwrap().unwrap()[2],
            (FIELD_MENTION_COUNT, ScanValue::I64(42)),
            "a fresh snapshot after reopen sees the commit apply_transaction_mvcc flushed"
        );
    }

    #[test]
    fn get_returns_every_field() {
        let adapter = sample_adapter();
        assert_eq!(
            adapter.get(Uuid::from_u128(1)).unwrap(),
            vec![
                (FIELD_LABEL, ScanValue::Str("Ada Lovelace".into())),
                (FIELD_KIND, ScanValue::Str("person".into())),
                (FIELD_MENTION_COUNT, ScanValue::I64(3)),
                // `ENT4-FR-002`: raw, stored order, un-normalized.
                (
                    FIELD_ALIASES,
                    ScanValue::StrList(vec!["Ada".into(), "Countess of Lovelace".into()])
                ),
            ]
        );
        assert_eq!(
            adapter.get(Uuid::from_u128(2)).unwrap()[3],
            (FIELD_ALIASES, ScanValue::StrList(vec![])),
            "an empty list is still a present field"
        );
        assert!(adapter.get(Uuid::from_u128(99)).is_none());
    }

    #[test]
    fn filter_eq_by_kind_open_ended_and_unsupported_fields() {
        let adapter = sample_adapter();
        assert_eq!(
            adapter.filter_eq(FIELD_KIND, &ScanValue::Str("person".into())),
            Ok(vec![Uuid::from_u128(1)])
        );
        // Open-ended: any string is accepted, no discriminant to fail.
        assert!(adapter
            .filter_eq(FIELD_KIND, &ScanValue::Str("nonexistent-kind".into()))
            .unwrap()
            .is_empty());
        assert_eq!(
            adapter.filter_eq(FIELD_MENTION_COUNT, &ScanValue::I64(0)),
            Err(ErrorCode::Unsupported)
        );
    }

    /// `ENT3-FR-005`: `label` or any alias, case/whitespace-insensitive,
    /// through the raw wire string; a miss is empty; a non-`Str` is
    /// `Malformed` (the same shape `kind`'s own non-`Str` case has).
    #[test]
    fn filter_eq_by_label_matches_label_and_aliases_normalized() {
        let adapter = sample_adapter();
        let ada = Ok(vec![Uuid::from_u128(1)]);
        assert_eq!(
            adapter.filter_eq(FIELD_LABEL, &ScanValue::Str("Ada Lovelace".into())),
            ada
        );
        assert_eq!(
            adapter.filter_eq(FIELD_LABEL, &ScanValue::Str("  ada LOVELACE ".into())),
            ada
        );
        // `ENT5-FR-001`: an internal whitespace run is the same name.
        assert_eq!(
            adapter.filter_eq(FIELD_LABEL, &ScanValue::Str("ada   lovelace".into())),
            ada
        );
        assert_eq!(
            adapter.filter_eq(FIELD_LABEL, &ScanValue::Str("countess of lovelace".into())),
            ada
        );
        assert_eq!(
            adapter.filter_eq(FIELD_LABEL, &ScanValue::Str("LONDINIUM".into())),
            Ok(vec![Uuid::from_u128(3)])
        );
        assert_eq!(
            adapter.filter_eq(FIELD_LABEL, &ScanValue::Str("nobody".into())),
            Ok(vec![])
        );
        assert_eq!(
            adapter.filter_eq(FIELD_LABEL, &ScanValue::I64(1)),
            Err(ErrorCode::Malformed)
        );
    }

    #[test]
    fn scan_and_update_mention_count_only_kind_is_read_only() {
        let adapter = sample_adapter();
        let mut counts = adapter.scan_field(FIELD_MENTION_COUNT).unwrap();
        counts.sort_by_key(|v| match v {
            ScanValue::I64(n) => *n,
            _ => 0,
        });
        assert_eq!(
            counts,
            vec![ScanValue::I64(1), ScanValue::I64(3), ScanValue::I64(5)]
        );
        assert_eq!(adapter.scan_field(FIELD_KIND), Err(ErrorCode::Unsupported));

        assert_eq!(
            adapter.update_field(Uuid::from_u128(1), FIELD_MENTION_COUNT, ScanValue::I64(4)),
            Ok(true)
        );
        assert_eq!(
            adapter.update_field(Uuid::from_u128(1), FIELD_KIND, ScanValue::Str("x".into())),
            Err(ErrorCode::Unsupported)
        );
    }

    #[test]
    fn neighbors_by_relation_and_unfiltered_and_list_relation_kinds() {
        let adapter = sample_adapter();
        assert_eq!(
            adapter.neighbors_by_relation(Uuid::from_u128(1), "relates_to"),
            Ok(vec![Uuid::from_u128(2)])
        );
        assert_eq!(
            adapter.neighbors_by_relation(Uuid::from_u128(1), "mentioned_with"),
            Ok(vec![Uuid::from_u128(3)])
        );
        assert_eq!(
            adapter.neighbors_by_relation(Uuid::from_u128(1), "unknown"),
            Err(ErrorCode::Malformed)
        );
        let mut unfiltered = adapter.neighbors(Uuid::from_u128(1)).unwrap();
        unfiltered.sort();
        assert_eq!(unfiltered, vec![Uuid::from_u128(2), Uuid::from_u128(3)]);
        let mut kinds = adapter.list_relation_kinds();
        kinds.sort();
        assert_eq!(
            kinds,
            vec!["mentioned_with".to_string(), "relates_to".to_string()]
        );
        assert_eq!(
            adapter.parent(Uuid::from_u128(1)),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            adapter.children(Uuid::from_u128(1)),
            Err(ErrorCode::Unsupported)
        );
    }

    /// `ENT4-FR-002`: `aliases` is a *known* field that supports nothing —
    /// every write/filter/scan path is `Unsupported`, never `UnknownField`
    /// (which tag 3 was before this round) and never `Malformed`.
    #[test]
    fn aliases_is_a_known_field_every_operation_refuses_as_unsupported() {
        let adapter = sample_adapter();
        let list = ScanValue::StrList(vec!["x".into()]);
        assert_eq!(
            adapter.filter_eq(FIELD_ALIASES, &list),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            adapter.filter_eq(FIELD_ALIASES, &ScanValue::Str("Ada".into())),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            adapter.scan_field(FIELD_ALIASES),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            adapter.update_field(Uuid::from_u128(1), FIELD_ALIASES, list.clone()),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            adapter.validate_op(&TransactionOp {
                id: Uuid::from_u128(1),
                field: FIELD_ALIASES,
                value: list,
            }),
            Err(ErrorCode::Unsupported)
        );
        // Tag 4 is still unknown — the boundary moved by exactly one.
        assert_eq!(adapter.scan_field(4), Err(ErrorCode::UnknownField));
        // A read-set entry naming `aliases` compares against the raw list.
        assert!(EntityConnectionStore::check_read_set(
            &[(
                Uuid::from_u128(3),
                FIELD_ALIASES,
                ScanValue::StrList(vec!["Londinium".into()])
            )],
            |id| adapter.store.get::<Entity>(id),
        )
        .is_ok());
    }

    #[test]
    fn describe_names_all_four_fields_and_reports_neighbors_only() {
        let adapter = sample_adapter();
        let schema = adapter.describe();
        // Four wire fields since protocol 11 — `aliases` gained
        // `FIELD_ALIASES` in `ENT4-FR-002` with every flag `false`.
        assert_eq!(schema.fields.len(), 4);
        let aliases = schema.fields.iter().find(|f| f.name == "aliases").unwrap();
        assert_eq!(aliases.tag, FIELD_ALIASES);
        assert_eq!(aliases.value_kind, ValueKind::StrList);
        assert!(
            !aliases.capabilities.filter_eq
                && !aliases.capabilities.scan
                && !aliases.capabilities.update
        );
        let label = schema.fields.iter().find(|f| f.name == "label").unwrap();
        assert!(
            label.capabilities.filter_eq && !label.capabilities.scan && !label.capabilities.update
        );
        let kind = schema.fields.iter().find(|f| f.name == "kind").unwrap();
        assert!(
            kind.capabilities.filter_eq && !kind.capabilities.scan && !kind.capabilities.update
        );
        let mention_count = schema
            .fields
            .iter()
            .find(|f| f.name == "mention_count")
            .unwrap();
        assert!(
            mention_count.capabilities.scan
                && mention_count.capabilities.update
                && !mention_count.capabilities.filter_eq
        );
        assert!(schema.relations.neighbors);
        assert!(!schema.relations.parent_children);
    }
    /// `INS-FR-006` (ADR-0046): all four fields, `aliases` as a
    /// `StrList`, validated before any write; the inserted entity is then
    /// found by label and alias through `filter_eq` on `label`; the
    /// repeat is a duplicate; a wrong-kind `aliases` is `Malformed`.
    #[test]
    fn insert_record_takes_aliases_as_a_str_list_and_indexes_them() {
        let adapter = sample_adapter();
        let id = Uuid::from_u128(77);
        let fields = vec![
            (FIELD_LABEL, ScanValue::Str("Grace Hopper".into())),
            (FIELD_KIND, ScanValue::Str("person".into())),
            (FIELD_MENTION_COUNT, ScanValue::I64(1)),
            (
                FIELD_ALIASES,
                ScanValue::StrList(vec!["Amazing Grace".into()]),
            ),
        ];
        assert_eq!(
            adapter.insert_record(id, fields.clone()),
            Ok(InsertOutcome::Inserted)
        );
        assert_eq!(adapter.get(id).unwrap(), fields);
        assert_eq!(
            adapter.filter_eq(FIELD_LABEL, &ScanValue::Str("amazing grace".into())),
            Ok(vec![id])
        );
        assert_eq!(
            adapter.filter_eq(FIELD_LABEL, &ScanValue::Str("GRACE HOPPER".into())),
            Ok(vec![id])
        );
        assert_eq!(adapter.neighbors(id), Ok(vec![]));
        assert_eq!(
            adapter.insert_record(id, fields.clone()),
            Ok(InsertOutcome::Duplicate)
        );

        let mut wrong_aliases = fields.clone();
        wrong_aliases[3] = (FIELD_ALIASES, ScanValue::Str("Amazing Grace".into()));
        assert_eq!(
            adapter.insert_record(Uuid::from_u128(78), wrong_aliases),
            Err(ErrorCode::Malformed)
        );
        assert_eq!(
            adapter.insert_record(Uuid::from_u128(78), fields[..3].to_vec()),
            Err(ErrorCode::Malformed),
            "aliases is required, an empty list is the way to say none"
        );
        assert!(adapter.get(Uuid::from_u128(78)).is_none());
    }
    /// `LNK-FR-009` (ADR-0047): open labels through the adapter — a link
    /// under a new label lands, `list_relation_kinds` and
    /// `neighbors_by_relation` see it, the repeat is `AlreadyLinked`, and
    /// every refusal maps to its code with nothing written.
    #[test]
    fn link_records_accepts_any_valid_label_and_maps_every_refusal() {
        let adapter = sample_adapter();
        let (ada, engine) = (Uuid::from_u128(1), Uuid::from_u128(2));
        assert_eq!(
            adapter.link_records(ada, engine, "invented_by"),
            Ok(LinkOutcome::Linked)
        );
        assert_eq!(
            adapter.link_records(engine, ada, "invented_by"),
            Ok(LinkOutcome::AlreadyLinked)
        );
        assert_eq!(
            adapter.neighbors_by_relation(engine, "invented_by"),
            Ok(vec![ada])
        );
        assert!(adapter
            .list_relation_kinds()
            .contains(&"invented_by".to_string()));
        assert_eq!(
            adapter.link_records(ada, Uuid::from_u128(99), "invented_by"),
            Err(ErrorCode::RecordNotFound)
        );
        assert_eq!(
            adapter.link_records(ada, ada, "relates_to"),
            Err(ErrorCode::Malformed)
        );
        for bad in ["", "no spaces", "../x"] {
            assert_eq!(
                adapter.link_records(ada, engine, bad),
                Err(ErrorCode::Malformed),
                "{bad:?}"
            );
        }
        assert!(!adapter
            .list_relation_kinds()
            .iter()
            .any(|k| k.contains(' ')));
    }

    /// `REP-FR-005` (ADR-0049): a replaced entity's new label and aliases
    /// resolve through `filter_eq` on `label`, the old alias no longer
    /// does, and its neighbors are untouched; an unknown id is `NotFound`.
    #[test]
    fn replace_record_moves_the_name_index_and_keeps_neighbors() {
        let adapter = sample_adapter();
        let id = Uuid::from_u128(1);
        let neighbors_before = adapter.neighbors(id).unwrap();
        assert!(!neighbors_before.is_empty());
        let edited = vec![
            (FIELD_LABEL, ScanValue::Str("Ada King".into())),
            (FIELD_KIND, ScanValue::Str("person".into())),
            (FIELD_MENTION_COUNT, ScanValue::I64(99)),
            (
                FIELD_ALIASES,
                ScanValue::StrList(vec!["Enchantress of Number".into()]),
            ),
        ];
        assert_eq!(
            adapter.replace_record(id, edited.clone()),
            Ok(ReplaceOutcome::Replaced)
        );
        assert_eq!(adapter.get(id).unwrap(), edited);
        assert_eq!(
            adapter.filter_eq(FIELD_LABEL, &ScanValue::Str("enchantress of number".into())),
            Ok(vec![id])
        );
        assert_eq!(
            adapter.filter_eq(FIELD_LABEL, &ScanValue::Str("ada lovelace".into())),
            Ok(vec![]),
            "the old label no longer resolves"
        );
        assert_eq!(adapter.neighbors(id), Ok(neighbors_before));
        assert_eq!(
            adapter.replace_record(Uuid::from_u128(4242), edited),
            Ok(ReplaceOutcome::NotFound)
        );
    }

    /// `DEL-FR-006` (ADR-0051): a deleted entity is gone from `get`, from
    /// the name index, and from its neighbors' lists; a repeat is
    /// `NotFound`.
    #[test]
    fn delete_record_removes_the_entity_its_names_and_its_edges() {
        let adapter = sample_adapter();
        let id = Uuid::from_u128(1);
        let neighbors = adapter.neighbors(id).unwrap();
        assert!(!neighbors.is_empty());
        assert_eq!(adapter.delete_record(id), Ok(DeleteOutcome::Deleted));
        assert!(adapter.get(id).is_none());
        assert_eq!(
            adapter.filter_eq(FIELD_LABEL, &ScanValue::Str("ada lovelace".into())),
            Ok(vec![])
        );
        for neighbor in neighbors {
            assert!(!adapter.neighbors(neighbor).unwrap().contains(&id));
        }
        assert_eq!(adapter.delete_record(id), Ok(DeleteOutcome::NotFound));
    }
}
