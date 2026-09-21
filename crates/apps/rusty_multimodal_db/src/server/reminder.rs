//! [`ConnectionStore`] adapter wrapping
//! [`crate::generic::production::GenericProductionStore<ReminderProductionStack>`]
//! for `Reminder` — this crate's fourth domain, and its first
//! `ConnectionStore` adapter gated by `server` alone, not `server` +
//! `research` (`RMD-FR-006`, ADR-0036): `Reminder` is real, deployable
//! capability, not reference material validating the generic schema
//! library, so unlike `server::order`/`server::employee` it needs no
//! `research` feature to reach.
//!
//! # No relation of either kind
//!
//! `Reminder` has no `ChildOf`/`SymmetricRelation` impl at all — the
//! one combination no existing adapter has (`server::dog`: neighbors
//! only; `server::order`: parent/children only; `server::employee`:
//! both). `parent`/`children`/`neighbors` all report
//! `ErrorCode::Unsupported` unconditionally, the identical shape
//! `server::dog`'s own missing half already uses.
//!
//! # `status`, not a plain number, is the durably-mutable field
//!
//! Every existing domain's `ScannableField` has been a plain number
//! (`Amount`/`SalaryCents`); `Reminder`'s is `status`, an enum
//! discriminant (`RMD-FR-002`). `scan_field`/`update_field` validate
//! the incoming `u32` against [`crate::generic::reminder::status_from_u32`]
//! before ever reaching the store — `ErrorCode::Malformed` on an
//! unrecognized value, nothing applied — the one genuinely new
//! validation path this domain needs beyond the mechanical repetition
//! every existing adapter already has.

use super::journal::{CheckpointFlush, CommitError, CommitGroup, JournalError, JournaledBatch};
use super::protocol::{
    DomainSchema, ErrorCode, FieldCapabilities, FieldDescriptor, FieldRef, ParentLookup, Predicate,
    RecordId, RelationCapabilities, ScanValue, TransactionOp, ValueKind,
};
use super::{
    bounded_filtered_page, bounded_walk_applies, filtered_page_by_candidates, page_by_scan,
    page_by_scan_desc, predicate_matches, uuid_pair_bounds, ConnectionStore, DeleteOutcome,
    InsertOutcome, KeyStats, PageRow, ReplaceIfOutcome, ReplaceOutcome,
};
use crate::generic::production::GenericProductionStore;
use crate::generic::query::{GetById, UpdateField};
use crate::generic::reminder::{
    status_from_u32, status_to_u32, DueAtField, DueAtOrder, Reminder, ReminderProductionStack,
    StatusField,
};
use crate::generic::{DeleteError, GuardedReplace, InsertError, ReplaceError};
use std::ops::Bound;
use std::path::Path;

pub const FIELD_TITLE: FieldRef = 0;
pub const FIELD_DUE_AT: FieldRef = 1;
pub const FIELD_STATUS: FieldRef = 2;

pub struct ReminderConnectionStore {
    store: GenericProductionStore<ReminderProductionStack>,
    /// `JRN-FR-001` (ADR-0025) — see `DogConnectionStore::with_journal`.
    journal: Option<CommitGroup>,
}

impl ReminderConnectionStore {
    pub fn new(store: GenericProductionStore<ReminderProductionStack>) -> Self {
        Self {
            store,
            journal: None,
        }
    }

    /// The crash-atomic variant — see `DogConnectionStore::with_journal`
    /// for the contract; identical here.
    pub fn with_journal(
        store: GenericProductionStore<ReminderProductionStack>,
        journal_path: &Path,
    ) -> Result<Self, JournalError> {
        let (journal, batches) = CommitGroup::open(journal_path)?;
        store.with_exclusive(|inner| -> Result<(), JournalError> {
            for (batch_index, batch) in batches.iter().enumerate() {
                let JournaledBatch::Transaction(ops) = batch else {
                    return Err(JournalError::Format(format!(
                        "entry {batch_index}: this adapter's journal cannot replay a write batch"
                    )));
                };
                Self::apply_batch(inner, ops).map_err(|(index, code)| JournalError::Replay {
                    batch: batch_index,
                    index,
                    code,
                })?;
            }
            inner.checkpoint_flush()?;
            journal.truncate()
        })?;
        Ok(Self {
            store,
            journal: Some(journal),
        })
    }

    /// Same validate-then-apply shape `server::dog`'s own uses —
    /// `Reminder`'s only mutable field over this protocol is `status`,
    /// validated against its four known discriminants
    /// (`status_from_u32`) before any write, the same "reject before
    /// any write" posture every existing domain's `validate_batch`
    /// already guarantees, now covering an enum-typed update for the
    /// first time.
    fn validate_batch(
        updates: &[TransactionOp],
        exists: impl Fn(RecordId) -> bool,
    ) -> Result<(), (usize, ErrorCode)> {
        for (i, op) in updates.iter().enumerate() {
            match (op.field, &op.value) {
                (FIELD_STATUS, ScanValue::U32(raw)) => {
                    if status_from_u32(*raw).is_none() {
                        return Err((i, ErrorCode::Malformed));
                    }
                    if !exists(op.id) {
                        return Err((i, ErrorCode::RecordNotFound));
                    }
                }
                (FIELD_STATUS, _) => return Err((i, ErrorCode::Malformed)),
                (FIELD_TITLE | FIELD_DUE_AT, _) => return Err((i, ErrorCode::Unsupported)),
                _ => return Err((i, ErrorCode::UnknownField)),
            }
        }
        Ok(())
    }

    fn apply_batch(
        inner: &mut ReminderProductionStack,
        updates: &[TransactionOp],
    ) -> Result<(), (usize, ErrorCode)> {
        for (i, op) in updates.iter().enumerate() {
            if let ScanValue::U32(status) = op.value {
                UpdateField::<Reminder, StatusField>::update(inner, op.id, status)
                    .map_err(|_| (i, ErrorCode::RecordNotFound))?;
            }
        }
        Ok(())
    }

    /// `INS-FR-006` (ADR-0046): the whole field list against this
    /// domain's schema, before any write — every one of the three tags
    /// exactly once with a value of its kind, `status` additionally a
    /// known discriminant (the same `status_from_u32` rule
    /// `update_field` applies). `Malformed` for a missing, repeated, or
    /// wrong-kind field; `UnknownField` for a tag this domain doesn't
    /// have.
    fn reminder_from_fields(
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<Reminder, ErrorCode> {
        let mut title = None;
        let mut due_at_unix_ms = None;
        let mut status = None;
        for (tag, value) in fields {
            match (tag, value) {
                (FIELD_TITLE, ScanValue::Str(v)) if title.is_none() => title = Some(v),
                (FIELD_DUE_AT, ScanValue::I64(v)) if due_at_unix_ms.is_none() => {
                    due_at_unix_ms = Some(v)
                }
                (FIELD_STATUS, ScanValue::U32(raw)) if status.is_none() => {
                    status = Some(status_from_u32(raw).ok_or(ErrorCode::Malformed)?)
                }
                (FIELD_TITLE | FIELD_DUE_AT | FIELD_STATUS, _) => return Err(ErrorCode::Malformed),
                _ => return Err(ErrorCode::UnknownField),
            }
        }
        let (Some(title), Some(due_at_unix_ms), Some(status)) = (title, due_at_unix_ms, status)
        else {
            return Err(ErrorCode::Malformed);
        };
        Ok(Reminder {
            id,
            title,
            due_at_unix_ms,
            status,
        })
    }

    /// `ISO-FR-002`/`ISO-FR-006` — see `DogConnectionStore::check_read_set`
    /// for the full contract; identical shape here.
    fn check_read_set(
        reads: &[(RecordId, FieldRef, ScanValue)],
        get: impl Fn(RecordId) -> Option<Reminder>,
    ) -> Result<(), (usize, ErrorCode)> {
        for (id, field, value) in reads {
            let current = get(*id).and_then(|reminder| match *field {
                FIELD_TITLE => Some(ScanValue::Str(reminder.title)),
                FIELD_DUE_AT => Some(ScanValue::I64(reminder.due_at_unix_ms)),
                FIELD_STATUS => Some(ScanValue::U32(status_to_u32(reminder.status))),
                _ => None,
            });
            if current.as_ref() != Some(value) {
                return Err((0, ErrorCode::Conflict));
            }
        }
        Ok(())
    }
}

impl ReminderConnectionStore {
    /// The wire shape of one reminder, in tag order — `get` and a
    /// `ReplaceIf` guard's evaluation both go through here.
    fn fields_of(reminder: Reminder) -> Vec<(FieldRef, ScanValue)> {
        vec![
            (FIELD_TITLE, ScanValue::Str(reminder.title)),
            (FIELD_DUE_AT, ScanValue::I64(reminder.due_at_unix_ms)),
            (FIELD_STATUS, ScanValue::U32(status_to_u32(reminder.status))),
        ]
    }
}

impl ConnectionStore for ReminderConnectionStore {
    fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>> {
        self.store.get::<Reminder>(id).map(Self::fields_of)
    }

    /// `SQL-FR-004`/`SQL-FR-005` (ADR-0034): every id from `all_ids`,
    /// each mapped through this adapter's own `get`.
    fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
        self.store
            .all_ids::<Reminder>()
            .into_iter()
            .filter_map(|id| self.get(id).map(|fields| (id, fields)))
            .collect()
    }

    /// `RDO-FR-002` (ADR-0080): a page ordered by `due_at_unix_ms` is a
    /// range walk of the stack's sorted index — the page's cost, not the
    /// table's (`ORD-FR-005`); any other orderable field takes the scan
    /// path. `validate_page` has already matched the cursor's kind.
    fn page(
        &self,
        order_by: FieldRef,
        after: Option<(ScanValue, RecordId)>,
        limit: usize,
    ) -> Result<Vec<PageRow>, ErrorCode> {
        if order_by != FIELD_DUE_AT {
            return Ok(page_by_scan(self, order_by, after, limit));
        }
        let cursor = match after {
            None => None,
            Some((ScanValue::I64(stamp), id)) => Some((stamp, id)),
            Some(_) => return Err(ErrorCode::Malformed),
        };
        Ok(self
            .store
            .page_by::<Reminder, DueAtOrder>(cursor, limit)
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
        if order_by != FIELD_DUE_AT {
            return Ok(page_by_scan_desc(self, order_by, before, limit));
        }
        let cursor = match before {
            None => None,
            Some((ScanValue::I64(stamp), id)) => Some((stamp, id)),
            Some(_) => return Err(ErrorCode::Malformed),
        };
        Ok(self
            .store
            .page_by_desc::<Reminder, DueAtOrder>(cursor, limit)
            .into_iter()
            .filter_map(|id| self.get(id).map(|fields| (id, fields)))
            .collect())
    }

    /// `RDO-FR-003` (ADR-0080): a page ordered by `due_at_unix_ms` whose
    /// filter does not plan the `due_at` equality index is the bounded
    /// walk — bounds on `due_at` start and cut it, every other
    /// predicate (`status = …`, the consumer's own) is passed over until
    /// the page fills (`FPW-FR-001`–`003`, `FPM-FR-001`–`003`); every
    /// other shape is the trait default's body.
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
            |start, limit| self.store.page_by::<Reminder, DueAtOrder>(start, limit),
            order_by,
            after,
            limit,
            filter,
        )
    }

    /// `RDO-FR-002` (ADR-0080): the field the stack's sorted index is over.
    fn range_field(&self) -> Option<FieldRef> {
        Some(FIELD_DUE_AT)
    }

    /// `RDO-FR-002` (ADR-0080): a `WHERE` range on `due_at_unix_ms` is a
    /// walk of the stack's sorted index between the two bounds — see
    /// `MemoryConnectionStore::range_ids`; identical here over
    /// `DueAtOrder`.
    fn range_ids(
        &self,
        field: FieldRef,
        lower: Bound<ScanValue>,
        upper: Bound<ScanValue>,
    ) -> Result<Vec<RecordId>, ErrorCode> {
        if field != FIELD_DUE_AT {
            return Err(ErrorCode::Unsupported);
        }
        let (lower, upper) = uuid_pair_bounds(lower, upper)?;
        Ok(self.store.range_by::<Reminder, DueAtOrder>(lower, upper))
    }

    /// `RDO-FR-002` (ADR-0080): the budgeted walk (`QPB-FR-002`) — see
    /// `MemoryConnectionStore::range_ids_limited`; identical here.
    /// `QCW-FR-002` (ADR-0081): how many records a `WHERE` range on
    /// `field_due_at` admits — the sorted index's own count between the
    /// bounds, no record read.
    fn range_count(
        &self,
        field: FieldRef,
        lower: Bound<ScanValue>,
        upper: Bound<ScanValue>,
    ) -> Result<u64, ErrorCode> {
        if field != FIELD_DUE_AT {
            return Err(ErrorCode::Unsupported);
        }
        let (lower, upper) = uuid_pair_bounds(lower, upper)?;
        Ok(self.store.range_count::<Reminder, DueAtOrder>(lower, upper) as u64)
    }

    /// `QKW-FR-002` (ADR-0082): the `field_due_at` keys a `WHERE`
    /// range admits, ascending — the sorted index's own keys between
    /// the bounds, no record read.
    fn range_keys(
        &self,
        field: FieldRef,
        lower: Bound<ScanValue>,
        upper: Bound<ScanValue>,
    ) -> Result<Vec<i64>, ErrorCode> {
        if field != FIELD_DUE_AT {
            return Err(ErrorCode::Unsupported);
        }
        let (lower, upper) = uuid_pair_bounds(lower, upper)?;
        Ok(self.store.range_keys::<Reminder, DueAtOrder>(lower, upper))
    }

    /// `QRF-FR-002` (ADR-0087): the walk folded into [`KeyStats`] —
    /// `range_keys` with nothing materialized.
    fn range_stats(
        &self,
        field: FieldRef,
        lower: Bound<ScanValue>,
        upper: Bound<ScanValue>,
    ) -> Result<KeyStats, ErrorCode> {
        if field != FIELD_DUE_AT {
            return Err(ErrorCode::Unsupported);
        }
        let (lower, upper) = uuid_pair_bounds(lower, upper)?;
        Ok(self.store.range_fold::<Reminder, DueAtOrder, _, _>(
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
        if field != FIELD_DUE_AT {
            return Err(ErrorCode::Unsupported);
        }
        let (lower, upper) = uuid_pair_bounds(lower, upper)?;
        Ok(self
            .store
            .range_by_limited::<Reminder, DueAtOrder>(lower, upper, limit))
    }

    fn filter_eq(&self, field: FieldRef, value: &ScanValue) -> Result<Vec<RecordId>, ErrorCode> {
        match (field, value) {
            (FIELD_DUE_AT, ScanValue::I64(due_at)) => {
                Ok(self.store.filter_eq::<Reminder, DueAtField>(due_at))
            }
            (FIELD_DUE_AT, _) => Err(ErrorCode::Malformed),
            (FIELD_STATUS | FIELD_TITLE, _) => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    fn scan_field(&self, field: FieldRef) -> Result<Vec<ScanValue>, ErrorCode> {
        match field {
            FIELD_STATUS => Ok(self
                .store
                .scan::<Reminder, StatusField>()
                .into_iter()
                .map(ScanValue::U32)
                .collect()),
            FIELD_TITLE | FIELD_DUE_AT => Err(ErrorCode::Unsupported),
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
            (FIELD_STATUS, ScanValue::U32(raw)) => {
                if status_from_u32(raw).is_none() {
                    return Err(ErrorCode::Malformed);
                }
                match self.store.update::<Reminder, StatusField>(id, raw) {
                    Ok(()) => Ok(true),
                    Err(_not_found) => Ok(false),
                }
            }
            (FIELD_STATUS, _) => Err(ErrorCode::Malformed),
            (FIELD_TITLE | FIELD_DUE_AT, _) => Err(ErrorCode::Unsupported),
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
        let reminder = Self::reminder_from_fields(id, fields)?;
        match self.store.insert(reminder) {
            Ok(()) => Ok(InsertOutcome::Inserted),
            Err(InsertError::Duplicate(_)) => Ok(InsertOutcome::Duplicate),
            Err(InsertError::Durability(_)) => Err(ErrorCode::Storage),
        }
    }

    /// `REP-FR-005` (ADR-0049): the same validation as `insert_record`,
    /// then one whole-record write under the store's own lock. An
    /// unknown id is the normal outcome, not an error.
    fn replace_record(
        &self,
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<ReplaceOutcome, ErrorCode> {
        let reminder = Self::reminder_from_fields(id, fields)?;
        match self.store.replace(reminder) {
            Ok(()) => Ok(ReplaceOutcome::Replaced),
            Err(ReplaceError::NotFound(_)) => Ok(ReplaceOutcome::NotFound),
            Err(ReplaceError::Durability(_)) => Err(ErrorCode::Storage),
        }
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
        let reminder = Self::reminder_from_fields(id, fields)?;
        let holds = |stored: &Reminder| predicate_matches(&Self::fields_of(stored.clone()), guard);
        match self.store.replace_if(reminder, holds) {
            Ok(GuardedReplace::Replaced) => Ok(ReplaceIfOutcome::Replaced),
            Ok(GuardedReplace::Refused) => Ok(ReplaceIfOutcome::GuardFailed),
            Err(ReplaceError::NotFound(_)) => Ok(ReplaceIfOutcome::NotFound),
            Err(ReplaceError::Durability(_)) => Err(ErrorCode::Storage),
        }
    }

    /// `DEL-FR-006` (ADR-0051): one whole-record delete under the store's
    /// own lock — the record and, within this table, every edge touching
    /// it. An unknown id is the normal outcome, not an error.
    fn delete_record(&self, id: RecordId) -> Result<DeleteOutcome, ErrorCode> {
        match self.store.delete::<Reminder>(id) {
            Ok(()) => Ok(DeleteOutcome::Deleted),
            Err(DeleteError::NotFound(_)) => Ok(DeleteOutcome::NotFound),
            Err(DeleteError::Durability(_)) => Err(ErrorCode::Storage),
        }
    }

    /// `CMP-FR-006` (ADR-0052): the stack compacted under the store's own
    /// write lock; a file that could not be rewritten is `Storage`.
    fn compact(&self) -> Result<crate::generic::CompactionReport, ErrorCode> {
        self.store.compact().map_err(|_| ErrorCode::Storage)
    }

    /// `RMD-FR-005`: `Reminder` has no relation of either kind.
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
            self.store.get::<Reminder>(id).is_some()
        })
        .map_err(|(_, code)| code)
    }

    fn table_name(&self) -> &str {
        "reminder"
    }

    fn describe(&self) -> DomainSchema {
        DomainSchema {
            fields: vec![
                FieldDescriptor {
                    tag: FIELD_TITLE,
                    name: "title".into(),
                    value_kind: ValueKind::Str,
                    capabilities: FieldCapabilities {
                        filter_eq: false,
                        scan: false,
                        update: false,
                    },
                },
                FieldDescriptor {
                    tag: FIELD_DUE_AT,
                    name: "due_at_unix_ms".into(),
                    value_kind: ValueKind::I64,
                    capabilities: FieldCapabilities {
                        filter_eq: true,
                        scan: false,
                        update: false,
                    },
                },
                FieldDescriptor {
                    tag: FIELD_STATUS,
                    name: "status".into(),
                    value_kind: ValueKind::U32,
                    capabilities: FieldCapabilities {
                        filter_eq: false,
                        scan: true,
                        update: true,
                    },
                },
            ],
            relations: RelationCapabilities {
                parent_children: false,
                neighbors: false,
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
                Self::validate_batch(updates, |id| GetById::<Reminder>::get(inner, id).is_some())?;
                Self::check_read_set(read_set, |id| GetById::<Reminder>::get(inner, id))?;
                Self::apply_batch(inner, updates)
            }),
            Some(journal) => {
                Self::validate_batch(updates, |id| self.store.get::<Reminder>(id).is_some())?;
                journal
                    .commit(updates, |turn| {
                        self.store.with_exclusive(|inner| {
                            Self::check_read_set(read_set, |id| {
                                GetById::<Reminder>::get(inner, id)
                            })?;
                            Self::apply_batch(inner, updates)?;
                            Ok(turn.checkpoint_due && inner.checkpoint_flush().is_ok())
                        })
                    })
                    .map_err(|e| match e {
                        CommitError::Journal(_) => (0, ErrorCode::Journal),
                        CommitError::Apply(e) => e,
                    })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generic::reminder::{create_reminder_production_stack, ReminderStatus};
    use crate::test_support::fresh_temp_dir;
    use uuid::Uuid;

    fn sample_adapter() -> ReminderConnectionStore {
        let dir = fresh_temp_dir("server_reminder_adapter").unwrap();
        let path = dir.join("reminders.mmap");
        let reminders = vec![
            Reminder {
                id: Uuid::from_u128(1),
                title: "Pay rent".into(),
                due_at_unix_ms: 1_000,
                status: ReminderStatus::Pending,
            },
            Reminder {
                id: Uuid::from_u128(2),
                title: "Call dentist".into(),
                due_at_unix_ms: 2_000,
                status: ReminderStatus::Snoozed,
            },
        ];
        let stack = create_reminder_production_stack(reminders, &path).unwrap();
        ReminderConnectionStore::new(GenericProductionStore::new(stack))
    }

    #[test]
    fn get_returns_every_field() {
        let adapter = sample_adapter();
        assert_eq!(
            adapter.get(Uuid::from_u128(1)).unwrap(),
            vec![
                (FIELD_TITLE, ScanValue::Str("Pay rent".into())),
                (FIELD_DUE_AT, ScanValue::I64(1_000)),
                (FIELD_STATUS, ScanValue::U32(0)),
            ]
        );
        assert!(adapter.get(Uuid::from_u128(99)).is_none());
    }

    /// `RDO-FR-002`/`003` (ADR-0080): a page by `due_at_unix_ms` walks
    /// the index (identical to the scan path's answer, from a cursor
    /// too); `range_ids`/`range_ids_limited` walk between bounds and
    /// refuse another field; a filtered page with a `status` reject
    /// walks past it and equals the trait default's body exactly, as
    /// does the `due_at` equality (the bucket, equality-first).
    #[test]
    fn due_at_pages_and_ranges_walk_the_index_and_match_the_default() {
        use crate::server::protocol::CompareOp::{Eq, Ge, Le};
        let adapter = sample_adapter();
        let id = Uuid::from_u128;
        let ids = |rows: Vec<PageRow>| rows.into_iter().map(|(id, _)| id).collect::<Vec<_>>();
        assert_eq!(adapter.range_field(), Some(FIELD_DUE_AT));
        assert_eq!(
            ids(adapter.page(FIELD_DUE_AT, None, 10).unwrap()),
            vec![id(1), id(2)]
        );
        assert_eq!(
            adapter.page(FIELD_DUE_AT, None, 10).unwrap(),
            page_by_scan(&adapter, FIELD_DUE_AT, None, 10)
        );
        assert_eq!(
            ids(adapter
                .page(FIELD_DUE_AT, Some((ScanValue::I64(1_000), id(1))), 10)
                .unwrap()),
            vec![id(2)]
        );
        assert_eq!(
            adapter.page(FIELD_DUE_AT, Some((ScanValue::U32(1), id(1))), 10),
            Err(ErrorCode::Malformed)
        );
        assert_eq!(
            adapter.range_ids(
                FIELD_DUE_AT,
                Bound::Unbounded,
                Bound::Included(ScanValue::I64(1_500))
            ),
            Ok(vec![id(1)])
        );
        assert_eq!(
            adapter.range_ids_limited(FIELD_DUE_AT, Bound::Unbounded, Bound::Unbounded, 1),
            Ok(None)
        );
        assert_eq!(
            adapter.range_ids(FIELD_STATUS, Bound::Unbounded, Bound::Unbounded),
            Err(ErrorCode::Unsupported)
        );
        let p = |field, op, value| Predicate { field, op, value };
        let shapes = [
            vec![
                p(FIELD_DUE_AT, Le, ScanValue::I64(9_000)),
                p(FIELD_STATUS, Eq, ScanValue::U32(2)),
            ],
            vec![p(FIELD_DUE_AT, Ge, ScanValue::I64(1_500))],
            vec![p(FIELD_STATUS, Eq, ScanValue::U32(0))],
            vec![p(FIELD_DUE_AT, Eq, ScanValue::I64(2_000))],
            vec![],
        ];
        for filter in shapes {
            assert_eq!(
                adapter
                    .filtered_page(FIELD_DUE_AT, None, 10, &filter)
                    .unwrap(),
                filtered_page_by_candidates(&adapter, FIELD_DUE_AT, None, 10, &filter),
                "{filter:?}"
            );
        }
        assert_eq!(
            ids(adapter
                .filtered_page(
                    FIELD_DUE_AT,
                    None,
                    10,
                    &[
                        p(FIELD_DUE_AT, Le, ScanValue::I64(9_000)),
                        p(FIELD_STATUS, Eq, ScanValue::U32(2)),
                    ],
                )
                .unwrap()),
            vec![id(2)],
            "walked past 1 (pending) to 2 (snoozed)"
        );
    }

    #[test]
    fn filter_eq_by_due_at_and_unsupported_fields() {
        let adapter = sample_adapter();
        assert_eq!(
            adapter.filter_eq(FIELD_DUE_AT, &ScanValue::I64(2_000)),
            Ok(vec![Uuid::from_u128(2)])
        );
        assert!(adapter
            .filter_eq(FIELD_DUE_AT, &ScanValue::I64(9_999))
            .unwrap()
            .is_empty());
        assert_eq!(
            adapter.filter_eq(FIELD_STATUS, &ScanValue::U32(0)),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            adapter.filter_eq(FIELD_TITLE, &ScanValue::Str("x".into())),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            adapter.filter_eq(99, &ScanValue::I64(0)),
            Err(ErrorCode::UnknownField)
        );
    }

    #[test]
    fn scan_and_update_status_only_with_discriminant_validation() {
        let adapter = sample_adapter();
        let mut statuses = adapter.scan_field(FIELD_STATUS).unwrap();
        statuses.sort_by_key(|v| match v {
            ScanValue::U32(n) => *n,
            _ => 0,
        });
        assert_eq!(
            statuses,
            vec![ScanValue::U32(0), ScanValue::U32(2)],
            "Pending, Snoozed"
        );
        assert_eq!(
            adapter.scan_field(FIELD_DUE_AT),
            Err(ErrorCode::Unsupported)
        );

        assert_eq!(
            adapter.update_field(Uuid::from_u128(1), FIELD_STATUS, ScanValue::U32(1)),
            Ok(true),
            "Done"
        );
        assert_eq!(
            adapter.get(Uuid::from_u128(1)).unwrap()[2],
            (FIELD_STATUS, ScanValue::U32(1))
        );
        assert_eq!(
            adapter.update_field(Uuid::from_u128(1), FIELD_STATUS, ScanValue::U32(9)),
            Err(ErrorCode::Malformed),
            "an unrecognized discriminant is rejected"
        );
        assert_eq!(
            adapter.update_field(Uuid::from_u128(99), FIELD_STATUS, ScanValue::U32(0)),
            Ok(false)
        );
        assert_eq!(
            adapter.update_field(Uuid::from_u128(1), FIELD_DUE_AT, ScanValue::I64(0)),
            Err(ErrorCode::Unsupported)
        );
    }

    #[test]
    fn describe_names_all_three_fields_and_reports_no_relations() {
        let adapter = sample_adapter();
        let schema = adapter.describe();
        assert_eq!(schema.fields.len(), 3);
        let status = schema.fields.iter().find(|f| f.name == "status").unwrap();
        assert!(
            status.capabilities.scan
                && status.capabilities.update
                && !status.capabilities.filter_eq
        );
        let due_at = schema
            .fields
            .iter()
            .find(|f| f.name == "due_at_unix_ms")
            .unwrap();
        assert!(
            due_at.capabilities.filter_eq
                && !due_at.capabilities.scan
                && !due_at.capabilities.update
        );
        let title = schema.fields.iter().find(|f| f.name == "title").unwrap();
        assert!(
            !title.capabilities.filter_eq && !title.capabilities.scan && !title.capabilities.update
        );
        assert!(!schema.relations.parent_children);
        assert!(!schema.relations.neighbors);
    }

    #[test]
    fn every_relation_request_is_unsupported() {
        let adapter = sample_adapter();
        assert_eq!(
            adapter.parent(Uuid::from_u128(1)),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            adapter.children(Uuid::from_u128(1)),
            Err(ErrorCode::Unsupported)
        );
        assert_eq!(
            adapter.neighbors(Uuid::from_u128(1)),
            Err(ErrorCode::Unsupported)
        );
    }
    /// `INS-FR-006` (ADR-0046): the whole field list is validated before
    /// any write — a complete, well-typed list inserts; the id is then a
    /// duplicate; a missing, repeated, wrong-kind, unknown, or bad-
    /// discriminant field is refused with nothing inserted.
    #[test]
    fn insert_record_validates_every_field_then_writes_once() {
        let adapter = sample_adapter();
        let full = |status: u32| {
            vec![
                (FIELD_TITLE, ScanValue::Str("Water plants".into())),
                (FIELD_DUE_AT, ScanValue::I64(3_000)),
                (FIELD_STATUS, ScanValue::U32(status)),
            ]
        };
        let id = Uuid::from_u128(3);
        assert_eq!(
            adapter.insert_record(id, full(0)),
            Ok(InsertOutcome::Inserted)
        );
        assert_eq!(adapter.get(id).unwrap(), full(0));
        assert_eq!(
            adapter.filter_eq(FIELD_DUE_AT, &ScanValue::I64(3_000)),
            Ok(vec![id])
        );
        assert_eq!(
            adapter.insert_record(id, full(1)),
            Ok(InsertOutcome::Duplicate)
        );
        assert_eq!(
            adapter.get(id).unwrap(),
            full(0),
            "the duplicate wrote nothing"
        );

        let refused: Vec<(Vec<(FieldRef, ScanValue)>, ErrorCode)> = vec![
            (full(9), ErrorCode::Malformed),
            (full(0)[..2].to_vec(), ErrorCode::Malformed),
            (
                {
                    let mut f = full(0);
                    f.push((FIELD_STATUS, ScanValue::U32(0)));
                    f
                },
                ErrorCode::Malformed,
            ),
            (
                vec![
                    (FIELD_TITLE, ScanValue::I64(1)),
                    (FIELD_DUE_AT, ScanValue::I64(3_000)),
                    (FIELD_STATUS, ScanValue::U32(0)),
                ],
                ErrorCode::Malformed,
            ),
            (
                {
                    let mut f = full(0);
                    f.push((99, ScanValue::U32(0)));
                    f
                },
                ErrorCode::UnknownField,
            ),
            (vec![], ErrorCode::Malformed),
        ];
        for (fields, code) in refused {
            assert_eq!(
                adapter.insert_record(Uuid::from_u128(4), fields.clone()),
                Err(code),
                "{fields:?}"
            );
            assert!(
                adapter.get(Uuid::from_u128(4)).is_none(),
                "nothing inserted"
            );
        }
    }

    /// `REP-FR-005` (ADR-0049): a replaced reminder is read back whole;
    /// an unknown id is `NotFound`; the domain's own `status` rule holds
    /// at replace exactly as at insert.
    #[test]
    fn replace_record_swaps_the_whole_reminder_and_keeps_the_status_rule() {
        let adapter = sample_adapter();
        let id = Uuid::from_u128(1);
        let mut edited = adapter.get(id).unwrap();
        for (tag, value) in edited.iter_mut() {
            if *tag == FIELD_TITLE {
                *value = ScanValue::Str("retitled".into());
            }
        }
        assert_eq!(
            adapter.replace_record(id, edited.clone()),
            Ok(ReplaceOutcome::Replaced)
        );
        assert_eq!(adapter.get(id).unwrap(), edited);
        assert_eq!(
            adapter.replace_record(Uuid::from_u128(99), edited.clone()),
            Ok(ReplaceOutcome::NotFound)
        );
        let mut bad_status = edited.clone();
        for (tag, value) in bad_status.iter_mut() {
            if *tag == FIELD_STATUS {
                *value = ScanValue::U32(99);
            }
        }
        assert_eq!(
            adapter.replace_record(id, bad_status),
            Err(ErrorCode::Malformed)
        );
        assert_eq!(adapter.get(id).unwrap(), edited, "nothing written");
    }

    /// `DEL-FR-006` (ADR-0051): a deleted reminder is gone from `get` and
    /// the scans; a repeat is `NotFound`.
    #[test]
    fn delete_record_removes_the_reminder_and_refuses_a_repeat() {
        let adapter = sample_adapter();
        let id = Uuid::from_u128(1);
        let before = adapter.scan_all().len();
        assert_eq!(adapter.delete_record(id), Ok(DeleteOutcome::Deleted));
        assert!(adapter.get(id).is_none());
        assert_eq!(adapter.scan_all().len(), before - 1);
        assert_eq!(adapter.delete_record(id), Ok(DeleteOutcome::NotFound));
    }
}
