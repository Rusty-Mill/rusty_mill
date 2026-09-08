//! The `Relation` domain — `REL-FR-001`–`005`, ADR-0058,
//! `docs/design/SERVER-RELATION-DOMAIN-DESIGN.md`: the consumer's
//! `entity_relations` table, a **directed, open-label edge with
//! metadata** (`subject --relation--> object`, when, from which node,
//! soft-deleted or not), modelled as a **record table** rather than as a
//! new edge primitive.
//!
//! # Why a table, not an edge layer
//!
//! `Symmetric`/`MultiSymmetric` keep undirected, metadata-free edges under
//! both endpoints. The consumer's relation is directed, carries a label a
//! caller supplies, and carries four more columns (`created_at`,
//! `updated_at`, `node_id`, `deleted_at`) it syncs by last-writer-wins —
//! which is to say it is a *record*, and every record capability this
//! crate has is exactly what it needs: `Insert`/`Replace`/`ReplaceIf`
//! (the merge), `Delete`, `Page` by `updated_at` (the pull), `Query` by
//! any field (out-edges, in-edges, one label), `Aggregate` (counts),
//! `Compact`. Endpoints are the consumer's own id strings, stored
//! verbatim: this table asserts nothing about the `entity` table, exactly
//! as the consumer's own foreign-key-less table does.
//!
//! # Shape
//!
//! - `subject` is the `IndexedField` — the one lookup a graph walk
//!   starts from (`FilterEq`, then filter by `relation` client-side or
//!   through `Query`).
//! - `updated_at_unix_ms` is the `ScannableField` — what `Page` orders
//!   by and last-writer-wins guards on.
//! - `node_id` and `deleted_at_unix_ms` carry `ADR-0056`'s sentinels:
//!   `""` unattributed, `0` live.
//! - No relation layer of its own: the stack is `GenericMmapStore`
//!   directly, `Reminder`'s shape.

use super::mmap_store::GenericMmapStore;
use super::traits::{IndexedField, Record, ScannableField, SchemaTag};
use crate::durability::DurabilityError;
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

/// One directed, labelled edge with the consumer's sync bookkeeping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relation {
    pub id: Uuid,
    /// The consumer's id of the edge's source, stored verbatim — never
    /// empty.
    pub subject: String,
    /// The caller's label — never empty; any string, no fixed set.
    pub relation: String,
    /// The consumer's id of the edge's target, stored verbatim — never
    /// empty.
    pub object: String,
    pub created_at_unix_ms: i64,
    /// The scannable field: what `Page` orders by and a last-writer-wins
    /// `ReplaceIf` guards on.
    pub updated_at_unix_ms: i64,
    /// `""` for an unattributed edge (`ADR-0056`'s sentinel).
    pub node_id: String,
    /// `0` for a live edge (`ADR-0056`'s sentinel); validated non-negative.
    pub deleted_at_unix_ms: i64,
}

impl Record for Relation {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

// Part of the on-disk format (the blob and log headers) — see `Memory`'s
// impl for the layout-change caveat.
impl SchemaTag for Relation {
    const SCHEMA_TAG: &'static str = "relation::Relation";
}

/// `REL-FR-002`: the equality-filterable field — every edge out of one
/// subject.
pub struct SubjectField;
impl IndexedField<SubjectField> for Relation {
    type IndexValue = String;
    fn indexed_value(&self) -> &String {
        &self.subject
    }
}

/// `REL-FR-002`: the durably-mutable field — the sync timestamp.
pub struct UpdatedAtField;
impl ScannableField<UpdatedAtField> for Relation {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.updated_at_unix_ms
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.updated_at_unix_ms = value;
    }
}

/// The durable production stack — `REL-FR-003`: no relation layer, just
/// `GenericMmapStore` directly, `Reminder`'s shape.
pub type RelationProductionStack = GenericMmapStore<Relation, SubjectField, UpdatedAtField>;

/// Build a fresh durable store at `path`.
///
/// # Errors
///
/// Everything [`GenericMmapStore::create`] can return.
pub fn create_relation_production_stack(
    relations: Vec<Relation>,
    path: &Path,
) -> Result<RelationProductionStack, DurabilityError> {
    GenericMmapStore::<Relation, SubjectField, UpdatedAtField>::create(relations, path)
}

/// Reopen from the files alone — records and the insert log.
///
/// # Errors
///
/// Everything [`GenericMmapStore::open_portable`] can return.
pub fn open_relation_production_stack_portable(
    path: &Path,
) -> Result<RelationProductionStack, DurabilityError> {
    GenericMmapStore::<Relation, SubjectField, UpdatedAtField>::open_portable(path)
}

/// Open the stack at `path` if its slot file exists, else create an
/// empty one — `ADR-0053`'s shape for a served process.
///
/// # Errors
///
/// Everything [`create_relation_production_stack`] or
/// [`open_relation_production_stack_portable`] can return.
pub fn open_or_create_relation_production_stack(
    path: &Path,
) -> Result<RelationProductionStack, DurabilityError> {
    if path.exists() {
        return open_relation_production_stack_portable(path);
    }
    create_relation_production_stack(Vec::new(), path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generic::query::{AllIds, FilterEq, GetById, ScanField};
    use crate::test_support::fresh_temp_dir;

    pub(crate) fn relation(n: u128, subject: &str, label: &str, object: &str) -> Relation {
        Relation {
            id: Uuid::from_u128(n),
            subject: subject.into(),
            relation: label.into(),
            object: object.into(),
            created_at_unix_ms: 1_000 * n as i64,
            updated_at_unix_ms: 1_000 * n as i64,
            node_id: String::new(),
            deleted_at_unix_ms: 0,
        }
    }

    /// `REL-FR-002`/`003`: out-edges by subject through the index, the
    /// timestamps through the scan; a runtime insert and a delete survive
    /// the portable reopen and the open-or-create path.
    #[test]
    fn subject_index_and_updated_at_scan_survive_every_reopen() {
        let dir = fresh_temp_dir("generic_relation").unwrap();
        let path = dir.join("relations.mmap");
        let mut store = create_relation_production_stack(
            vec![
                relation(1, "aaa", "works_with", "bbb"),
                relation(2, "aaa", "located_in", "ccc"),
                relation(3, "bbb", "works_with", "aaa"),
            ],
            &path,
        )
        .unwrap();
        let mut out = store.filter_eq(&"aaa".to_string());
        out.sort();
        assert_eq!(out, vec![Uuid::from_u128(1), Uuid::from_u128(2)]);
        assert_eq!(store.filter_eq(&"zzz".to_string()), Vec::<Uuid>::new());
        let mut stamps = store.scan();
        stamps.sort();
        assert_eq!(stamps, vec![1_000, 2_000, 3_000]);

        store.insert(relation(4, "ccc", "part_of", "aaa")).unwrap();
        store.delete(Uuid::from_u128(2)).unwrap();
        drop(store);

        let reopened = open_relation_production_stack_portable(&path).unwrap();
        assert_eq!(reopened.all_ids().len(), 3);
        assert!(GetById::<Relation>::get(&reopened, Uuid::from_u128(2)).is_none());
        assert_eq!(
            reopened.filter_eq(&"ccc".to_string()),
            vec![Uuid::from_u128(4)]
        );
        drop(reopened);

        let again = open_or_create_relation_production_stack(&path).unwrap();
        assert_eq!(again.all_ids().len(), 3, "reopened, not recreated");
        let fresh = open_or_create_relation_production_stack(&dir.join("other.mmap")).unwrap();
        assert!(fresh.all_ids().is_empty(), "created empty");
    }
}
