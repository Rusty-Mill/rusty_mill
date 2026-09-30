//! A table's writes, recorded in its change log — `CHL-FR-002`/`003`,
//! `ADR-0131`. [`ChangeLogged`](crate::server::changelogged::ChangeLogged) wraps any [`ConnectionStore`](crate::server::ConnectionStore), forwards every
//! method to it, and — for the writes — runs the inner write and, if it took
//! effect, appends the effective ops to the [`ChangeLog`](crate::server::changelog::ChangeLog) under the log's
//! lock (`ChangeLog::commit`), so the log's order is the apply order and only
//! committed writes are in it. Wrapping the store, not each adapter, is what
//! keeps every write path (a single-shot request, a transaction or session
//! commit, a batch) through one place; the forwarding methods below are the
//! trait's every other method, so nothing the inner store overrides falls back
//! to a default.
//!
//! What is *not* recorded, stated plainly: [`ConnectionStore::detach_record`]
//! (a cross-table cascade, dropped edges on a table that did not initiate the
//! delete), `compact` (a storage rewrite the standby does for itself), and a
//! write that took no effect (a duplicate insert, a guard that failed).

use super::changelog::{ChangeLog, LogPosition};
use super::journal::JournalStats;
use super::nullable::NullableField;
use super::protocol::{
    DomainSchema, ErrorCode, FieldRef, ParentLookup, Predicate, RecordId, RelationDescriptor,
    ScanValue, TransactionOp, WriteOp, WriteResult,
};
use super::serve::{
    BackupReport, ConnectionStore, DeleteOutcome, InsertOutcome, KeyStats, LinkOutcome, PageRow,
    ReplaceIfOutcome, ReplaceOutcome,
};
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;

/// A [`ConnectionStore`] whose writes are recorded in a [`ChangeLog`].
pub struct ChangeLogged {
    inner: Arc<dyn ConnectionStore>,
    log: Arc<ChangeLog>,
}

impl ChangeLogged {
    /// Record `inner`'s writes in `log`.
    pub fn new(inner: Arc<dyn ConnectionStore>, log: Arc<ChangeLog>) -> Self {
        Self { inner, log }
    }

    /// The change log, for the server's shutdown and metrics.
    pub fn log(&self) -> &Arc<ChangeLog> {
        &self.log
    }
}

/// The op a standby replays for `op` once it produced `result`, or `None` if
/// it took no effect. A `ReplaceIf` that applied is a plain `Replace`: the
/// standby need not re-evaluate a guard the primary already passed.
pub fn effective_op(op: &WriteOp, result: &WriteResult) -> Option<WriteOp> {
    match (op, result) {
        (WriteOp::Insert { .. }, WriteResult::Inserted)
        | (WriteOp::Replace { .. }, WriteResult::Replaced)
        | (WriteOp::Delete { .. }, WriteResult::Deleted)
        | (WriteOp::Link { .. }, WriteResult::Linked)
        | (WriteOp::UpdateField { .. }, WriteResult::Updated) => Some(op.clone()),
        (WriteOp::ReplaceIf { id, fields, .. }, WriteResult::Replaced) => Some(WriteOp::Replace {
            id: *id,
            fields: fields.clone(),
        }),
        _ => None,
    }
}

fn effective_ops(ops: &[WriteOp], results: &[WriteResult]) -> Vec<WriteOp> {
    ops.iter()
        .zip(results)
        .filter_map(|(op, result)| effective_op(op, result))
        .collect()
}

fn update_ops(updates: &[TransactionOp]) -> Vec<WriteOp> {
    updates
        .iter()
        .map(|u| WriteOp::UpdateField {
            id: u.id,
            field: u.field,
            value: u.value.clone(),
        })
        .collect()
}

impl ConnectionStore for ChangeLogged {
    fn update_field(
        &self,
        id: RecordId,
        field: FieldRef,
        value: ScanValue,
    ) -> Result<bool, ErrorCode> {
        self.log
            .commit(|| {
                let result = self.inner.update_field(id, field, value.clone());
                let ops = matches!(result, Ok(true)).then(|| {
                    vec![WriteOp::UpdateField {
                        id,
                        field,
                        value: value.clone(),
                    }]
                });
                (result, ops)
            })
            .and_then(|r| r)
    }

    fn insert_record(
        &self,
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<InsertOutcome, ErrorCode> {
        self.log
            .commit(|| {
                let result = self.inner.insert_record(id, fields.clone());
                let ops = matches!(result, Ok(InsertOutcome::Inserted))
                    .then(|| vec![WriteOp::Insert { id, fields }]);
                (result, ops)
            })
            .and_then(|r| r)
    }

    fn replace_record(
        &self,
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
    ) -> Result<ReplaceOutcome, ErrorCode> {
        self.log
            .commit(|| {
                let result = self.inner.replace_record(id, fields.clone());
                let ops = matches!(result, Ok(ReplaceOutcome::Replaced))
                    .then(|| vec![WriteOp::Replace { id, fields }]);
                (result, ops)
            })
            .and_then(|r| r)
    }

    fn replace_record_if(
        &self,
        id: RecordId,
        fields: Vec<(FieldRef, ScanValue)>,
        guard: &Predicate,
    ) -> Result<ReplaceIfOutcome, ErrorCode> {
        self.log
            .commit(|| {
                let result = self.inner.replace_record_if(id, fields.clone(), guard);
                let ops = matches!(result, Ok(ReplaceIfOutcome::Replaced))
                    .then(|| vec![WriteOp::Replace { id, fields }]);
                (result, ops)
            })
            .and_then(|r| r)
    }

    fn delete_record(&self, id: RecordId) -> Result<DeleteOutcome, ErrorCode> {
        self.log
            .commit(|| {
                let result = self.inner.delete_record(id);
                let ops = matches!(result, Ok(DeleteOutcome::Deleted))
                    .then(|| vec![WriteOp::Delete { id }]);
                (result, ops)
            })
            .and_then(|r| r)
    }

    fn link_records(
        &self,
        left: RecordId,
        right: RecordId,
        relation: &str,
    ) -> Result<LinkOutcome, ErrorCode> {
        self.log
            .commit(|| {
                let result = self.inner.link_records(left, right, relation);
                let ops = matches!(result, Ok(LinkOutcome::Linked)).then(|| {
                    vec![WriteOp::Link {
                        left,
                        right,
                        relation: relation.to_string(),
                    }]
                });
                (result, ops)
            })
            .and_then(|r| r)
    }

    fn apply_transaction(
        &self,
        updates: &[TransactionOp],
        read_set: &[(RecordId, FieldRef, ScanValue)],
    ) -> Result<(), (usize, ErrorCode)> {
        self.log
            .commit(|| {
                let result = self.inner.apply_transaction(updates, read_set);
                let ops = result.is_ok().then(|| update_ops(updates));
                (result, ops)
            })
            .map_err(|code| (0, code))
            .and_then(|r| r)
    }

    fn apply_transaction_mvcc(
        &self,
        updates: &[TransactionOp],
        read_set: &[(RecordId, FieldRef, ScanValue)],
        mvcc_snapshot: u64,
    ) -> Result<(), (usize, ErrorCode)> {
        self.log
            .commit(|| {
                let result = self
                    .inner
                    .apply_transaction_mvcc(updates, read_set, mvcc_snapshot);
                let ops = result.is_ok().then(|| update_ops(updates));
                (result, ops)
            })
            .map_err(|code| (0, code))
            .and_then(|r| r)
    }

    fn apply_write_op(&self, op: &WriteOp) -> WriteResult {
        self.log
            .commit(|| {
                let result = self.inner.apply_write_op(op);
                let ops = effective_op(op, &result).map(|op| vec![op]);
                (result, ops)
            })
            .unwrap_or_else(WriteResult::Failed)
    }

    fn write_batch(
        &self,
        ops: &[WriteOp],
        atomic: bool,
    ) -> Result<Vec<WriteResult>, (usize, ErrorCode)> {
        self.log
            .commit(|| {
                let result = self.inner.write_batch(ops, atomic);
                let effective = result
                    .as_ref()
                    .ok()
                    .map(|results| effective_ops(ops, results));
                (result, effective)
            })
            .map_err(|code| (0, code))
            .and_then(|r| r)
    }

    fn strict_commit_supported(&self) -> bool {
        self.inner.strict_commit_supported()
    }

    fn write_batch_strict(&self, ops: &[WriteOp]) -> Result<Vec<WriteResult>, (usize, ErrorCode)> {
        self.log
            .commit(|| {
                let result = self.inner.write_batch_strict(ops);
                let effective = result
                    .as_ref()
                    .ok()
                    .map(|results| effective_ops(ops, results));
                (result, effective)
            })
            .map_err(|code| (0, code))
            .and_then(|r| r)
    }

    fn changes_since(
        &self,
        epoch: u64,
        after: u64,
        limit: usize,
    ) -> Result<(LogPosition, Vec<Vec<WriteOp>>), ErrorCode> {
        self.log.since(epoch, after, limit)
    }

    #[allow(clippy::type_complexity)]
    fn fetch_snapshot_at(&self) -> Result<(Vec<(String, Vec<u8>)>, Option<(u64, u64)>), ErrorCode> {
        // Under the log's lock, so the files and the position agree: no
        // write is between them.
        self.log.with_position(|position| {
            self.inner
                .fetch_snapshot()
                .map(|files| (files, Some((position.epoch, position.head))))
        })
    }

    fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>> {
        self.inner.get(id)
    }

    fn filter_eq(&self, field: FieldRef, value: &ScanValue) -> Result<Vec<RecordId>, ErrorCode> {
        self.inner.filter_eq(field, value)
    }

    fn scan_field(&self, field: FieldRef) -> Result<Vec<ScanValue>, ErrorCode> {
        self.inner.scan_field(field)
    }

    fn parent(&self, id: RecordId) -> Result<ParentLookup, ErrorCode> {
        self.inner.parent(id)
    }

    fn children(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        self.inner.children(id)
    }

    fn neighbors(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        self.inner.neighbors(id)
    }

    fn neighbors_by_relation(
        &self,
        id: RecordId,
        relation: &str,
    ) -> Result<Vec<RecordId>, ErrorCode> {
        self.inner.neighbors_by_relation(id, relation)
    }

    fn list_relation_kinds(&self) -> Vec<String> {
        self.inner.list_relation_kinds()
    }

    fn describe_relations(&self) -> Vec<RelationDescriptor> {
        self.inner.describe_relations()
    }

    fn table_name(&self) -> &str {
        self.inner.table_name()
    }

    fn page(
        &self,
        order_by: FieldRef,
        after: Option<(ScanValue, RecordId)>,
        limit: usize,
    ) -> Result<Vec<PageRow>, ErrorCode> {
        self.inner.page(order_by, after, limit)
    }

    fn page_keys(&self, order_by: FieldRef) -> Vec<(RecordId, i128)> {
        self.inner.page_keys(order_by)
    }

    fn range_field(&self) -> Option<FieldRef> {
        self.inner.range_field()
    }

    fn range_ids(
        &self,
        _field: FieldRef,
        _lower: Bound<ScanValue>,
        _upper: Bound<ScanValue>,
    ) -> Result<Vec<RecordId>, ErrorCode> {
        self.inner.range_ids(_field, _lower, _upper)
    }

    fn range_ids_limited(
        &self,
        _field: FieldRef,
        _lower: Bound<ScanValue>,
        _upper: Bound<ScanValue>,
        _limit: usize,
    ) -> Result<Option<Vec<RecordId>>, ErrorCode> {
        self.inner.range_ids_limited(_field, _lower, _upper, _limit)
    }

    fn range_count(
        &self,
        _field: FieldRef,
        _lower: Bound<ScanValue>,
        _upper: Bound<ScanValue>,
    ) -> Result<u64, ErrorCode> {
        self.inner.range_count(_field, _lower, _upper)
    }

    fn range_keys(
        &self,
        _field: FieldRef,
        _lower: Bound<ScanValue>,
        _upper: Bound<ScanValue>,
    ) -> Result<Vec<i64>, ErrorCode> {
        self.inner.range_keys(_field, _lower, _upper)
    }

    fn range_stats(
        &self,
        _field: FieldRef,
        _lower: Bound<ScanValue>,
        _upper: Bound<ScanValue>,
    ) -> Result<KeyStats, ErrorCode> {
        self.inner.range_stats(_field, _lower, _upper)
    }

    fn filtered_page(
        &self,
        order_by: FieldRef,
        after: Option<(ScanValue, RecordId)>,
        limit: usize,
        filter: &[Predicate],
    ) -> Result<Vec<PageRow>, ErrorCode> {
        self.inner.filtered_page(order_by, after, limit, filter)
    }

    fn page_desc(
        &self,
        order_by: FieldRef,
        before: Option<(ScanValue, RecordId)>,
        limit: usize,
    ) -> Result<Vec<PageRow>, ErrorCode> {
        self.inner.page_desc(order_by, before, limit)
    }

    fn filtered_page_desc(
        &self,
        order_by: FieldRef,
        before: Option<(ScanValue, RecordId)>,
        limit: usize,
        filter: &[Predicate],
    ) -> Result<Vec<PageRow>, ErrorCode> {
        self.inner
            .filtered_page_desc(order_by, before, limit, filter)
    }

    fn count_edges(&self, _relation: &str) -> Result<u64, ErrorCode> {
        self.inner.count_edges(_relation)
    }

    fn detach_record(&self, _relation: &str, _id: RecordId) -> Result<usize, ErrorCode> {
        self.inner.detach_record(_relation, _id)
    }

    fn compact(&self) -> Result<crate::generic::CompactionReport, ErrorCode> {
        self.inner.compact()
    }

    fn backup(&self, _target_dir: &Path) -> Result<BackupReport, ErrorCode> {
        self.inner.backup(_target_dir)
    }

    fn fetch_snapshot(&self) -> Result<Vec<(String, Vec<u8>)>, ErrorCode> {
        self.inner.fetch_snapshot()
    }

    fn describe(&self) -> DomainSchema {
        self.inner.describe()
    }

    fn validate_op(&self, op: &TransactionOp) -> Result<(), ErrorCode> {
        self.inner.validate_op(op)
    }

    fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
        self.inner.scan_all()
    }

    fn record_count(&self) -> Option<usize> {
        self.inner.record_count()
    }

    fn nullable_fields(&self) -> &[NullableField] {
        self.inner.nullable_fields()
    }

    fn mvcc_supported(&self) -> bool {
        self.inner.mvcc_supported()
    }

    fn mvcc_begin(&self) -> u64 {
        self.inner.mvcc_begin()
    }

    fn mvcc_release(&self, _snapshot_txn: u64) {
        self.inner.mvcc_release(_snapshot_txn)
    }

    fn mvcc_get(
        &self,
        _id: RecordId,
        _snapshot_txn: u64,
    ) -> Result<Option<Vec<(FieldRef, ScanValue)>>, ErrorCode> {
        self.inner.mvcc_get(_id, _snapshot_txn)
    }

    fn mvcc_history_entries(&self) -> Option<u64> {
        self.inner.mvcc_history_entries()
    }

    fn journal_stats(&self) -> Option<JournalStats> {
        self.inner.journal_stats()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CHL-FR-002`: an op is in the log only if it took effect, and a
    /// `ReplaceIf` that applied is a plain `Replace`.
    #[test]
    fn only_effective_ops_are_recorded_and_a_guarded_replace_is_unconditional() {
        let id = RecordId::from_u128(1);
        let fields = vec![(0u16, ScanValue::Str("a".into()))];
        let guard = Predicate {
            field: 0,
            op: super::super::protocol::CompareOp::Eq,
            value: ScanValue::Str("a".into()),
        };
        let insert = WriteOp::Insert {
            id,
            fields: fields.clone(),
        };
        assert_eq!(
            effective_op(&insert, &WriteResult::Inserted),
            Some(insert.clone())
        );
        assert_eq!(effective_op(&insert, &WriteResult::Duplicate), None);
        let guarded = WriteOp::ReplaceIf {
            id,
            fields: fields.clone(),
            guard,
        };
        assert_eq!(
            effective_op(&guarded, &WriteResult::Replaced),
            Some(WriteOp::Replace {
                id,
                fields: fields.clone()
            })
        );
        assert_eq!(effective_op(&guarded, &WriteResult::GuardFailed), None);
        assert_eq!(effective_op(&guarded, &WriteResult::NotFound), None);
        let link = WriteOp::Link {
            left: id,
            right: id,
            relation: "r".into(),
        };
        assert_eq!(effective_op(&link, &WriteResult::AlreadyLinked), None);
        assert_eq!(
            effective_op(&link, &WriteResult::Failed(ErrorCode::Storage)),
            None
        );
        assert_eq!(
            effective_ops(
                &[insert, guarded, link],
                &[
                    WriteResult::Inserted,
                    WriteResult::GuardFailed,
                    WriteResult::Linked
                ]
            )
            .len(),
            2
        );
    }
}
