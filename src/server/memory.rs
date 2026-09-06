//! [`ConnectionStore`] adapter wrapping
//! [`crate::generic::production::GenericProductionStore<MemoryProductionStack>`]
//! for `Memory` — this crate's sixth domain and third front-door one
//! (`MEM-FR-006`, ADR-0048, `docs/design/SERVER-MEMORY-DOMAIN-DESIGN.md`),
//! `server`-gated alone like `reminder`/`entity`. Eleven fields:
//! `category` is equality-filterable, `access_count` scannable and
//! updatable (non-negative — the one domain rule), everything else
//! read-only over the wire after insert and reachable through `Query`/
//! `Aggregate` like any field. `tags` is a `StrList`. No relation of
//! either kind; every relation request is `Unsupported`.
//!
//! See `crate::generic::memory`'s own module docs for what the
//! consumer's table holds that this record does not, and why.

use super::journal::{CheckpointFlush, CommitError, CommitGroup, JournalError};
use super::protocol::{
    DomainSchema, ErrorCode, FieldCapabilities, FieldDescriptor, FieldRef, ParentLookup, RecordId,
    RelationCapabilities, ScanValue, TransactionOp, ValueKind,
};
use super::{ConnectionStore, InsertOutcome};
use crate::generic::memory::{AccessCountField, CategoryField, Memory, MemoryProductionStack};
use crate::generic::production::GenericProductionStore;
use crate::generic::query::{GetById, UpdateField};
use crate::generic::InsertError;
use std::path::Path;

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

/// Every field but `access_count`: read-only over the wire after insert
/// (`MEM-FR-004`) — the whole-record replacement the consumer's
/// `update_memory` needs is the next round, named in the module docs.
const READ_ONLY_FIELDS: [FieldRef; 10] = [
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
];

/// `MEM-FR-003`: `access_count` is a counter — a negative value is
/// `Malformed` before any write, the one domain rule this adapter adds.
fn valid_access_count(value: i64) -> bool {
    value >= 0
}

pub struct MemoryConnectionStore {
    store: GenericProductionStore<MemoryProductionStack>,
    /// `JRN-FR-001` (ADR-0025) — see `DogConnectionStore::with_journal`.
    journal: Option<CommitGroup>,
}

impl MemoryConnectionStore {
    pub fn new(store: GenericProductionStore<MemoryProductionStack>) -> Self {
        Self {
            store,
            journal: None,
        }
    }

    /// The crash-atomic variant — see `DogConnectionStore::with_journal`
    /// for the contract; identical here.
    pub fn with_journal(
        store: GenericProductionStore<MemoryProductionStack>,
        journal_path: &Path,
    ) -> Result<Self, JournalError> {
        let (journal, batches) = CommitGroup::open(journal_path)?;
        store.with_exclusive(|inner| -> Result<(), JournalError> {
            for (batch_index, batch) in batches.iter().enumerate() {
                Self::apply_batch(inner, batch).map_err(|(index, code)| JournalError::Replay {
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
    /// domain's schema, before any write — all eleven tags exactly once
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
                (tag, _) if tag <= FIELD_ACCESS_COUNT => return Err(ErrorCode::Malformed),
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
        ]
    }
}

impl ConnectionStore for MemoryConnectionStore {
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

    fn filter_eq(&self, field: FieldRef, value: &ScanValue) -> Result<Vec<RecordId>, ErrorCode> {
        match (field, value) {
            (FIELD_CATEGORY, ScanValue::Str(category)) => {
                Ok(self.store.filter_eq::<Memory, CategoryField>(category))
            }
            (FIELD_CATEGORY, _) => Err(ErrorCode::Malformed),
            (field, _) if field <= FIELD_ACCESS_COUNT => Err(ErrorCode::Unsupported),
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
                match self.store.update::<Memory, AccessCountField>(id, count) {
                    Ok(()) => Ok(true),
                    Err(_not_found) => Ok(false),
                }
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
        match self.store.insert(memory) {
            Ok(()) => Ok(InsertOutcome::Inserted),
            Err(InsertError::Duplicate(_)) => Ok(InsertOutcome::Duplicate),
            Err(InsertError::Durability(_)) => Err(ErrorCode::Storage),
        }
    }

    /// `MEM-FR-005`: `Memory` has no relation of either kind — yet.
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

    /// `STV-FR-002`: `validate_batch` on this one operation.
    fn validate_op(&self, op: &TransactionOp) -> Result<(), ErrorCode> {
        Self::validate_batch(std::slice::from_ref(op), |id| {
            self.store.get::<Memory>(id).is_some()
        })
        .map_err(|(_, code)| code)
    }

    fn describe(&self) -> DomainSchema {
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
        // (`GRP-FR-001`–`005`) and where the read-set check runs in each.
        match &self.journal {
            None => self.store.with_exclusive(|inner| {
                Self::validate_batch(updates, |id| GetById::<Memory>::get(inner, id).is_some())?;
                Self::check_read_set(read_set, |id| GetById::<Memory>::get(inner, id))?;
                Self::apply_batch(inner, updates)
            }),
            Some(journal) => {
                Self::validate_batch(updates, |id| self.store.get::<Memory>(id).is_some())?;
                journal
                    .commit(updates, |turn| {
                        self.store.with_exclusive(|inner| {
                            Self::check_read_set(read_set, |id| GetById::<Memory>::get(inner, id))?;
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
    use crate::generic::memory::create_memory_production_stack;
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
            &path,
        )
        .unwrap();
        MemoryConnectionStore::new(GenericProductionStore::new(stack))
    }

    fn full_fields(n: u128) -> Vec<(FieldRef, ScanValue)> {
        MemoryConnectionStore::fields_of(memory(n, "decision", false))
    }

    #[test]
    fn get_returns_every_field_in_tag_order_and_describe_matches() {
        let adapter = sample_adapter();
        let fields = adapter.get(Uuid::from_u128(3)).unwrap();
        assert_eq!(fields.len(), 11);
        assert_eq!(
            fields[2],
            (FIELD_TAGS, ScanValue::StrList(vec!["t".into()]))
        );
        assert_eq!(fields[9], (FIELD_SENSITIVE, ScanValue::Bool(true)));
        let schema = adapter.describe();
        assert_eq!(schema.fields.len(), 11);
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
        assert!(!schema.relations.neighbors && !schema.relations.parent_children);
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
    fn insert_record_takes_all_eleven_fields_and_refuses_every_malformed_list() {
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
            adapter.insert_record(Uuid::from_u128(5), full_fields(5)[..10].to_vec()),
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
        assert_eq!(
            adapter.link_records(Uuid::from_u128(1), Uuid::from_u128(2), "x"),
            Err(ErrorCode::Unsupported)
        );
        assert!(adapter.list_relation_kinds().is_empty());
    }
}
