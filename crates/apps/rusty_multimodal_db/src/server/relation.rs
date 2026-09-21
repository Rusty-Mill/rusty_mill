//! The `Relation` domain's [`ConnectionStore`] adapter — `REL-FR-004`,
//! ADR-0058: the consumer's `entity_relations` table over the wire as a
//! record table. Seven fields in tag order — `subject` (indexed:
//! `FilterEq` is "every edge out of this subject"), `relation`, `object`,
//! `created_at_unix_ms`, `updated_at_unix_ms` (scannable and updatable:
//! `Page` orders by it, last-writer-wins guards on it), `node_id` and
//! `deleted_at_unix_ms` (`ADR-0056`'s sentinels). `Insert`/`Replace`/
//! `ReplaceIf`/`Delete`/`Compact` as every front-door domain; no relation
//! layer of its own, so every edge request is `Unsupported`. Served by
//! `memory_server` as its third table, `relation`.

use super::journal::{CheckpointFlush, CommitError, CommitGroup, JournalError, JournaledBatch};
use super::mvcc::{self, MvccState};
use super::protocol::{
    DomainSchema, ErrorCode, FieldCapabilities, FieldDescriptor, FieldRef, ParentLookup, Predicate,
    RecordId, RelationCapabilities, ScanValue, TransactionOp, ValueKind, WriteOp, WriteResult,
};
use super::{
    bounded_filtered_page, bounded_walk_applies, copy_table_files, filtered_page_by_candidates,
    page_by_scan, page_by_scan_desc, page_key, predicate_matches, read_table_files,
    uuid_pair_bounds, validate_predicate, BackupReport, ConnectionStore, DeleteOutcome,
    InsertOutcome, KeyStats, PageRow, ReadTableFilesError, ReplaceIfOutcome, ReplaceOutcome,
};
use crate::durability::DurabilityError;
use crate::generic::insert_log::{self, LogEntry};
use crate::generic::production::GenericProductionStore;
use crate::generic::query::{AllIds, Delete, GetById, Insert, Replace, UpdateField};
use crate::generic::relation::{
    open_relation_production_stack_portable, Relation, RelationProductionStack, SubjectField,
    UpdatedAtField,
};
use crate::generic::traits::SchemaTag;
use crate::generic::{DeleteError, InsertError, ReplaceError};
use std::ops::Bound;
use std::path::{Path, PathBuf};

pub const FIELD_SUBJECT: FieldRef = 0;
pub const FIELD_RELATION: FieldRef = 1;
pub const FIELD_OBJECT: FieldRef = 2;
pub const FIELD_CREATED_AT: FieldRef = 3;
pub const FIELD_UPDATED_AT: FieldRef = 4;
/// `ADR-0056`'s sentinel: `""` is unattributed.
pub const FIELD_NODE_ID: FieldRef = 5;
/// `ADR-0056`'s sentinel: `0` is live; validated non-negative.
pub const FIELD_DELETED_AT: FieldRef = 6;

/// Every field but the scannable `updated_at_unix_ms`: read-only over
/// `UpdateField`, changed by whole-record `Replace`.
const READ_ONLY_FIELDS: [FieldRef; 6] = [
    FIELD_SUBJECT,
    FIELD_RELATION,
    FIELD_OBJECT,
    FIELD_CREATED_AT,
    FIELD_NODE_ID,
    FIELD_DELETED_AT,
];

/// `ADR-0072`'s `MVCC2-FR-001`/`010`: an active table's MVCC state plus
/// the `mmap_path` [`MvccState::flush`] needs to name `<mmap_path>.mvcc`.
struct MvccHandle {
    state: MvccState,
    mmap_path: PathBuf,
}

pub struct RelationConnectionStore {
    store: GenericProductionStore<RelationProductionStack>,
    /// `JRN-FR-001` (ADR-0025) — see `DogConnectionStore::with_journal`.
    journal: Option<CommitGroup>,
    /// `BAK-FR-002` (ADR-0065) — see `DogConnectionStore::with_backup_source`.
    backup_source: Option<PathBuf>,
    /// `SYU-FR-001` (ADR-0097): when set, every in-place field write that
    /// no journal covers is `msync`ed (the stack's `Flush`) before it is
    /// acknowledged; unset, the acknowledgement precedes durability by
    /// up to the OS's own write-back — the documented loss window.
    sync_updates: bool,
    /// `ADR-0072`'s `MVCC2-FR-001` — see `MemoryConnectionStore`'s own
    /// `mvcc` field for the full contract; identical here.
    mvcc: Option<MvccHandle>,
}

impl RelationConnectionStore {
    pub fn new(store: GenericProductionStore<RelationProductionStack>) -> Self {
        Self {
            store,
            journal: None,
            backup_source: None,
            sync_updates: false,
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

    /// `SYU-FR-002`: the `msync` `with_synced_updates` asks for, taken
    /// under the store's write lock; a failure withholds the
    /// acknowledgement as `Storage`.
    fn sync_update_ack(&self) -> Result<(), ErrorCode> {
        if !self.sync_updates {
            return Ok(());
        }
        self.store.flush().map_err(|_| ErrorCode::Storage)
    }

    /// `ADR-0072`'s `MVCC2-FR-001`/`003` — see
    /// `MemoryConnectionStore::with_mvcc` for the full contract
    /// (including its own documented restart-recovery gap); identical
    /// here.
    pub fn with_mvcc(mut self, mmap_path: &Path) -> Result<Self, DurabilityError> {
        let reconstructed = MvccState::open(mmap_path)?;
        let log = insert_log::log_path(mmap_path);
        let entries = insert_log::read_entries(&log, Relation::SCHEMA_TAG)?;
        Self::fold_pending_log_entries(&reconstructed, entries);
        self.mvcc = Some(MvccHandle {
            state: reconstructed.state,
            mmap_path: mmap_path.to_path_buf(),
        });
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
    /// underlying reopen (`open_relation_production_stack_portable`)
    /// fails.
    pub fn open_with_mvcc(path: &Path) -> Result<Self, DurabilityError> {
        let log = insert_log::log_path(path);
        let pending_entries = insert_log::read_entries(&log, Relation::SCHEMA_TAG)?;

        let stack = open_relation_production_stack_portable(path)?;
        let store = GenericProductionStore::new(stack);

        let reconstructed = MvccState::open(path)?;
        Self::fold_pending_log_entries(&reconstructed, pending_entries);
        Ok(Self {
            store,
            journal: None,
            backup_source: None,
            sync_updates: false,
            mvcc: Some(MvccHandle {
                state: reconstructed.state,
                mmap_path: path.to_path_buf(),
            }),
        })
    }

    /// Shared by [`Self::with_mvcc`] and [`Self::open_with_mvcc`] — see
    /// `MemoryConnectionStore::fold_pending_log_entries` for the full
    /// contract; identical here.
    fn fold_pending_log_entries(
        reconstructed: &mvcc::Reconstructed,
        entries: Vec<LogEntry<Relation, RecordId>>,
    ) {
        let entries = entries
            .into_iter()
            .skip(reconstructed.insert_log_entries_reflected);
        for entry in entries {
            let txn_id = reconstructed.state.counter().next();
            reconstructed.state.with_index(|index| match entry {
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
    }

    /// The crash-atomic variant — see `DogConnectionStore::with_journal`
    /// for the contract; identical here.
    pub fn with_journal(
        store: GenericProductionStore<RelationProductionStack>,
        journal_path: &Path,
    ) -> Result<Self, JournalError> {
        let (journal, batches) = CommitGroup::open(journal_path)?;
        store.with_exclusive(|inner| -> Result<(), JournalError> {
            let schema = Self::schema();
            for (batch_index, batch) in batches.iter().enumerate() {
                match batch {
                    JournaledBatch::Transaction(ops) => {
                        Self::apply_batch(inner, ops).map_err(|(index, code)| {
                            JournalError::Replay {
                                batch: batch_index,
                                index,
                                code,
                            }
                        })?;
                    }
                    JournaledBatch::Write(ops) => {
                        Self::replay_write_batch(inner, &schema, ops).map_err(
                            |(index, code)| JournalError::Replay {
                                batch: batch_index,
                                index,
                                code,
                            },
                        )?;
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
            mvcc: None,
        })
    }

    /// `WBJ-FR-003` (ADR-0063) — see `MemoryConnectionStore::
    /// replay_write_batch` for the full contract; identical here.
    fn replay_write_batch(
        inner: &mut RelationProductionStack,
        schema: &DomainSchema,
        ops: &[WriteOp],
    ) -> Result<(), (usize, ErrorCode)> {
        for (i, op) in ops.iter().enumerate() {
            let prepared = Self::prepare_write(schema, op).map_err(|code| (i, code))?;
            Self::apply_prepared(inner, prepared).map_err(|code| (i, code))?;
        }
        Ok(())
    }

    /// This domain's `DomainSchema`, without an instance — needed at
    /// `with_journal`'s replay. `ConnectionStore::describe` delegates
    /// here.
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
                FieldDescriptor {
                    tag: FIELD_SUBJECT,
                    name: "subject".into(),
                    value_kind: ValueKind::Str,
                    capabilities: FieldCapabilities {
                        filter_eq: true,
                        scan: false,
                        update: false,
                    },
                },
                field(FIELD_RELATION, "relation", ValueKind::Str),
                field(FIELD_OBJECT, "object", ValueKind::Str),
                field(FIELD_CREATED_AT, "created_at_unix_ms", ValueKind::I64),
                FieldDescriptor {
                    tag: FIELD_UPDATED_AT,
                    name: "updated_at_unix_ms".into(),
                    value_kind: ValueKind::I64,
                    capabilities: FieldCapabilities {
                        filter_eq: false,
                        scan: true,
                        update: true,
                    },
                },
                field(FIELD_NODE_ID, "node_id", ValueKind::Str),
                field(FIELD_DELETED_AT, "deleted_at_unix_ms", ValueKind::I64),
            ],
            relations: RelationCapabilities {
                parent_children: false,
                neighbors: false,
            },
        }
    }

    /// The validate-then-apply shape every adapter uses — the one
    /// updatable field over this protocol is `updated_at_unix_ms`.
    fn validate_batch(
        updates: &[TransactionOp],
        exists: impl Fn(RecordId) -> bool,
    ) -> Result<(), (usize, ErrorCode)> {
        for (i, op) in updates.iter().enumerate() {
            match (op.field, &op.value) {
                (FIELD_UPDATED_AT, ScanValue::I64(_)) => {
                    if !exists(op.id) {
                        return Err((i, ErrorCode::RecordNotFound));
                    }
                }
                (FIELD_UPDATED_AT, _) => return Err((i, ErrorCode::Malformed)),
                (field, _) if READ_ONLY_FIELDS.contains(&field) => {
                    return Err((i, ErrorCode::Unsupported))
                }
                _ => return Err((i, ErrorCode::UnknownField)),
            }
        }
        Ok(())
    }

    fn apply_batch(
        inner: &mut RelationProductionStack,
        updates: &[TransactionOp],
    ) -> Result<(), (usize, ErrorCode)> {
        for (i, op) in updates.iter().enumerate() {
            if let ScanValue::I64(stamp) = op.value {
                UpdateField::<Relation, UpdatedAtField>::update(inner, op.id, stamp)
                    .map_err(|_| (i, ErrorCode::RecordNotFound))?;
            }
        }
        Ok(())
    }

    /// `INS-FR-006` (ADR-0046): the whole field list against this
    /// domain's schema, before any write — all seven tags exactly once
    /// with a value of its kind; `subject`, `relation`, and `object`
    /// non-empty; `deleted_at_unix_ms` non-negative. `Malformed` for a
    /// missing, repeated, wrong-kind, or out-of-rule field;
    /// `UnknownField` for a tag this domain doesn't have.
    fn relation_from_fields(
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<Relation, ErrorCode> {
        let mut subject = None;
        let mut relation = None;
        let mut object = None;
        let mut created_at = None;
        let mut updated_at = None;
        let mut node_id = None;
        let mut deleted_at = None;
        for (tag, value) in fields {
            match (tag, value) {
                (FIELD_SUBJECT, ScanValue::Str(v)) if subject.is_none() && !v.is_empty() => {
                    subject = Some(v)
                }
                (FIELD_RELATION, ScanValue::Str(v)) if relation.is_none() && !v.is_empty() => {
                    relation = Some(v)
                }
                (FIELD_OBJECT, ScanValue::Str(v)) if object.is_none() && !v.is_empty() => {
                    object = Some(v)
                }
                (FIELD_CREATED_AT, ScanValue::I64(v)) if created_at.is_none() => {
                    created_at = Some(v)
                }
                (FIELD_UPDATED_AT, ScanValue::I64(v)) if updated_at.is_none() => {
                    updated_at = Some(v)
                }
                (FIELD_NODE_ID, ScanValue::Str(v)) if node_id.is_none() => node_id = Some(v),
                (FIELD_DELETED_AT, ScanValue::I64(v)) if deleted_at.is_none() && v >= 0 => {
                    deleted_at = Some(v)
                }
                (tag, _) if tag <= FIELD_DELETED_AT => return Err(ErrorCode::Malformed),
                _ => return Err(ErrorCode::UnknownField),
            }
        }
        let (
            Some(subject),
            Some(relation),
            Some(object),
            Some(created_at_unix_ms),
            Some(updated_at_unix_ms),
            Some(node_id),
            Some(deleted_at_unix_ms),
        ) = (
            subject, relation, object, created_at, updated_at, node_id, deleted_at,
        )
        else {
            return Err(ErrorCode::Malformed);
        };
        Ok(Relation {
            id,
            subject,
            relation,
            object,
            created_at_unix_ms,
            updated_at_unix_ms,
            node_id,
            deleted_at_unix_ms,
        })
    }

    fn check_read_set(
        reads: &[(RecordId, FieldRef, ScanValue)],
        get: impl Fn(RecordId) -> Option<Relation>,
    ) -> Result<(), (usize, ErrorCode)> {
        for (id, field, value) in reads {
            let current = get(*id).and_then(|relation| {
                Self::fields_of(relation)
                    .into_iter()
                    .find(|(tag, _)| tag == field)
                    .map(|(_, value)| value)
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
        mvcc.state.with_index(|index| {
            for op in updates {
                index.record_write((op.id, op.field), txn_id, Some(op.value.clone()));
            }
        });
    }

    /// `ADR-0072`'s `MVCC2-FR-002`/`008` — see `MemoryConnectionStore::
    /// mvcc_record_write_ops` for the full contract; identical here,
    /// minus `Link` (`Relation` has no edge layer of its own — `WriteOp::
    /// Link` is already `Unsupported` in `prepare_write`, so it never
    /// reaches here as a real result).
    fn mvcc_record_write_ops(&self, ops: &[WriteOp], results: &[WriteResult]) {
        let Some(mvcc) = &self.mvcc else { return };
        if !mvcc.state.is_active() {
            return;
        }
        let writes: Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> = ops
            .iter()
            .zip(results)
            .filter_map(|(op, result)| match (op, result) {
                (WriteOp::Insert { id, fields }, WriteResult::Inserted)
                | (WriteOp::Replace { id, fields }, WriteResult::Replaced)
                | (WriteOp::ReplaceIf { id, fields, .. }, WriteResult::Replaced) => {
                    Some((*id, fields.clone()))
                }
                (WriteOp::Delete { id }, WriteResult::Deleted) => Some((*id, Vec::new())),
                _ => None,
            })
            .collect();
        if writes.is_empty() {
            return;
        }
        let deletes: std::collections::HashSet<RecordId> = ops
            .iter()
            .zip(results)
            .filter_map(|(op, result)| match (op, result) {
                (WriteOp::Delete { id }, WriteResult::Deleted) => Some(*id),
                _ => None,
            })
            .collect();
        let txn_id = mvcc.state.counter().next();
        mvcc.state.with_index(|index| {
            for (id, fields) in writes {
                if deletes.contains(&id) {
                    index.record_write((id, mvcc::EXISTENCE_FIELD), txn_id, None);
                    continue;
                }
                index.record_write(
                    (id, mvcc::EXISTENCE_FIELD),
                    txn_id,
                    Some(ScanValue::Bool(true)),
                );
                for (tag, value) in fields {
                    index.record_write((id, tag), txn_id, Some(value));
                }
            }
        });
    }

    /// `ADR-0072`'s `MVCC2-FR-010` — see `MemoryConnectionStore::
    /// mvcc_flush_now` for the full contract; identical here.
    fn mvcc_flush_now(&self, journal_entries: u64) -> bool {
        let Some(mvcc) = &self.mvcc else { return true };
        if !mvcc.state.is_active() {
            return true;
        }
        let insert_log_count = insert_log::read_entries::<Relation, RecordId>(
            &insert_log::log_path(&mvcc.mmap_path),
            Relation::SCHEMA_TAG,
        )
        .map(|entries| entries.len())
        .unwrap_or(0);
        mvcc.state
            .flush(&mvcc.mmap_path, insert_log_count, journal_entries as usize)
            .is_ok()
    }
}

impl RelationConnectionStore {
    /// The wire shape of one relation, in tag order — `get`, the read-set
    /// check, and a `ReplaceIf` guard's evaluation all go through here.
    fn fields_of(relation: Relation) -> Vec<(FieldRef, ScanValue)> {
        vec![
            (FIELD_SUBJECT, ScanValue::Str(relation.subject)),
            (FIELD_RELATION, ScanValue::Str(relation.relation)),
            (FIELD_OBJECT, ScanValue::Str(relation.object)),
            (
                FIELD_CREATED_AT,
                ScanValue::I64(relation.created_at_unix_ms),
            ),
            (
                FIELD_UPDATED_AT,
                ScanValue::I64(relation.updated_at_unix_ms),
            ),
            (FIELD_NODE_ID, ScanValue::Str(relation.node_id)),
            (
                FIELD_DELETED_AT,
                ScanValue::I64(relation.deleted_at_unix_ms),
            ),
        ]
    }
}

/// `WBT-FR-003` (ADR-0060): a parsed, pre-validated write of an atomic
/// [`WriteOp`] batch on `Relation`. `Relation` is a record table with no
/// edge layer, so `Link` is not representable here — an atomic batch
/// carrying one aborts `Unsupported` in `prepare_write`.
enum PreparedWrite {
    Insert(Relation),
    Replace(Relation),
    ReplaceIf(Relation, Predicate),
    Delete(RecordId),
}

impl RelationConnectionStore {
    fn prepare_write(schema: &DomainSchema, op: &WriteOp) -> Result<PreparedWrite, ErrorCode> {
        Ok(match op {
            WriteOp::Insert { id, fields } => {
                PreparedWrite::Insert(Self::relation_from_fields(*id, fields.clone())?)
            }
            WriteOp::Replace { id, fields } => {
                PreparedWrite::Replace(Self::relation_from_fields(*id, fields.clone())?)
            }
            WriteOp::ReplaceIf { id, fields, guard } => {
                validate_predicate(schema, guard)?;
                PreparedWrite::ReplaceIf(
                    Self::relation_from_fields(*id, fields.clone())?,
                    guard.clone(),
                )
            }
            WriteOp::Delete { id } => PreparedWrite::Delete(*id),
            WriteOp::Link { .. } => return Err(ErrorCode::Unsupported),
        })
    }

    fn apply_prepared(
        inner: &mut RelationProductionStack,
        prepared: PreparedWrite,
    ) -> Result<WriteResult, ErrorCode> {
        Ok(match prepared {
            PreparedWrite::Insert(relation) => match Insert::insert(inner, relation) {
                Ok(()) => WriteResult::Inserted,
                Err(InsertError::Duplicate(_)) => WriteResult::Duplicate,
                Err(InsertError::Durability(_)) => return Err(ErrorCode::Storage),
            },
            PreparedWrite::Replace(relation) => match Replace::replace(inner, relation) {
                Ok(()) => WriteResult::Replaced,
                Err(ReplaceError::NotFound(_)) => WriteResult::NotFound,
                Err(ReplaceError::Durability(_)) => return Err(ErrorCode::Storage),
            },
            PreparedWrite::ReplaceIf(relation, guard) => {
                let id = relation.id;
                match GetById::<Relation>::get(inner, id) {
                    None => WriteResult::NotFound,
                    Some(stored) => {
                        if predicate_matches(&Self::fields_of(stored), &guard) {
                            match Replace::replace(inner, relation) {
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
            PreparedWrite::Delete(id) => match Delete::<Relation>::delete(inner, id) {
                Ok(()) => WriteResult::Deleted,
                Err(DeleteError::NotFound(_)) => WriteResult::NotFound,
                Err(DeleteError::Durability(_)) => return Err(ErrorCode::Storage),
            },
        })
    }
}

impl ConnectionStore for RelationConnectionStore {
    fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>> {
        self.store.get::<Relation>(id).map(Self::fields_of)
    }

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
        let apply = |inner: &mut RelationProductionStack| {
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

    /// `SQL-FR-004`/`SQL-FR-005` (ADR-0034): every id from `all_ids`,
    /// each mapped through this adapter's own `get`.
    fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
        self.store
            .all_ids::<Relation>()
            .into_iter()
            .filter_map(|id| self.get(id).map(|fields| (id, fields)))
            .collect()
    }

    /// `ORD-FR-005` (ADR-0059): a page ordered by `updated_at_unix_ms`
    /// is a range walk of the stack's sorted index; any other orderable
    /// field takes the scan path. A cursor here is `I64` or absent
    /// (`validate_page`).
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
            .page_by::<Relation, UpdatedAtField>(cursor, limit)
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
            .page_by_desc::<Relation, UpdatedAtField>(cursor, limit)
            .into_iter()
            .filter_map(|id| self.get(id).map(|fields| (id, fields)))
            .collect())
    }

    /// `PAG-FR-002` (ADR-0055): the sort key of every record read
    /// straight off [`Relation`], so a page materializes only its own rows.
    /// A field this arm list does not name falls back to the wire shape
    /// — the same key [`page_key`] derives for the trait default.
    fn page_keys(&self, order_by: FieldRef) -> Vec<(RecordId, i128)> {
        self.store
            .all_ids::<Relation>()
            .into_iter()
            .filter_map(|id| {
                let record = self.store.get::<Relation>(id)?;
                let key = match order_by {
                    FIELD_CREATED_AT => Some(i128::from(record.created_at_unix_ms)),
                    FIELD_UPDATED_AT => Some(i128::from(record.updated_at_unix_ms)),
                    FIELD_DELETED_AT => Some(i128::from(record.deleted_at_unix_ms)),
                    _ => None,
                };
                let key = key.unwrap_or_else(|| page_key(&Self::fields_of(record), order_by, id).0);
                Some((id, key))
            })
            .collect()
    }

    /// `FPW-FR-001`–`003` (ADR-0076), `FPM-FR-001`–`003` (ADR-0077): see
    /// `MemoryConnectionStore::filtered_page`; identical here over
    /// `UpdatedAtField`, `subject` the declared index equality-first
    /// keeps.
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
            |start, limit| self.store.page_by::<Relation, UpdatedAtField>(start, limit),
            order_by,
            after,
            limit,
            filter,
        )
    }

    /// `QPR-FR-002` (ADR-0075): the field [`RelationProductionStack`]'s
    /// sorted index is over (`ORD-FR-004`).
    fn range_field(&self) -> Option<FieldRef> {
        Some(FIELD_UPDATED_AT)
    }

    /// `QPR-FR-002` (ADR-0075): a `WHERE` range on `updated_at_unix_ms`
    /// is a walk of the stack's sorted index between the two bounds —
    /// see `MemoryConnectionStore::range_ids`; identical here over
    /// `UpdatedAtField`.
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
        Ok(self
            .store
            .range_by::<Relation, UpdatedAtField>(lower, upper))
    }

    /// `QPB-FR-002` (ADR-0079): see `MemoryConnectionStore::
    /// range_ids_limited`; identical here over `UpdatedAtField`.
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
            .range_count::<Relation, UpdatedAtField>(lower, upper) as u64)
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
            .range_keys::<Relation, UpdatedAtField>(lower, upper))
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
        Ok(self.store.range_fold::<Relation, UpdatedAtField, _, _>(
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
            .range_by_limited::<Relation, UpdatedAtField>(lower, upper, limit))
    }

    fn filter_eq(&self, field: FieldRef, value: &ScanValue) -> Result<Vec<RecordId>, ErrorCode> {
        match (field, value) {
            (FIELD_SUBJECT, ScanValue::Str(subject)) => {
                Ok(self.store.filter_eq::<Relation, SubjectField>(subject))
            }
            (FIELD_SUBJECT, _) => Err(ErrorCode::Malformed),
            (field, _) if field <= FIELD_DELETED_AT => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    fn scan_field(&self, field: FieldRef) -> Result<Vec<ScanValue>, ErrorCode> {
        match field {
            FIELD_UPDATED_AT => Ok(self
                .store
                .scan::<Relation, UpdatedAtField>()
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
            (FIELD_UPDATED_AT, ScanValue::I64(stamp)) => {
                match self.store.update::<Relation, UpdatedAtField>(id, stamp) {
                    Ok(()) => {
                        self.sync_update_ack()?;
                        Ok(true)
                    }
                    Err(_not_found) => Ok(false),
                }
            }
            (FIELD_UPDATED_AT, _) => Err(ErrorCode::Malformed),
            (field, _) if READ_ONLY_FIELDS.contains(&field) => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    /// `INS-FR-006` (ADR-0046): validate, then one write under the
    /// store's own lock. A duplicate is the normal outcome, not an
    /// error; a durability failure is `Storage` (see the trait's docs).
    fn insert_record(
        &self,
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<InsertOutcome, ErrorCode> {
        let relation = Self::relation_from_fields(id, fields)?;
        self.store
            .with_exclusive(|inner| match Insert::insert(inner, relation) {
                Ok(()) => {
                    self.mvcc_record_write(
                        &Self::fields_of(
                            GetById::<Relation>::get(inner, id).expect("just inserted"),
                        ),
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
    /// then one whole-record write under the store's own lock. An
    /// unknown id is the normal outcome, not an error.
    fn replace_record(
        &self,
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<ReplaceOutcome, ErrorCode> {
        let relation = Self::relation_from_fields(id, fields)?;
        self.store
            .with_exclusive(|inner| match Replace::replace(inner, relation) {
                Ok(()) => {
                    self.mvcc_record_write(
                        &Self::fields_of(
                            GetById::<Relation>::get(inner, id).expect("just replaced"),
                        ),
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
        let relation = Self::relation_from_fields(id, fields)?;
        self.store.with_exclusive(|inner| {
            let Some(current) = GetById::<Relation>::get(inner, id) else {
                return Ok(ReplaceIfOutcome::NotFound);
            };
            if !predicate_matches(&Self::fields_of(current), guard) {
                return Ok(ReplaceIfOutcome::GuardFailed);
            }
            match Replace::replace(inner, relation) {
                Ok(()) => {
                    self.mvcc_record_write(
                        &Self::fields_of(
                            GetById::<Relation>::get(inner, id).expect("just replaced"),
                        ),
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
            .with_exclusive(|inner| match Delete::<Relation>::delete(inner, id) {
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

    /// `REL-FR-003`: a *table of edges* has no edge layer of its own —
    /// out-edges are `FilterEq subject`, in-edges `Query object = …`.
    fn parent(&self, _id: RecordId) -> Result<ParentLookup, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn children(&self, _id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn neighbors(&self, _id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn neighbors_by_relation(
        &self,
        _id: RecordId,
        _relation: &str,
    ) -> Result<Vec<RecordId>, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn list_relation_kinds(&self) -> Vec<String> {
        Vec::new()
    }

    /// `STV-FR-002`: `validate_batch` on this one operation, with the
    /// same per-call existence read the journaled path uses.
    fn validate_op(&self, op: &TransactionOp) -> Result<(), ErrorCode> {
        Self::validate_batch(std::slice::from_ref(op), |id| {
            self.store.get::<Relation>(id).is_some()
        })
        .map_err(|(_, code)| code)
    }

    fn table_name(&self) -> &str {
        "relation"
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
        // (`GRP-FR-001`–`005`) and where the read-set check runs in each;
        // identical here.
        match &self.journal {
            None => self.store.with_exclusive(|inner| {
                Self::validate_batch(updates, |id| GetById::<Relation>::get(inner, id).is_some())?;
                Self::check_read_set(read_set, |id| GetById::<Relation>::get(inner, id))?;
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
                Self::validate_batch(updates, |id| self.store.get::<Relation>(id).is_some())?;
                journal
                    .commit(updates, |turn| {
                        self.store.with_exclusive(|inner| {
                            Self::check_read_set(read_set, |id| {
                                GetById::<Relation>::get(inner, id)
                            })?;
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
                Self::validate_batch(updates, |id| GetById::<Relation>::get(inner, id).is_some())?;
                Self::check_read_set(read_set, |id| GetById::<Relation>::get(inner, id))?;
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
                Self::validate_batch(updates, |id| self.store.get::<Relation>(id).is_some())?;
                journal
                    .commit(updates, |turn| {
                        self.store.with_exclusive(|inner| {
                            Self::check_read_set(read_set, |id| {
                                GetById::<Relation>::get(inner, id)
                            })?;
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

    /// `ADR-0072`'s `MVCC2-FR-012`: `Relation` implements real MVCC.
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
                for id in AllIds::<Relation>::all_ids(inner) {
                    if let Some(record) = GetById::<Relation>::get(inner, id) {
                        let fields = Self::fields_of(record);
                        mvcc.state.with_index(|index| {
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
        let snapshot_txn = mvcc.state.with_index(|index| index.last_committed());
        mvcc.state.open_snapshots().register(snapshot_txn);
        snapshot_txn
    }

    fn mvcc_release(&self, snapshot_txn: u64) {
        if let Some(mvcc) = &self.mvcc {
            mvcc.state.open_snapshots().deregister(snapshot_txn);
        }
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
    use crate::generic::relation::{
        create_relation_production_stack, open_relation_production_stack_portable,
    };
    use crate::test_support::fresh_temp_dir;
    use uuid::Uuid;

    fn sample() -> Vec<Relation> {
        let relation = |n: u128, subject: &str, label: &str, object: &str| Relation {
            id: Uuid::from_u128(n),
            subject: subject.into(),
            relation: label.into(),
            object: object.into(),
            created_at_unix_ms: 1_000 * n as i64,
            updated_at_unix_ms: 1_000 * n as i64,
            node_id: String::new(),
            deleted_at_unix_ms: 0,
        };
        vec![
            relation(1, "aaa", "works_with", "bbb"),
            relation(2, "aaa", "located_in", "ccc"),
            relation(3, "bbb", "works_with", "aaa"),
        ]
    }

    fn sample_adapter() -> RelationConnectionStore {
        let dir = fresh_temp_dir("server_relation_adapter").unwrap();
        let stack =
            create_relation_production_stack(sample(), &dir.join("relations.mmap")).unwrap();
        RelationConnectionStore::new(GenericProductionStore::new(stack))
    }

    fn sample_adapter_with_mvcc() -> (RelationConnectionStore, std::path::PathBuf) {
        let dir = fresh_temp_dir("server_relation_mvcc_adapter").unwrap();
        let path = dir.join("relations.mmap");
        let stack = create_relation_production_stack(sample(), &path).unwrap();
        let adapter = RelationConnectionStore::new(GenericProductionStore::new(stack))
            .with_mvcc(&path)
            .unwrap();
        (adapter, path)
    }

    /// `ADR-0072`: activation baseline-seeds a pre-existing record; a
    /// snapshot survives a concurrent ordinary `Replace` and a concurrent
    /// atomic `WriteBatch`; a key created after the snapshot began stays
    /// invisible; a session commit conflicts against a concurrent
    /// ordinary write. Mirrors `MemoryConnectionStore`'s own MVCC test
    /// suite — see that module for the per-scenario rationale.
    #[test]
    fn mvcc_activation_snapshot_isolation_and_conflict_detection() {
        let (adapter, _path) = sample_adapter_with_mvcc();
        assert!(adapter.mvcc_supported());
        let id = Uuid::from_u128(1);
        let before = adapter.mvcc_begin();
        assert_eq!(
            adapter.mvcc_get(id, before).unwrap(),
            adapter.get(id),
            "the baseline sees the pre-existing record"
        );

        let full = |subject: &str, updated_at: i64| {
            vec![
                (FIELD_SUBJECT, ScanValue::Str(subject.into())),
                (FIELD_RELATION, ScanValue::Str("works_with".into())),
                (FIELD_OBJECT, ScanValue::Str("bbb".into())),
                (FIELD_CREATED_AT, ScanValue::I64(1_000)),
                (FIELD_UPDATED_AT, ScanValue::I64(updated_at)),
                (FIELD_NODE_ID, ScanValue::Str(String::new())),
                (FIELD_DELETED_AT, ScanValue::I64(0)),
            ]
        };
        assert_eq!(
            adapter.replace_record(id, full("edited", 2_000)),
            Ok(ReplaceOutcome::Replaced)
        );
        assert_eq!(
            adapter.mvcc_get(id, before).unwrap(),
            Some(full("aaa", 1_000)),
            "the snapshot still sees the pre-replace value"
        );

        let new_id = Uuid::from_u128(99);
        assert_eq!(
            adapter.insert_record(new_id, full("new", 3_000)),
            Ok(InsertOutcome::Inserted)
        );
        assert_eq!(adapter.mvcc_get(new_id, before).unwrap(), None);

        let after = adapter.mvcc_begin();
        assert_eq!(
            adapter.mvcc_get(id, after).unwrap(),
            Some(full("edited", 2_000))
        );
        assert!(adapter.mvcc_get(new_id, after).unwrap().is_some());

        // A session commit against the stale `before` snapshot conflicts
        // with the ordinary replace made above.
        assert_eq!(
            adapter.apply_transaction_mvcc(
                &[TransactionOp {
                    id,
                    field: FIELD_UPDATED_AT,
                    value: ScanValue::I64(9_000),
                }],
                &[],
                before,
            ),
            Err((0, ErrorCode::Conflict))
        );
        // The same commit against the fresh snapshot succeeds.
        assert_eq!(
            adapter.apply_transaction_mvcc(
                &[TransactionOp {
                    id,
                    field: FIELD_UPDATED_AT,
                    value: ScanValue::I64(9_000),
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
    /// `Delete` are recorded too.
    #[test]
    fn mvcc_snapshot_survives_a_concurrent_atomic_write_batch() {
        let (adapter, _path) = sample_adapter_with_mvcc();
        let (existing, deleted) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let before = adapter.mvcc_begin();
        let before_existing = adapter.get(existing).unwrap();
        let before_deleted = adapter.get(deleted).unwrap();

        let mut edited = before_existing.clone();
        edited[1] = (FIELD_RELATION, ScanValue::Str("edited".into()));
        let ops = vec![
            WriteOp::Replace {
                id: existing,
                fields: edited.clone(),
            },
            WriteOp::Delete { id: deleted },
        ];
        assert_eq!(
            adapter.write_batch(&ops, true),
            Ok(vec![WriteResult::Replaced, WriteResult::Deleted])
        );
        assert_eq!(
            adapter.mvcc_get(existing, before).unwrap(),
            Some(before_existing)
        );
        assert_eq!(
            adapter.mvcc_get(deleted, before).unwrap(),
            Some(before_deleted)
        );

        let after = adapter.mvcc_begin();
        assert_eq!(adapter.mvcc_get(existing, after).unwrap(), Some(edited));
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
        edited[1] = (FIELD_RELATION, ScanValue::Str("edited".into()));
        assert_eq!(
            adapter.replace_record(id, edited.clone()),
            Ok(ReplaceOutcome::Replaced)
        );
        adapter.compact().unwrap();
        drop(adapter);

        let stack = open_relation_production_stack_portable(&path).unwrap();
        let reopened = RelationConnectionStore::new(GenericProductionStore::new(stack))
            .with_mvcc(&path)
            .unwrap();
        assert_eq!(reopened.mvcc_get(id, before).unwrap(), Some(original));
        let after = reopened.mvcc_begin();
        assert_eq!(reopened.mvcc_get(id, after).unwrap(), Some(edited));
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
        edited[1] = (
            FIELD_RELATION,
            ScanValue::Str("edited, never flushed".into()),
        );
        assert_eq!(
            adapter.replace_record(id, edited.clone()),
            Ok(ReplaceOutcome::Replaced),
            "an ordinary write — appends to the insert log, no Compact/flush follows"
        );
        drop(adapter);

        let reopened = RelationConnectionStore::open_with_mvcc(&path).unwrap();
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

        let full = |subject: &str| {
            vec![
                (FIELD_SUBJECT, ScanValue::Str(subject.into())),
                (FIELD_RELATION, ScanValue::Str("works_with".into())),
                (FIELD_OBJECT, ScanValue::Str("bbb".into())),
                (FIELD_CREATED_AT, ScanValue::I64(1_000)),
                (FIELD_UPDATED_AT, ScanValue::I64(1_000)),
                (FIELD_NODE_ID, ScanValue::Str(String::new())),
                (FIELD_DELETED_AT, ScanValue::I64(0)),
            ]
        };

        let new_id = Uuid::from_u128(99);
        assert_eq!(
            adapter.insert_record(new_id, full("new")),
            Ok(InsertOutcome::Inserted)
        );
        assert_eq!(
            read_field(new_id, mvcc::EXISTENCE_FIELD),
            Some(ScanValue::Bool(true)),
            "insert_record must flush to disk immediately, not only at Compact"
        );

        let id = Uuid::from_u128(1);
        assert_eq!(
            adapter.replace_record(id, full("edited")),
            Ok(ReplaceOutcome::Replaced)
        );
        assert_eq!(
            read_field(id, FIELD_SUBJECT),
            Some(ScanValue::Str("edited".into())),
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

        let new_id = Uuid::from_u128(99);
        let ops = vec![WriteOp::Insert {
            id: new_id,
            fields: vec![
                (FIELD_SUBJECT, ScanValue::Str("new".into())),
                (FIELD_RELATION, ScanValue::Str("works_with".into())),
                (FIELD_OBJECT, ScanValue::Str("bbb".into())),
                (FIELD_CREATED_AT, ScanValue::I64(1_000)),
                (FIELD_UPDATED_AT, ScanValue::I64(1_000)),
                (FIELD_NODE_ID, ScanValue::Str(String::new())),
                (FIELD_DELETED_AT, ScanValue::I64(0)),
            ],
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
                    field: FIELD_UPDATED_AT,
                    value: ScanValue::I64(9_000),
                }],
                &[],
            ),
            Ok(())
        );
        drop(adapter);

        let reopened = RelationConnectionStore::open_with_mvcc(&path).unwrap();
        assert!(reopened.mvcc_supported());
        assert_eq!(
            reopened.mvcc_get(id, before).unwrap().unwrap()[4],
            (FIELD_UPDATED_AT, ScanValue::I64(1_000)),
            "the pre-commit snapshot still sees the pre-commit value after reopen"
        );
        let after = reopened.mvcc_begin();
        assert_eq!(
            reopened.mvcc_get(id, after).unwrap().unwrap()[4],
            (FIELD_UPDATED_AT, ScanValue::I64(9_000)),
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
                    field: FIELD_UPDATED_AT,
                    value: ScanValue::I64(9_000),
                }],
                &[],
                before,
            ),
            Ok(())
        );
        drop(adapter);

        let reopened = RelationConnectionStore::open_with_mvcc(&path).unwrap();
        assert_eq!(
            reopened.mvcc_get(id, before).unwrap().unwrap()[4],
            (FIELD_UPDATED_AT, ScanValue::I64(1_000)),
            "the pre-commit snapshot still sees the pre-commit value after reopen"
        );
        let after = reopened.mvcc_begin();
        assert_eq!(
            reopened.mvcc_get(id, after).unwrap().unwrap()[4],
            (FIELD_UPDATED_AT, ScanValue::I64(9_000)),
            "a fresh snapshot after reopen sees the commit apply_transaction_mvcc flushed"
        );
    }

    /// `REL-FR-004`: seven fields in tag order, `subject` the one
    /// `filter_eq`, `updated_at_unix_ms` the one scan/update, no relations.
    #[test]
    fn describe_get_filter_scan_and_update_match_the_shape() {
        let adapter = sample_adapter();
        let schema = adapter.describe();
        assert_eq!(schema.fields.len(), 7);
        let names: Vec<&str> = schema.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "subject",
                "relation",
                "object",
                "created_at_unix_ms",
                "updated_at_unix_ms",
                "node_id",
                "deleted_at_unix_ms"
            ]
        );
        assert!(schema.fields[0].capabilities.filter_eq);
        assert!(schema.fields[4].capabilities.scan && schema.fields[4].capabilities.update);
        assert!(!schema.relations.neighbors && !schema.relations.parent_children);

        let fields = adapter.get(Uuid::from_u128(1)).unwrap();
        assert_eq!(fields[0], (FIELD_SUBJECT, ScanValue::Str("aaa".into())));
        assert_eq!(fields[6], (FIELD_DELETED_AT, ScanValue::I64(0)));
        let mut out = adapter
            .filter_eq(FIELD_SUBJECT, &ScanValue::Str("aaa".into()))
            .unwrap();
        out.sort();
        assert_eq!(out, vec![Uuid::from_u128(1), Uuid::from_u128(2)]);
        assert_eq!(
            adapter.filter_eq(FIELD_OBJECT, &ScanValue::Str("aaa".into())),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(adapter.scan_field(FIELD_UPDATED_AT).unwrap().len(), 3);
        assert_eq!(
            adapter.update_field(Uuid::from_u128(1), FIELD_UPDATED_AT, ScanValue::I64(9_000)),
            Ok(true)
        );
        assert_eq!(
            adapter.get(Uuid::from_u128(1)).unwrap()[4],
            (FIELD_UPDATED_AT, ScanValue::I64(9_000))
        );
        assert_eq!(
            adapter.neighbors(Uuid::from_u128(1)),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(adapter.table_name(), "relation");
    }

    /// `REL-FR-004`: insert validates every field and the three rules
    /// (non-empty endpoints and label, non-negative stamp); replace,
    /// guarded replace, and delete behave as every front-door domain.
    #[test]
    fn insert_replace_replace_if_and_delete_apply_the_rules() {
        use crate::server::protocol::{CompareOp, Predicate};
        let adapter = sample_adapter();
        let id = Uuid::from_u128(4);
        let full = |subject: &str, updated_at: i64| {
            vec![
                (FIELD_SUBJECT, ScanValue::Str(subject.into())),
                (FIELD_RELATION, ScanValue::Str("part_of".into())),
                (FIELD_OBJECT, ScanValue::Str("aaa".into())),
                (FIELD_CREATED_AT, ScanValue::I64(4_000)),
                (FIELD_UPDATED_AT, ScanValue::I64(updated_at)),
                (FIELD_NODE_ID, ScanValue::Str("laptop".into())),
                (FIELD_DELETED_AT, ScanValue::I64(0)),
            ]
        };
        assert_eq!(
            adapter.insert_record(id, full("ccc", 4_000)),
            Ok(InsertOutcome::Inserted)
        );
        assert_eq!(adapter.get(id).unwrap(), full("ccc", 4_000));
        assert_eq!(
            adapter.insert_record(id, full("ccc", 4_000)),
            Ok(InsertOutcome::Duplicate)
        );
        assert_eq!(
            adapter.insert_record(Uuid::from_u128(5), full("", 5_000)),
            Err(ErrorCode::Malformed),
            "an empty subject"
        );
        let mut negative = full("ccc", 5_000);
        negative[6] = (FIELD_DELETED_AT, ScanValue::I64(-1));
        assert_eq!(
            adapter.insert_record(Uuid::from_u128(5), negative),
            Err(ErrorCode::Malformed)
        );
        assert_eq!(
            adapter.insert_record(Uuid::from_u128(5), full("ccc", 5_000)[..6].to_vec()),
            Err(ErrorCode::Malformed),
            "a missing field"
        );
        assert!(adapter.get(Uuid::from_u128(5)).is_none(), "nothing written");

        assert_eq!(
            adapter.replace_record(id, full("ddd", 6_000)),
            Ok(ReplaceOutcome::Replaced)
        );
        assert_eq!(
            adapter.filter_eq(FIELD_SUBJECT, &ScanValue::Str("ddd".into())),
            Ok(vec![id]),
            "the index moved"
        );
        let lww = |mine: i64| Predicate {
            field: FIELD_UPDATED_AT,
            op: CompareOp::Lt,
            value: ScanValue::I64(mine),
        };
        assert_eq!(
            adapter.replace_record_if(id, full("ddd", 5_000), &lww(5_000)),
            Ok(ReplaceIfOutcome::GuardFailed)
        );
        assert_eq!(
            adapter.replace_record_if(id, full("ddd", 7_000), &lww(7_000)),
            Ok(ReplaceIfOutcome::Replaced)
        );
        assert_eq!(adapter.delete_record(id), Ok(DeleteOutcome::Deleted));
        assert_eq!(adapter.delete_record(id), Ok(DeleteOutcome::NotFound));
    }
}
