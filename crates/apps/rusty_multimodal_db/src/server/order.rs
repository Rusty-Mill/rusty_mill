//! [`ConnectionStore`] adapter wrapping
//! [`crate::generic::production::GenericProductionStore<OrderProductionStack>`]
//! for `Order`/`Customer` — the second validation domain (a real directed
//! relation, via `Parent`/`Children`; no symmetric relation, so
//! `Neighbors` is unsupported here — the complementary case to
//! [`super::dog`]). Behind the `research` feature: `order_customer` itself
//! is research-gated reference material (see `crate::generic`'s own module
//! docs), so validating the server against it needs both `server` and
//! `research` enabled together.
//!
//! # `OrderProductionStack` only durably tracks `Status`/`Amount`
//!
//! `Order` has three `ScannableField`s in-memory (`Amount`, `CreatedAt`,
//! `DiscountCents`), but [`OrderProductionStack`] (the *durable*
//! production stack this adapter wraps) only carries `Status` (indexed)
//! and `Amount` (the one mmap-backed field) — `CreatedAt`/`DiscountCents`
//! aren't part of it at all (see `order_customer`'s own module docs on
//! why exactly one field is durable). So `ScanField`/`UpdateField` only
//! ever support `Amount` here; `GetById` still returns every field
//! (reconstructed from the caller-supplied records, the same write-through
//! guarantee every durable variant in this crate provides) since a
//! full-record read doesn't need a field to be independently scannable.
//!
//! # `Parent`/`Children` take differently-typed ids
//!
//! `Parent`'s `id` is an `Order` id (every order has exactly one
//! customer); `Children`'s `id` is a `Customer` id (a customer has zero or
//! more orders) — matching the real, asymmetric shape of a directed
//! relation (`docs/design/GENERIC-SCHEMA-DESIGN.md` §4.3), not a
//! convenience this adapter invents.

use super::journal::{CheckpointFlush, CommitError, CommitGroup, JournalError, JournaledBatch};
use super::mvcc::{self, MvccState};
use super::protocol::{
    DomainSchema, ErrorCode, FieldCapabilities, FieldDescriptor, FieldRef, ParentLookup, RecordId,
    RelationCapabilities, ScanValue, TransactionOp, ValueKind,
};
use super::ConnectionStore;
use crate::durability::DurabilityError;
use crate::generic::order_customer::{
    Amount, BelongsToCustomer, Customer, Order, OrderProductionStack, OrderStatus, Status,
};
use crate::generic::production::GenericProductionStore;
use crate::generic::query::{GetById, UpdateField};
use std::path::{Path, PathBuf};

pub const FIELD_AMOUNT: FieldRef = 0;
pub const FIELD_STATUS: FieldRef = 1;
pub const FIELD_CREATED_AT: FieldRef = 2;
pub const FIELD_DISCOUNT: FieldRef = 3;

/// `OrderStatus`'s wire encoding — a fixed discriminant, not `OrderStatus`
/// itself (the protocol's [`ScanValue`] enum stays domain-agnostic; this
/// mapping is this adapter's own concern, the same way the field tags
/// themselves are).
fn status_to_u32(status: OrderStatus) -> u32 {
    match status {
        OrderStatus::Pending => 0,
        OrderStatus::Shipped => 1,
        OrderStatus::Delivered => 2,
        OrderStatus::Cancelled => 3,
        OrderStatus::Refunded => 4,
    }
}

fn status_from_u32(value: u32) -> Option<OrderStatus> {
    match value {
        0 => Some(OrderStatus::Pending),
        1 => Some(OrderStatus::Shipped),
        2 => Some(OrderStatus::Delivered),
        3 => Some(OrderStatus::Cancelled),
        4 => Some(OrderStatus::Refunded),
        _ => None,
    }
}

/// `ADR-0072`'s `MVCC2-FR-001`/`010` — see `MemoryConnectionStore`'s own
/// `MvccHandle` for the full contract; identical here, except `Order`
/// has no insert log or `Compact` at all (no runtime `Insert`/`Replace`/
/// `Delete` exists for this domain — its only mutation path is a session
/// `Commit`/journaled `Request::Transaction` batch of `UpdateField` ops),
/// so `MvccState::flush`'s `insert_log_entries_reflected` is always `0`
/// here and the only reclamation boundary is a journal checkpoint.
struct MvccHandle {
    state: MvccState,
    mmap_path: PathBuf,
}

pub struct OrderConnectionStore {
    store: GenericProductionStore<OrderProductionStack>,
    /// `JRN-FR-001` (ADR-0025) — see `DogConnectionStore::with_journal`.
    journal: Option<CommitGroup>,
    /// `ADR-0072`'s `MVCC2-FR-001` — see `MemoryConnectionStore`'s own
    /// `mvcc` field for the full contract; identical here.
    mvcc: Option<MvccHandle>,
}

impl OrderConnectionStore {
    pub fn new(store: GenericProductionStore<OrderProductionStack>) -> Self {
        Self {
            store,
            journal: None,
            mvcc: None,
        }
    }

    /// `ADR-0072`'s `MVCC2-FR-001`/`003`: enable real MVCC for this
    /// table, reconstructing `<mmap_path>.mvcc` (if any). Unlike
    /// `MemoryConnectionStore::with_mvcc`, there is no insert log to
    /// fold here at all — `Order` has no runtime `Insert`/`Replace`/
    /// `Delete`, so every write is a session `Commit`/journaled
    /// `UpdateField` batch, and the only durable record of MVCC history
    /// beyond `<mmap_path>.mvcc` itself is the journal, replayed the
    /// same way `with_journal` already replays it into the store (see
    /// that constructor's own doc comment for the accepted, pre-existing
    /// gap this shares with `ADR-0033`'s own rejected-commit window: a
    /// journal replayed on open is not folded into MVCC either, the
    /// identical shape `MVCC-OPEN-HOOK-PROPOSAL.md`'s own "Open
    /// questions" section named for `Memory`/`Entity`/`Relation` and
    /// left for a follow-up decision).
    ///
    /// # Errors
    ///
    /// Returns [`DurabilityError`] if `<mmap_path>.mvcc` exists but
    /// can't be read/decoded.
    pub fn with_mvcc(mut self, mmap_path: &Path) -> Result<Self, DurabilityError> {
        let reconstructed = MvccState::open(mmap_path)?;
        self.mvcc = Some(MvccHandle {
            state: reconstructed.state,
            mmap_path: mmap_path.to_path_buf(),
        });
        Ok(self)
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

    /// `ADR-0072`'s `MVCC2-FR-010` — see `MemoryConnectionStore::
    /// mvcc_flush_now` for the full contract; simplified here since
    /// there is no insert log at all (`insert_log_entries_reflected` is
    /// always `0`).
    fn mvcc_flush_now(&self, journal_entries: u64) -> bool {
        let Some(mvcc) = &self.mvcc else { return true };
        if !mvcc.state.is_active() {
            return true;
        }
        mvcc.state
            .flush(&mvcc.mmap_path, 0, journal_entries as usize)
            .is_ok()
    }

    /// `ADR-0072`'s `MVCC2-FR-008`: record a single ordinary (non-batch)
    /// field write — the `update_field` path's own equivalent of
    /// `MemoryConnectionStore::mvcc_record_write`, sized to one field
    /// since that is all `update_field` ever changes.
    fn mvcc_record_field_write(&self, id: RecordId, field: FieldRef, value: ScanValue) {
        let Some(mvcc) = &self.mvcc else { return };
        if !mvcc.state.is_active() {
            return;
        }
        let txn_id = mvcc.state.counter().next();
        mvcc.state
            .with_index(|index| index.record_write((id, field), txn_id, Some(value)));
    }

    /// The crash-atomic variant — see `DogConnectionStore::with_journal`
    /// for the contract; identical here.
    pub fn with_journal(
        store: GenericProductionStore<OrderProductionStack>,
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
            mvcc: None,
        })
    }

    /// Same validate-then-apply shape `server::dog`'s own uses —
    /// `Order`'s only mutable field over this protocol is `amount_cents`.
    /// Safe under one continuously held lock: see
    /// `docs/design/SERVER-TRANSACTION-DESIGN.md`'s own "no runtime
    /// deletion" invariant.
    fn validate_batch(
        updates: &[TransactionOp],
        exists: impl Fn(RecordId) -> bool,
    ) -> Result<(), (usize, ErrorCode)> {
        for (i, op) in updates.iter().enumerate() {
            match (op.field, &op.value) {
                (FIELD_AMOUNT, ScanValue::I64(_)) => {
                    if !exists(op.id) {
                        return Err((i, ErrorCode::RecordNotFound));
                    }
                }
                (FIELD_AMOUNT, _) => return Err((i, ErrorCode::Malformed)),
                (FIELD_STATUS | FIELD_CREATED_AT | FIELD_DISCOUNT, _) => {
                    return Err((i, ErrorCode::Unsupported))
                }
                _ => return Err((i, ErrorCode::UnknownField)),
            }
        }
        Ok(())
    }

    fn apply_batch(
        inner: &mut OrderProductionStack,
        updates: &[TransactionOp],
    ) -> Result<(), (usize, ErrorCode)> {
        for (i, op) in updates.iter().enumerate() {
            if let ScanValue::I64(amount) = op.value {
                UpdateField::<Order, Amount>::update(inner, op.id, amount)
                    .map_err(|_| (i, ErrorCode::RecordNotFound))?;
            }
        }
        Ok(())
    }

    /// `ISO-FR-002`/`ISO-FR-006` — see `DogConnectionStore::check_read_set`
    /// for the full contract; identical shape here.
    fn check_read_set(
        reads: &[(RecordId, FieldRef, ScanValue)],
        get: impl Fn(RecordId) -> Option<Order>,
    ) -> Result<(), (usize, ErrorCode)> {
        for (id, field, value) in reads {
            let current = get(*id).and_then(|order| match *field {
                FIELD_AMOUNT => Some(ScanValue::I64(order.amount_cents)),
                FIELD_STATUS => Some(ScanValue::U32(status_to_u32(order.status))),
                FIELD_CREATED_AT => Some(ScanValue::I64(order.created_at_unix_ms)),
                FIELD_DISCOUNT => Some(ScanValue::I64(order.discount_cents)),
                _ => None,
            });
            if current.as_ref() != Some(value) {
                return Err((0, ErrorCode::Conflict));
            }
        }
        Ok(())
    }
}

impl ConnectionStore for OrderConnectionStore {
    fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>> {
        self.store.get::<Order>(id).map(|order| {
            vec![
                (FIELD_AMOUNT, ScanValue::I64(order.amount_cents)),
                (FIELD_STATUS, ScanValue::U32(status_to_u32(order.status))),
                (FIELD_CREATED_AT, ScanValue::I64(order.created_at_unix_ms)),
                (FIELD_DISCOUNT, ScanValue::I64(order.discount_cents)),
            ]
        })
    }

    /// `SQL-FR-004`/`SQL-FR-005` (ADR-0034): every id from `all_ids`,
    /// each mapped through this adapter's own `get`.
    fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
        self.store
            .all_ids::<Order>()
            .into_iter()
            .filter_map(|id| self.get(id).map(|fields| (id, fields)))
            .collect()
    }

    fn filter_eq(&self, field: FieldRef, value: &ScanValue) -> Result<Vec<RecordId>, ErrorCode> {
        match (field, value) {
            (FIELD_STATUS, ScanValue::U32(raw)) => match status_from_u32(*raw) {
                Some(status) => Ok(self.store.filter_eq::<Order, Status>(&status)),
                None => Err(ErrorCode::Malformed),
            },
            (FIELD_STATUS, _) => Err(ErrorCode::Malformed),
            (FIELD_AMOUNT | FIELD_CREATED_AT | FIELD_DISCOUNT, _) => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    fn scan_field(&self, field: FieldRef) -> Result<Vec<ScanValue>, ErrorCode> {
        match field {
            FIELD_AMOUNT => Ok(self
                .store
                .scan::<Order, Amount>()
                .into_iter()
                .map(ScanValue::I64)
                .collect()),
            FIELD_STATUS | FIELD_CREATED_AT | FIELD_DISCOUNT => Err(ErrorCode::Unsupported),
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
            (FIELD_AMOUNT, ScanValue::I64(amount)) => {
                match self.store.update::<Order, Amount>(id, amount) {
                    Ok(()) => {
                        self.mvcc_record_field_write(id, FIELD_AMOUNT, ScanValue::I64(amount));
                        Ok(true)
                    }
                    Err(_not_found) => Ok(false),
                }
            }
            (FIELD_AMOUNT, _) => Err(ErrorCode::Malformed),
            (FIELD_STATUS | FIELD_CREATED_AT | FIELD_DISCOUNT, _) => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    fn parent(&self, id: RecordId) -> Result<ParentLookup, ErrorCode> {
        match self.store.parent::<Order, BelongsToCustomer>(id) {
            Ok(Some(customer_id)) => Ok(ParentLookup::Parent(customer_id)),
            Ok(None) => Ok(ParentLookup::NoParent),
            Err(_not_found) => Ok(ParentLookup::ChildNotFound),
        }
    }

    fn children(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        Ok(self
            .store
            .children::<Customer, Order, BelongsToCustomer>(id))
    }

    fn neighbors(&self, _id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        // Order/Customer has no SymmetricRelation.
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
            self.store.get::<Order>(id).is_some()
        })
        .map_err(|(_, code)| code)
    }

    fn table_name(&self) -> &str {
        "order"
    }

    fn describe(&self) -> DomainSchema {
        let read_only = |value_kind: ValueKind, name: &str, tag: FieldRef| FieldDescriptor {
            tag,
            name: name.into(),
            value_kind,
            capabilities: FieldCapabilities {
                filter_eq: false,
                scan: false,
                update: false,
            },
        };
        DomainSchema {
            fields: vec![
                FieldDescriptor {
                    tag: FIELD_AMOUNT,
                    name: "amount_cents".into(),
                    value_kind: ValueKind::I64,
                    capabilities: FieldCapabilities {
                        filter_eq: false,
                        scan: true,
                        update: true,
                    },
                },
                FieldDescriptor {
                    tag: FIELD_STATUS,
                    name: "status".into(),
                    value_kind: ValueKind::U32,
                    capabilities: FieldCapabilities {
                        filter_eq: true,
                        scan: false,
                        update: false,
                    },
                },
                read_only(ValueKind::I64, "created_at_unix_ms", FIELD_CREATED_AT),
                read_only(ValueKind::I64, "discount_cents", FIELD_DISCOUNT),
            ],
            relations: RelationCapabilities {
                parent_children: true,
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
                Self::validate_batch(updates, |id| GetById::<Order>::get(inner, id).is_some())?;
                Self::check_read_set(read_set, |id| GetById::<Order>::get(inner, id))?;
                Self::apply_batch(inner, updates)?;
                self.mvcc_record_transaction(updates);
                Ok(())
            }),
            Some(journal) => {
                Self::validate_batch(updates, |id| self.store.get::<Order>(id).is_some())?;
                journal
                    .commit(updates, |turn| {
                        self.store.with_exclusive(|inner| {
                            Self::check_read_set(read_set, |id| GetById::<Order>::get(inner, id))?;
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
                Self::validate_batch(updates, |id| GetById::<Order>::get(inner, id).is_some())?;
                Self::check_read_set(read_set, |id| GetById::<Order>::get(inner, id))?;
                conflict_check()?;
                Self::apply_batch(inner, updates)?;
                self.mvcc_record_transaction(updates);
                Ok(())
            }),
            Some(journal) => {
                Self::validate_batch(updates, |id| self.store.get::<Order>(id).is_some())?;
                journal
                    .commit(updates, |turn| {
                        self.store.with_exclusive(|inner| {
                            Self::check_read_set(read_set, |id| GetById::<Order>::get(inner, id))?;
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

    /// `ADR-0072`'s `MVCC2-FR-012`: `Order` implements real MVCC.
    fn mvcc_supported(&self) -> bool {
        self.mvcc.is_some()
    }

    /// `ADR-0072`'s `MVCC2-FR-001`/`005`: activate (baseline-seed every
    /// currently-live record, once — `Order`'s own "no runtime deletion"
    /// invariant means `all_ids` never changes at runtime, so reading it
    /// once before the exclusive section is safe) and open a new
    /// snapshot.
    fn mvcc_begin(&self) -> u64 {
        let Some(mvcc) = &self.mvcc else { return 0 };
        if !mvcc.state.is_active() {
            let ids = self.store.all_ids::<Order>();
            self.store.with_exclusive(|inner| {
                mvcc.state.activate();
                for id in ids {
                    if let Some(order) = GetById::<Order>::get(inner, id) {
                        mvcc.state.with_index(|index| {
                            index.record_write(
                                (id, mvcc::EXISTENCE_FIELD),
                                mvcc::BASELINE_TXN,
                                Some(ScanValue::Bool(true)),
                            );
                            index.record_write(
                                (id, FIELD_AMOUNT),
                                mvcc::BASELINE_TXN,
                                Some(ScanValue::I64(order.amount_cents)),
                            );
                            index.record_write(
                                (id, FIELD_STATUS),
                                mvcc::BASELINE_TXN,
                                Some(ScanValue::U32(status_to_u32(order.status))),
                            );
                            index.record_write(
                                (id, FIELD_CREATED_AT),
                                mvcc::BASELINE_TXN,
                                Some(ScanValue::I64(order.created_at_unix_ms)),
                            );
                            index.record_write(
                                (id, FIELD_DISCOUNT),
                                mvcc::BASELINE_TXN,
                                Some(ScanValue::I64(order.discount_cents)),
                            );
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
        let mut fields = Vec::with_capacity(self.describe().fields.len());
        for field in self.describe().fields {
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
    use crate::generic::order_customer::{create_order_production_stack, OrderStatus};
    use crate::test_support::fresh_temp_dir;
    use uuid::Uuid;

    fn sample_adapter() -> OrderConnectionStore {
        let dir = fresh_temp_dir("server_order_adapter").unwrap();
        let path = dir.join("amount.mmap");
        let orders = vec![
            crate::generic::order_customer::Order {
                id: Uuid::from_u128(1),
                customer_id: Uuid::from_u128(100),
                amount_cents: 2_500,
                status: OrderStatus::Shipped,
                created_at_unix_ms: 1_000,
                discount_cents: 0,
            },
            crate::generic::order_customer::Order {
                id: Uuid::from_u128(2),
                customer_id: Uuid::from_u128(100),
                amount_cents: 4_200,
                status: OrderStatus::Pending,
                created_at_unix_ms: 2_000,
                discount_cents: 0,
            },
        ];
        let stack = create_order_production_stack(orders, &path).unwrap();
        OrderConnectionStore::new(GenericProductionStore::new(stack))
    }

    #[test]
    fn get_returns_every_field() {
        let adapter = sample_adapter();
        assert_eq!(
            adapter.get(Uuid::from_u128(1)).unwrap(),
            vec![
                (FIELD_AMOUNT, ScanValue::I64(2_500)),
                (FIELD_STATUS, ScanValue::U32(1)),
                (FIELD_CREATED_AT, ScanValue::I64(1_000)),
                (FIELD_DISCOUNT, ScanValue::I64(0)),
            ]
        );
        assert!(adapter.get(Uuid::from_u128(99)).is_none());
    }

    #[test]
    fn filter_eq_by_status_and_unsupported_fields() {
        let adapter = sample_adapter();
        assert_eq!(
            adapter.filter_eq(FIELD_STATUS, &ScanValue::U32(1)),
            Ok(vec![Uuid::from_u128(1)])
        );
        assert_eq!(
            adapter.filter_eq(FIELD_AMOUNT, &ScanValue::I64(0)),
            Err(ErrorCode::Unsupported)
        );
    }

    #[test]
    fn scan_and_update_amount_only() {
        let adapter = sample_adapter();
        let mut amounts = adapter.scan_field(FIELD_AMOUNT).unwrap();
        amounts.sort_by_key(|v| match v {
            ScanValue::I64(n) => *n,
            _ => 0,
        });
        assert_eq!(amounts, vec![ScanValue::I64(2_500), ScanValue::I64(4_200)]);
        assert_eq!(
            adapter.scan_field(FIELD_DISCOUNT),
            Err(ErrorCode::Unsupported)
        );

        assert_eq!(
            adapter.update_field(Uuid::from_u128(1), FIELD_AMOUNT, ScanValue::I64(9_000)),
            Ok(true)
        );
        assert_eq!(
            adapter.get(Uuid::from_u128(1)).unwrap()[0],
            (FIELD_AMOUNT, ScanValue::I64(9_000))
        );
        assert_eq!(
            adapter.update_field(Uuid::from_u128(99), FIELD_AMOUNT, ScanValue::I64(1)),
            Ok(false)
        );
    }

    #[test]
    fn parent_and_children_reflect_belongs_to_customer() {
        let adapter = sample_adapter();
        assert_eq!(
            adapter.parent(Uuid::from_u128(1)),
            Ok(ParentLookup::Parent(Uuid::from_u128(100)))
        );
        assert_eq!(
            adapter.parent(Uuid::from_u128(99)),
            Ok(ParentLookup::ChildNotFound)
        );

        let mut children = adapter.children(Uuid::from_u128(100)).unwrap();
        children.sort();
        assert_eq!(children, vec![Uuid::from_u128(1), Uuid::from_u128(2)]);
    }

    #[test]
    fn describe_names_all_four_fields_and_reports_parent_children_only() {
        let adapter = sample_adapter();
        let schema = adapter.describe();
        assert_eq!(schema.fields.len(), 4);
        let amount = schema
            .fields
            .iter()
            .find(|f| f.name == "amount_cents")
            .unwrap();
        assert!(
            amount.capabilities.scan
                && amount.capabilities.update
                && !amount.capabilities.filter_eq
        );
        let status = schema.fields.iter().find(|f| f.name == "status").unwrap();
        assert!(
            status.capabilities.filter_eq
                && !status.capabilities.scan
                && !status.capabilities.update
        );
        for read_only_name in ["created_at_unix_ms", "discount_cents"] {
            let field = schema
                .fields
                .iter()
                .find(|f| f.name == read_only_name)
                .unwrap();
            assert!(
                !field.capabilities.filter_eq
                    && !field.capabilities.scan
                    && !field.capabilities.update
            );
        }
        assert!(schema.relations.parent_children);
        assert!(!schema.relations.neighbors);
    }

    #[test]
    fn neighbors_is_unsupported() {
        let adapter = sample_adapter();
        assert_eq!(
            adapter.neighbors(Uuid::from_u128(1)),
            Err(ErrorCode::Unsupported)
        );
    }

    /// `ADR-0072`: a table with two pre-existing records, built with
    /// [`OrderConnectionStore::with_mvcc`] so `SESSION_MVCC_ISOLATION` is
    /// available — but not yet activated (`mvcc_begin` does that).
    fn sample_adapter_with_mvcc() -> OrderConnectionStore {
        let dir = fresh_temp_dir("server_order_mvcc_adapter").unwrap();
        let path = dir.join("amount.mmap");
        let orders = vec![
            crate::generic::order_customer::Order {
                id: Uuid::from_u128(1),
                customer_id: Uuid::from_u128(100),
                amount_cents: 2_500,
                status: OrderStatus::Shipped,
                created_at_unix_ms: 1_000,
                discount_cents: 0,
            },
            crate::generic::order_customer::Order {
                id: Uuid::from_u128(2),
                customer_id: Uuid::from_u128(100),
                amount_cents: 4_200,
                status: OrderStatus::Pending,
                created_at_unix_ms: 2_000,
                discount_cents: 0,
            },
        ];
        let stack = create_order_production_stack(orders, &path).unwrap();
        OrderConnectionStore::new(GenericProductionStore::new(stack))
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
            Some(vec![
                (FIELD_AMOUNT, ScanValue::I64(2_500)),
                (FIELD_STATUS, ScanValue::U32(1)),
                (FIELD_CREATED_AT, ScanValue::I64(1_000)),
                (FIELD_DISCOUNT, ScanValue::I64(0)),
            ])
        );
        adapter.mvcc_release(snapshot);
    }

    /// `MVCC2-FR-006`/`008`, acceptance criterion 4: a real-MVCC snapshot
    /// survives a *concurrent ordinary* (non-session) write — not only a
    /// conflicting session commit — and a fresh snapshot taken afterward
    /// sees the new value.
    #[test]
    fn mvcc_snapshot_survives_a_concurrent_ordinary_update() {
        let adapter = sample_adapter_with_mvcc();
        let id = Uuid::from_u128(1);
        let before = adapter.mvcc_begin();

        assert_eq!(
            adapter.update_field(id, FIELD_AMOUNT, ScanValue::I64(9_000)),
            Ok(true)
        );

        assert_eq!(
            adapter.mvcc_get(id, before).unwrap(),
            Some(vec![
                (FIELD_AMOUNT, ScanValue::I64(2_500)),
                (FIELD_STATUS, ScanValue::U32(1)),
                (FIELD_CREATED_AT, ScanValue::I64(1_000)),
                (FIELD_DISCOUNT, ScanValue::I64(0)),
            ]),
            "the snapshot still sees the pre-update value"
        );
        let after = adapter.mvcc_begin();
        assert_eq!(
            adapter.mvcc_get(id, after).unwrap(),
            Some(vec![
                (FIELD_AMOUNT, ScanValue::I64(9_000)),
                (FIELD_STATUS, ScanValue::U32(1)),
                (FIELD_CREATED_AT, ScanValue::I64(1_000)),
                (FIELD_DISCOUNT, ScanValue::I64(0)),
            ]),
            "a fresh snapshot sees the ordinary write"
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
            field: FIELD_AMOUNT,
            value: ScanValue::I64(5_000),
        };

        // Uncontested: applies and is visible afterward.
        assert_eq!(
            adapter.apply_transaction_mvcc(std::slice::from_ref(&update), &[], snapshot),
            Ok(())
        );
        assert_eq!(
            adapter.get(id).unwrap()[0],
            (FIELD_AMOUNT, ScanValue::I64(5_000))
        );

        // The same (now stale) snapshot's own second attempt conflicts
        // with the write it just made.
        assert_eq!(
            adapter.apply_transaction_mvcc(&[update], &[], snapshot),
            Err((0, ErrorCode::Conflict))
        );
        adapter.mvcc_release(snapshot);
    }
}
