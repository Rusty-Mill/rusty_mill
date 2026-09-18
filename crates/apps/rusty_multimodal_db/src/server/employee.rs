//! [`ConnectionStore`] adapter wrapping
//! [`crate::generic::production::GenericProductionStore<EmployeeProductionStack>`]
//! for `Employee` — the third validation domain, and the first where
//! `Parent`/`Children` *and* `Neighbors` are all real (no domain-shaped
//! `ErrorCode::Unsupported`): `reports_to` (`ChildOf`, self-referential)
//! and `collaborates_with` (`SymmetricRelation`, self-referential) both
//! target `Employee` itself. See `crate::generic_spike::employee_impl`'s
//! own module doc comment for the real, load-bearing gap this combination
//! found and fixed directly in `crate::generic::{store,production}`
//! (`Reversed` never forwarded `Neighbors`; `GenericProductionStore` had
//! no `neighbors` method) before this adapter could even be written.

use super::journal::{CheckpointFlush, CommitError, CommitGroup, JournalError, JournaledBatch};
use super::mvcc::{self, MvccState};
use super::protocol::{
    DomainSchema, ErrorCode, FieldCapabilities, FieldDescriptor, FieldRef, JoinRelation,
    ParentLookup, RecordId, RelationCapabilities, RelationDescriptor, ScanValue, TransactionOp,
    ValueKind,
};
use super::{default_relation_descriptors, ConnectionStore, LinkOutcome};
use crate::durability::DurabilityError;
use crate::generic::production::GenericProductionStore;
use crate::generic::query::{GetById, UpdateField};
use crate::generic::LinkError;
use crate::generic_spike::employee_impl::{
    CollaboratesWith, Department, DepartmentField, Employee, EmployeeProductionStack, ReportsTo,
    SalaryCents,
};
use std::path::{Path, PathBuf};

pub const FIELD_NAME: FieldRef = 0;
pub const FIELD_DEPARTMENT: FieldRef = 1;
pub const FIELD_SALARY: FieldRef = 2;

/// `Department`'s wire encoding — a fixed discriminant, the same pattern
/// `server::order`'s `status_to_u32`/`status_from_u32` already established
/// for an enum `IndexedField`.
fn department_to_u32(department: Department) -> u32 {
    match department {
        Department::Engineering => 0,
        Department::Sales => 1,
        Department::Support => 2,
    }
}

fn department_from_u32(value: u32) -> Option<Department> {
    match value {
        0 => Some(Department::Engineering),
        1 => Some(Department::Sales),
        2 => Some(Department::Support),
        _ => None,
    }
}

/// `ADR-0072`'s `MVCC2-FR-001`/`010` — see `MemoryConnectionStore`'s own
/// `MvccHandle` for the full contract; identical here, except `Employee`
/// has no insert log or `Compact` at all (no runtime `Insert`/`Replace`/
/// `Delete` exists for this domain — its only mutation path is a session
/// `Commit`/journaled `Request::Transaction` batch of `UpdateField` ops),
/// so `MvccState::flush`'s `insert_log_entries_reflected` is always `0`
/// here and the only reclamation boundary is a journal checkpoint.
struct MvccHandle {
    state: MvccState,
    mmap_path: PathBuf,
}

pub struct EmployeeConnectionStore {
    store: GenericProductionStore<EmployeeProductionStack>,
    /// `JRN-FR-001` (ADR-0025) — see `DogConnectionStore::with_journal`.
    journal: Option<CommitGroup>,
    mvcc: Option<MvccHandle>,
}

impl EmployeeConnectionStore {
    pub fn new(store: GenericProductionStore<EmployeeProductionStack>) -> Self {
        Self {
            store,
            journal: None,
            mvcc: None,
        }
    }

    /// The crash-atomic variant — see `DogConnectionStore::with_journal`
    /// for the contract; identical here.
    pub fn with_journal(
        store: GenericProductionStore<EmployeeProductionStack>,
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

    /// `ADR-0072`'s `MVCC2-FR-001` opt-in hook — see
    /// `MemoryConnectionStore::with_mvcc` for the full contract. Simplified
    /// relative to `Memory`/`Entity`/`Relation`'s own version: `Employee`
    /// has no insert log to fold at open time, so this only reads the
    /// persisted `.mvcc` sidecar (if any) — the "journal remainder gap"
    /// this leaves is the same accepted, pre-existing, shared limitation
    /// named in `docs/design/MVCC-OPEN-HOOK-PROPOSAL.md`'s own "Open
    /// questions" section.
    pub fn with_mvcc(mut self, mmap_path: &Path) -> Result<Self, DurabilityError> {
        let reconstructed = MvccState::open(mmap_path)?;
        self.mvcc = Some(MvccHandle {
            state: reconstructed.state,
            mmap_path: mmap_path.to_path_buf(),
        });
        Ok(self)
    }

    /// See `MemoryConnectionStore::mvcc_record_transaction` — identical:
    /// one txn id per whole nonempty batch.
    fn mvcc_record_transaction(&self, updates: &[TransactionOp]) {
        let Some(mvcc) = &self.mvcc else { return };
        if updates.is_empty() {
            return;
        }
        let txn_id = mvcc.state.counter().next();
        mvcc.state.with_index(|index| {
            for op in updates {
                index.record_write((op.id, op.field), txn_id, Some(op.value.clone()));
            }
        });
    }

    /// See `MemoryConnectionStore::mvcc_flush_now` — identical, except
    /// `insert_log_entries_reflected` is always `0` (no insert log here).
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

    /// Same validate-then-apply shape `server::dog`'s own uses —
    /// `Employee`'s only mutable field over this protocol is
    /// `salary_cents`. Safe under one continuously held lock: see
    /// `docs/design/SERVER-TRANSACTION-DESIGN.md`'s own "no runtime
    /// deletion" invariant.
    fn validate_batch(
        updates: &[TransactionOp],
        exists: impl Fn(RecordId) -> bool,
    ) -> Result<(), (usize, ErrorCode)> {
        for (i, op) in updates.iter().enumerate() {
            match (op.field, &op.value) {
                (FIELD_SALARY, ScanValue::I64(_)) => {
                    if !exists(op.id) {
                        return Err((i, ErrorCode::RecordNotFound));
                    }
                }
                (FIELD_SALARY, _) => return Err((i, ErrorCode::Malformed)),
                (FIELD_NAME | FIELD_DEPARTMENT, _) => return Err((i, ErrorCode::Unsupported)),
                _ => return Err((i, ErrorCode::UnknownField)),
            }
        }
        Ok(())
    }

    fn apply_batch(
        inner: &mut EmployeeProductionStack,
        updates: &[TransactionOp],
    ) -> Result<(), (usize, ErrorCode)> {
        for (i, op) in updates.iter().enumerate() {
            if let ScanValue::I64(salary) = op.value {
                UpdateField::<Employee, SalaryCents>::update(inner, op.id, salary)
                    .map_err(|_| (i, ErrorCode::RecordNotFound))?;
            }
        }
        Ok(())
    }

    /// `ISO-FR-002`/`ISO-FR-006` — see `DogConnectionStore::check_read_set`
    /// for the full contract; identical shape here.
    fn check_read_set(
        reads: &[(RecordId, FieldRef, ScanValue)],
        get: impl Fn(RecordId) -> Option<Employee>,
    ) -> Result<(), (usize, ErrorCode)> {
        for (id, field, value) in reads {
            let current = get(*id).and_then(|employee| match *field {
                FIELD_NAME => Some(ScanValue::Str(employee.name)),
                FIELD_DEPARTMENT => Some(ScanValue::U32(department_to_u32(employee.department))),
                FIELD_SALARY => Some(ScanValue::I64(employee.salary_cents)),
                _ => None,
            });
            if current.as_ref() != Some(value) {
                return Err((0, ErrorCode::Conflict));
            }
        }
        Ok(())
    }
}

impl ConnectionStore for EmployeeConnectionStore {
    fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>> {
        self.store.get::<Employee>(id).map(|employee| {
            vec![
                (FIELD_NAME, ScanValue::Str(employee.name)),
                (
                    FIELD_DEPARTMENT,
                    ScanValue::U32(department_to_u32(employee.department)),
                ),
                (FIELD_SALARY, ScanValue::I64(employee.salary_cents)),
            ]
        })
    }

    /// `SQL-FR-004`/`SQL-FR-005` (ADR-0034): every id from `all_ids`,
    /// each mapped through this adapter's own `get`.
    fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
        self.store
            .all_ids::<Employee>()
            .into_iter()
            .filter_map(|id| self.get(id).map(|fields| (id, fields)))
            .collect()
    }

    fn filter_eq(&self, field: FieldRef, value: &ScanValue) -> Result<Vec<RecordId>, ErrorCode> {
        match (field, value) {
            (FIELD_DEPARTMENT, ScanValue::U32(raw)) => match department_from_u32(*raw) {
                Some(department) => Ok(self
                    .store
                    .filter_eq::<Employee, DepartmentField>(&department)),
                None => Err(ErrorCode::Malformed),
            },
            (FIELD_DEPARTMENT, _) => Err(ErrorCode::Malformed),
            (FIELD_NAME | FIELD_SALARY, _) => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    fn scan_field(&self, field: FieldRef) -> Result<Vec<ScanValue>, ErrorCode> {
        match field {
            FIELD_SALARY => Ok(self
                .store
                .scan::<Employee, SalaryCents>()
                .into_iter()
                .map(ScanValue::I64)
                .collect()),
            FIELD_NAME | FIELD_DEPARTMENT => Err(ErrorCode::Unsupported),
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
            (FIELD_SALARY, ScanValue::I64(salary)) => {
                match self.store.update::<Employee, SalaryCents>(id, salary) {
                    Ok(()) => {
                        self.mvcc_record_field_write(id, FIELD_SALARY, ScanValue::I64(salary));
                        // See `DogConnectionStore::update_field`: never
                        // journaled and no `Compact` boundary, so this path
                        // must flush MVCC history on every write it
                        // records.
                        if !self.mvcc_flush_now(0) {
                            return Err(ErrorCode::Storage);
                        }
                        Ok(true)
                    }
                    Err(_not_found) => Ok(false),
                }
            }
            (FIELD_SALARY, _) => Err(ErrorCode::Malformed),
            (FIELD_NAME | FIELD_DEPARTMENT, _) => Err(ErrorCode::Unsupported),
            _ => Err(ErrorCode::UnknownField),
        }
    }

    fn parent(&self, id: RecordId) -> Result<ParentLookup, ErrorCode> {
        match self.store.parent::<Employee, ReportsTo>(id) {
            Ok(Some(manager_id)) => Ok(ParentLookup::Parent(manager_id)),
            Ok(None) => Ok(ParentLookup::NoParent),
            Err(_not_found) => Ok(ParentLookup::ChildNotFound),
        }
    }

    fn children(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        Ok(self.store.children::<Employee, Employee, ReportsTo>(id))
    }

    fn neighbors(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        Ok(self.store.neighbors::<Employee, CollaboratesWith>(id))
    }

    /// `ENT2-FR-004`: `Employee` has exactly one relation label,
    /// `collaborates_with`.
    fn neighbors_by_relation(
        &self,
        id: RecordId,
        relation: &str,
    ) -> Result<Vec<RecordId>, ErrorCode> {
        if relation == "collaborates_with" {
            Ok(self.store.neighbors::<Employee, CollaboratesWith>(id))
        } else {
            Err(ErrorCode::Malformed)
        }
    }

    fn list_relation_kinds(&self) -> Vec<String> {
        vec!["collaborates_with".to_string()]
    }

    /// `LNK-FR-009` (ADR-0047): the fixed-label case — exactly one
    /// label, through the `Symmetric` layer's own `link`.
    fn link_records(
        &self,
        left: RecordId,
        right: RecordId,
        relation: &str,
    ) -> Result<LinkOutcome, ErrorCode> {
        if relation != "collaborates_with" {
            return Err(ErrorCode::Malformed);
        }
        match self.store.link::<Employee, CollaboratesWith>(left, right) {
            Ok(crate::generic::LinkOutcome::Linked) => Ok(LinkOutcome::Linked),
            Ok(crate::generic::LinkOutcome::AlreadyLinked) => Ok(LinkOutcome::AlreadyLinked),
            Err(LinkError::UnknownRecord(_)) => Err(ErrorCode::RecordNotFound),
            Err(LinkError::SelfLoop(_) | LinkError::InvalidLabel(_)) => Err(ErrorCode::Malformed),
            Err(LinkError::Durability(_)) => Err(ErrorCode::Storage),
        }
    }

    /// `JOIN-FR-002` (ADR-0044): `reports_to` is self-referential — an
    /// employee's manager is an employee — so `parent`/`children` are
    /// joinable within this table. The conservative default omits them
    /// (it cannot know the parent type); this is the one adapter today
    /// that can honestly say otherwise. `Order` keeps the default: its
    /// parent is a `Customer` no store holds (ADR-0045).
    fn describe_relations(&self) -> Vec<RelationDescriptor> {
        let mut relations =
            default_relation_descriptors(&self.describe(), self.list_relation_kinds());
        relations.push(RelationDescriptor {
            name: "parent".to_string(),
            kind: JoinRelation::Parent,
            target_table: None,
        });
        relations.push(RelationDescriptor {
            name: "children".to_string(),
            kind: JoinRelation::Children,
            target_table: None,
        });
        relations
    }

    /// `STV-FR-002`: `validate_batch` on this one operation, with the
    /// same per-call existence read the journaled path uses.
    fn validate_op(&self, op: &TransactionOp) -> Result<(), ErrorCode> {
        Self::validate_batch(std::slice::from_ref(op), |id| {
            self.store.get::<Employee>(id).is_some()
        })
        .map_err(|(_, code)| code)
    }

    fn table_name(&self) -> &str {
        "employee"
    }

    fn describe(&self) -> DomainSchema {
        DomainSchema {
            fields: vec![
                FieldDescriptor {
                    tag: FIELD_NAME,
                    name: "name".into(),
                    value_kind: ValueKind::Str,
                    capabilities: FieldCapabilities {
                        filter_eq: false,
                        scan: false,
                        update: false,
                    },
                },
                FieldDescriptor {
                    tag: FIELD_DEPARTMENT,
                    name: "department".into(),
                    value_kind: ValueKind::U32,
                    capabilities: FieldCapabilities {
                        filter_eq: true,
                        scan: false,
                        update: false,
                    },
                },
                FieldDescriptor {
                    tag: FIELD_SALARY,
                    name: "salary_cents".into(),
                    value_kind: ValueKind::I64,
                    capabilities: FieldCapabilities {
                        filter_eq: false,
                        scan: true,
                        update: true,
                    },
                },
            ],
            // The first domain where both are true — Dog has neighbors
            // only, Order/Customer has parent_children only.
            relations: RelationCapabilities {
                parent_children: true,
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
                Self::validate_batch(updates, |id| GetById::<Employee>::get(inner, id).is_some())?;
                Self::check_read_set(read_set, |id| GetById::<Employee>::get(inner, id))?;
                Self::apply_batch(inner, updates)?;
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
                Self::validate_batch(updates, |id| self.store.get::<Employee>(id).is_some())?;
                journal
                    .commit(updates, |turn| {
                        self.store.with_exclusive(|inner| {
                            Self::check_read_set(read_set, |id| {
                                GetById::<Employee>::get(inner, id)
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

    /// `ADR-0072`'s `MVCC2-FR-002`/`003` — see
    /// `MemoryConnectionStore::apply_transaction_mvcc` for the full
    /// contract; identical shape here.
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
                Self::validate_batch(updates, |id| GetById::<Employee>::get(inner, id).is_some())?;
                Self::check_read_set(read_set, |id| GetById::<Employee>::get(inner, id))?;
                conflict_check()?;
                Self::apply_batch(inner, updates)?;
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
                Self::validate_batch(updates, |id| self.store.get::<Employee>(id).is_some())?;
                journal
                    .commit(updates, |turn| {
                        self.store.with_exclusive(|inner| {
                            Self::check_read_set(read_set, |id| {
                                GetById::<Employee>::get(inner, id)
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

    fn mvcc_supported(&self) -> bool {
        self.mvcc.is_some()
    }

    fn mvcc_begin(&self) -> u64 {
        let Some(mvcc) = &self.mvcc else { return 0 };
        if !mvcc.state.is_active() {
            let ids = self.store.all_ids::<Employee>();
            self.store.with_exclusive(|inner| {
                mvcc.state.activate();
                for id in ids {
                    if let Some(employee) = GetById::<Employee>::get(inner, id) {
                        mvcc.state.with_index(|index| {
                            index.record_write(
                                (id, mvcc::EXISTENCE_FIELD),
                                mvcc::BASELINE_TXN,
                                Some(ScanValue::Bool(true)),
                            );
                            index.record_write(
                                (id, FIELD_NAME),
                                mvcc::BASELINE_TXN,
                                Some(ScanValue::Str(employee.name)),
                            );
                            index.record_write(
                                (id, FIELD_DEPARTMENT),
                                mvcc::BASELINE_TXN,
                                Some(ScanValue::U32(department_to_u32(employee.department))),
                            );
                            index.record_write(
                                (id, FIELD_SALARY),
                                mvcc::BASELINE_TXN,
                                Some(ScanValue::I64(employee.salary_cents)),
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
        let mut fields = Vec::with_capacity(3);
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
    use crate::generic_spike::employee_impl::{
        create_employee_production_stack, open_employee_production_stack_portable,
    };
    use crate::test_support::fresh_temp_dir;
    use uuid::Uuid;

    fn sample_adapter() -> EmployeeConnectionStore {
        let dir = fresh_temp_dir("server_employee_adapter").unwrap();
        let path = dir.join("salary.mmap");
        let employees = vec![
            Employee {
                id: Uuid::from_u128(1),
                name: "Alex".into(),
                department: Department::Engineering,
                salary_cents: 1_200_000,
                manager_id: None,
            },
            Employee {
                id: Uuid::from_u128(2),
                name: "Bel".into(),
                department: Department::Engineering,
                salary_cents: 950_000,
                manager_id: Some(Uuid::from_u128(1)),
            },
            Employee {
                id: Uuid::from_u128(3),
                name: "Cas".into(),
                department: Department::Sales,
                salary_cents: 800_000,
                manager_id: Some(Uuid::from_u128(1)),
            },
        ];
        let edges = vec![(Uuid::from_u128(2), Uuid::from_u128(3))];
        let stack = create_employee_production_stack(employees, &edges, &path).unwrap();
        EmployeeConnectionStore::new(GenericProductionStore::new(stack))
    }

    #[test]
    fn get_returns_every_field() {
        let adapter = sample_adapter();
        assert_eq!(
            adapter.get(Uuid::from_u128(2)).unwrap(),
            vec![
                (FIELD_NAME, ScanValue::Str("Bel".into())),
                (FIELD_DEPARTMENT, ScanValue::U32(0)),
                (FIELD_SALARY, ScanValue::I64(950_000)),
            ]
        );
        assert!(adapter.get(Uuid::from_u128(99)).is_none());
    }

    #[test]
    fn filter_by_department_and_unsupported_fields() {
        let adapter = sample_adapter();
        let mut engineers = adapter
            .filter_eq(FIELD_DEPARTMENT, &ScanValue::U32(0))
            .unwrap();
        engineers.sort();
        assert_eq!(engineers, vec![Uuid::from_u128(1), Uuid::from_u128(2)]);
        assert_eq!(
            adapter.filter_eq(FIELD_SALARY, &ScanValue::I64(0)),
            Err(ErrorCode::Unsupported)
        );
    }

    #[test]
    fn scan_and_update_salary_only() {
        let adapter = sample_adapter();
        assert_eq!(
            adapter.update_field(Uuid::from_u128(2), FIELD_SALARY, ScanValue::I64(1_000_000)),
            Ok(true)
        );
        assert_eq!(
            adapter.get(Uuid::from_u128(2)).unwrap()[2],
            (FIELD_SALARY, ScanValue::I64(1_000_000))
        );
        assert_eq!(
            adapter.update_field(Uuid::from_u128(99), FIELD_SALARY, ScanValue::I64(1)),
            Ok(false)
        );
        assert_eq!(adapter.scan_field(FIELD_NAME), Err(ErrorCode::Unsupported));
    }

    #[test]
    fn parent_children_and_neighbors_are_all_real_for_the_first_time() {
        let adapter = sample_adapter();
        assert_eq!(
            adapter.parent(Uuid::from_u128(2)),
            Ok(ParentLookup::Parent(Uuid::from_u128(1)))
        );
        assert_eq!(
            adapter.parent(Uuid::from_u128(1)),
            Ok(ParentLookup::NoParent)
        );

        let mut reports = adapter.children(Uuid::from_u128(1)).unwrap();
        reports.sort();
        assert_eq!(reports, vec![Uuid::from_u128(2), Uuid::from_u128(3)]);

        assert_eq!(
            adapter.neighbors(Uuid::from_u128(2)),
            Ok(vec![Uuid::from_u128(3)])
        );
    }

    #[test]
    fn describe_reports_both_relation_kinds_as_supported() {
        let adapter = sample_adapter();
        let schema = adapter.describe();
        assert_eq!(schema.fields.len(), 3);
        assert!(schema.relations.parent_children);
        assert!(schema.relations.neighbors);
    }
    /// `LNK-FR-009` (ADR-0047): the fixed-label case — only
    /// `collaborates_with`, through the `Symmetric` layer.
    #[test]
    fn link_records_accepts_only_collaborates_with() {
        let adapter = sample_adapter();
        assert_eq!(
            adapter.link_records(Uuid::from_u128(1), Uuid::from_u128(3), "collaborates_with"),
            Ok(LinkOutcome::Linked)
        );
        assert_eq!(
            adapter.link_records(Uuid::from_u128(3), Uuid::from_u128(1), "collaborates_with"),
            Ok(LinkOutcome::AlreadyLinked)
        );
        assert!(adapter
            .neighbors(Uuid::from_u128(3))
            .unwrap()
            .contains(&Uuid::from_u128(1)));
        assert_eq!(
            adapter.link_records(Uuid::from_u128(1), Uuid::from_u128(2), "reports_to"),
            Err(ErrorCode::Malformed),
            "a fixed-label domain creates nothing"
        );
        assert_eq!(adapter.list_relation_kinds(), vec!["collaborates_with"]);
    }

    /// `ADR-0072`: a table with pre-existing records, built with
    /// [`EmployeeConnectionStore::with_mvcc`] so `SESSION_MVCC_ISOLATION`
    /// is available — but not yet activated (`mvcc_begin` does that).
    fn sample_adapter_with_mvcc() -> EmployeeConnectionStore {
        let dir = fresh_temp_dir("server_employee_mvcc_adapter").unwrap();
        let path = dir.join("salary.mmap");
        let employees = vec![
            Employee {
                id: Uuid::from_u128(1),
                name: "Alex".into(),
                department: Department::Engineering,
                salary_cents: 1_200_000,
                manager_id: None,
            },
            Employee {
                id: Uuid::from_u128(2),
                name: "Bel".into(),
                department: Department::Engineering,
                salary_cents: 950_000,
                manager_id: Some(Uuid::from_u128(1)),
            },
        ];
        let stack = create_employee_production_stack(employees, &[], &path).unwrap();
        EmployeeConnectionStore::new(GenericProductionStore::new(stack))
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
                (FIELD_NAME, ScanValue::Str("Alex".into())),
                (FIELD_DEPARTMENT, ScanValue::U32(0)),
                (FIELD_SALARY, ScanValue::I64(1_200_000)),
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
            adapter.update_field(id, FIELD_SALARY, ScanValue::I64(1_500_000)),
            Ok(true)
        );

        assert_eq!(
            adapter.mvcc_get(id, before).unwrap(),
            Some(vec![
                (FIELD_NAME, ScanValue::Str("Alex".into())),
                (FIELD_DEPARTMENT, ScanValue::U32(0)),
                (FIELD_SALARY, ScanValue::I64(1_200_000)),
            ]),
            "the snapshot still sees the pre-update value"
        );
        let after = adapter.mvcc_begin();
        assert_eq!(
            adapter.mvcc_get(id, after).unwrap(),
            Some(vec![
                (FIELD_NAME, ScanValue::Str("Alex".into())),
                (FIELD_DEPARTMENT, ScanValue::U32(0)),
                (FIELD_SALARY, ScanValue::I64(1_500_000)),
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
            field: FIELD_SALARY,
            value: ScanValue::I64(1_300_000),
        };

        // Uncontested: applies and is visible afterward.
        assert_eq!(
            adapter.apply_transaction_mvcc(std::slice::from_ref(&update), &[], snapshot),
            Ok(())
        );
        assert_eq!(
            adapter.get(id).unwrap()[2],
            (FIELD_SALARY, ScanValue::I64(1_300_000))
        );

        // The same (now stale) snapshot's own second attempt conflicts
        // with the write it just made.
        assert_eq!(
            adapter.apply_transaction_mvcc(&[update], &[], snapshot),
            Err((0, ErrorCode::Conflict))
        );
        adapter.mvcc_release(snapshot);
    }

    fn single_employee_with_mvcc() -> (EmployeeConnectionStore, std::path::PathBuf) {
        let dir = fresh_temp_dir("server_employee_mvcc_flush").unwrap();
        let path = dir.join("salary.mmap");
        let employees = vec![Employee {
            id: Uuid::from_u128(1),
            name: "Alex".into(),
            department: Department::Engineering,
            salary_cents: 1_200_000,
            manager_id: None,
        }];
        let stack = create_employee_production_stack(employees, &[], &path).unwrap();
        let adapter = EmployeeConnectionStore::new(GenericProductionStore::new(stack))
            .with_mvcc(&path)
            .unwrap();
        (adapter, path)
    }

    /// The round-ten fix: `Employee` is never journaled by a single
    /// `update_field`/`apply_transaction` and has no insert log or
    /// `Compact` to piggyback on — the *only* thing that could ever
    /// persist a write to `<mmap_path>.mvcc` here is the write path
    /// itself flushing synchronously. Before this fix, `update_field`
    /// recorded into the in-memory index but never flushed, so a restart
    /// with no intervening write silently lost it from MVCC's own
    /// bookkeeping.
    #[test]
    fn update_field_flushes_mvcc_history_and_a_restart_reconstructs_it() {
        let (adapter, path) = single_employee_with_mvcc();
        let id = Uuid::from_u128(1);

        let before = adapter.mvcc_begin();
        assert_eq!(
            adapter.update_field(id, FIELD_SALARY, ScanValue::I64(1_500_000)),
            Ok(true)
        );
        drop(adapter);

        let stack = open_employee_production_stack_portable(&path).unwrap();
        let reopened = EmployeeConnectionStore::new(GenericProductionStore::new(stack))
            .with_mvcc(&path)
            .unwrap();
        assert!(reopened.mvcc_supported());
        assert_eq!(
            reopened.mvcc_get(id, before).unwrap().unwrap()[2],
            (FIELD_SALARY, ScanValue::I64(1_200_000)),
            "the pre-update snapshot still sees the pre-update value after reopen"
        );
        let after = reopened.mvcc_begin();
        assert_eq!(
            reopened.mvcc_get(id, after).unwrap().unwrap()[2],
            (FIELD_SALARY, ScanValue::I64(1_500_000)),
            "a fresh snapshot after reopen sees the write update_field flushed"
        );
    }

    /// See `update_field_flushes_mvcc_history_and_a_restart_reconstructs_
    /// it`: `apply_transaction`'s own non-journaled path has the identical
    /// gap and fix.
    #[test]
    fn apply_transaction_non_journaled_flushes_mvcc_history_and_a_restart_reconstructs_it() {
        let (adapter, path) = single_employee_with_mvcc();
        let id = Uuid::from_u128(1);

        let before = adapter.mvcc_begin();
        assert_eq!(
            adapter.apply_transaction(
                &[TransactionOp {
                    id,
                    field: FIELD_SALARY,
                    value: ScanValue::I64(1_500_000),
                }],
                &[],
            ),
            Ok(())
        );
        drop(adapter);

        let stack = open_employee_production_stack_portable(&path).unwrap();
        let reopened = EmployeeConnectionStore::new(GenericProductionStore::new(stack))
            .with_mvcc(&path)
            .unwrap();
        assert_eq!(
            reopened.mvcc_get(id, before).unwrap().unwrap()[2],
            (FIELD_SALARY, ScanValue::I64(1_200_000)),
            "the pre-commit snapshot still sees the pre-commit value after reopen"
        );
        let after = reopened.mvcc_begin();
        assert_eq!(
            reopened.mvcc_get(id, after).unwrap().unwrap()[2],
            (FIELD_SALARY, ScanValue::I64(1_500_000)),
            "a fresh snapshot after reopen sees the commit apply_transaction flushed"
        );
    }

    /// See `update_field_flushes_mvcc_history_and_a_restart_reconstructs_
    /// it`: `apply_transaction_mvcc`'s own non-journaled path has the
    /// identical gap and fix.
    #[test]
    fn apply_transaction_mvcc_non_journaled_flushes_mvcc_history_and_a_restart_reconstructs_it() {
        let (adapter, path) = single_employee_with_mvcc();
        let id = Uuid::from_u128(1);

        let before = adapter.mvcc_begin();
        assert_eq!(
            adapter.apply_transaction_mvcc(
                &[TransactionOp {
                    id,
                    field: FIELD_SALARY,
                    value: ScanValue::I64(1_500_000),
                }],
                &[],
                before,
            ),
            Ok(())
        );
        drop(adapter);

        let stack = open_employee_production_stack_portable(&path).unwrap();
        let reopened = EmployeeConnectionStore::new(GenericProductionStore::new(stack))
            .with_mvcc(&path)
            .unwrap();
        assert_eq!(
            reopened.mvcc_get(id, before).unwrap().unwrap()[2],
            (FIELD_SALARY, ScanValue::I64(1_200_000)),
            "the pre-commit snapshot still sees the pre-commit value after reopen"
        );
        let after = reopened.mvcc_begin();
        assert_eq!(
            reopened.mvcc_get(id, after).unwrap().unwrap()[2],
            (FIELD_SALARY, ScanValue::I64(1_500_000)),
            "a fresh snapshot after reopen sees the commit apply_transaction_mvcc flushed"
        );
    }
}
