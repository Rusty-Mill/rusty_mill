//! `Memory` — this crate's sixth domain and third front-door one
//! (`MEM-FR-001`, ADR-0048, `docs/design/SERVER-MEMORY-DOMAIN-DESIGN.md`):
//! the consumer's own `memories` table, the one `ADR-0036` scoped out
//! ("schema-less memory content … a genuinely different kind of
//! database") and `ADR-0045` named as gate (ii) — "a `Memory` domain is
//! the obvious candidate" for the second table. It is buildable now
//! because the two rounds before it made a table *writable*: records
//! (`ADR-0046`) and, for the domains that have them, edges (`ADR-0047`).
//!
//! # Shape — a bounded projection of `memories`, not the whole table
//!
//! `rusty_remind_me`'s `memories` has thirty columns (`schema_tables.sql:
//! 56-64`). This record carries the eleven a memory *is* — what the
//! consumer's `add_memory` writes and its `list`/`get` read back — and
//! names what it leaves out (the module docs of `ADR-0048` and the
//! design's Non-goals): the retrieval-scoring triple (`decay_rate`,
//! `vitality`, `base_weight` — `f64`, which no stored `ValueKind`
//! carries), the sync bookkeeping (`node_id`, `client`), the SPO triple
//! and `superseded_by` (nullable, and the wire has no null), the
//! document chunking pair, `remind_at` (the `Reminder` domain's job),
//! and `deleted_at` (no runtime deletion, still).
//!
//! - `category` is the equality-filterable `IndexedField` — the
//!   consumer indexes it (`idx_memories_category`) and every list filters
//!   on it. Open `String`, as `Entity::kind` is (`ADR-0039`).
//! - `access_count` is the durably-mutable `ScannableField` — the one
//!   counter the consumer bumps on every retrieval, a plain `i64` as
//!   `Entity::mention_count` is.
//! - `tags` is a `Vec<String>` (`ScanValue::StrList`, protocol 11) —
//!   the consumer stores a JSON array and a `memory_tags` link table.
//! - `metadata_json` carries the consumer's JSON object *as text*: the
//!   store never parses it. This is the honest way to hold "schema-less
//!   content" in a fixed-schema record — it is a field the caller owns.
//! - `content`, `source`, `memory_type`, `status`, the two timestamps,
//!   and `sensitive` are read-only over the wire after insert. The
//!   consumer's `update_memory` edits `content`/`category`/`tags`/
//!   `metadata`/`sensitive` in place — a *whole-record replacement*
//!   this library does not have yet (`UpdateField` moves one `Copy`
//!   value). Named as the next round, not hidden.
//!
//! # Identity
//!
//! The consumer mints `mem_<uuid4 simple>` (`db/queries.rs:101`) and
//! its own `ADR-0016` declares ids opaque. This crate's `RecordId` is
//! `Uuid`; a bridge strips the prefix on the way in and restores it on
//! the way out, losslessly. No derivation, no convention beyond that.
//!
//! # No relation of either kind — yet
//!
//! `memory_entities` (memory → entity) and `memory_associations`
//! (memory ↔ memory) are the consumer's edges. The first is a
//! *cross-table* link, `ADR-0045`'s territory; the second could be a
//! `MultiSymmetric` over this record exactly as `Entity` has, and is
//! left for the round that also gives this table its second table.
//! So `MemoryProductionStack = GenericMmapStore<Memory, CategoryField,
//! AccessCountField>` — `Reminder`'s shape, the simplest this library
//! supports.

use super::mmap_store::GenericMmapStore;
use super::traits::{IndexedField, Record, ScannableField, SchemaTag};
use crate::durability::DurabilityError;
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

/// One memory — see the module docs for what each field carries and
/// what the consumer's table holds that this does not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    pub id: Uuid,
    /// The memory's text. Read-only over the wire after insert.
    pub content: String,
    /// The consumer's `category` (default `general`) — the
    /// equality-filterable `IndexedField` (`MEM-FR-002`).
    pub category: String,
    /// The consumer's JSON-array `tags`, as a list.
    pub tags: Vec<String>,
    /// The consumer's `source` (default `manual`).
    pub source: String,
    /// The consumer's `metadata` JSON object, carried as text and never
    /// parsed here (`MEM-FR-004`).
    pub metadata_json: String,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
    /// The consumer's classification: `unclassified`, `decision`,
    /// `fact`, `reference`, … — open, as `category` is.
    pub memory_type: String,
    /// The consumer's lifecycle state, `active` unless something
    /// archived it — open `String`.
    pub status: String,
    /// The consumer's "don't surface by default" flag — a convenience,
    /// not access control, on both sides.
    pub sensitive: bool,
    /// The consumer's retrieval counter — the durably-mutable
    /// `ScannableField` (`MEM-FR-003`).
    pub access_count: i64,
}

impl Record for Memory {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

// Part of the on-disk format (the blob and log headers) — see `Order`/
// `Reminder`'s own impls for the same caveat.
impl SchemaTag for Memory {
    const SCHEMA_TAG: &'static str = "memory::Memory";
}

/// `MEM-FR-002`: the equality-filterable field.
pub struct CategoryField;
impl IndexedField<CategoryField> for Memory {
    type IndexValue = String;
    fn indexed_value(&self) -> &String {
        &self.category
    }
}

/// `MEM-FR-003`: the durably-mutable field.
pub struct AccessCountField;
impl ScannableField<AccessCountField> for Memory {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.access_count
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.access_count = value;
    }
}

/// The durable production stack — `MEM-FR-005`: no relation of either
/// kind, so `GenericMmapStore` directly.
pub type MemoryProductionStack = GenericMmapStore<Memory, CategoryField, AccessCountField>;

/// Build a fresh, durable production store for `Memory` at `path` —
/// two files, `path` (the mmap file) and `<path>.records` (the record
/// blob); an insert log appears beside them once something is inserted.
///
/// # Errors
///
/// Returns [`DurabilityError::Io`] under the same conditions
/// [`GenericMmapStore::create`] does; [`DurabilityError::Serde`] if
/// `memories` can't be serialized.
pub fn create_memory_production_stack(
    memories: Vec<Memory>,
    path: &Path,
) -> Result<MemoryProductionStack, DurabilityError> {
    GenericMmapStore::<Memory, CategoryField, AccessCountField>::create(memories, path)
}

/// Reopen an existing durable production store for `Memory` at `path`
/// from a caller-supplied record set — the insert log is folded in
/// (`INS-FR-004`).
///
/// # Errors
///
/// Everything [`GenericMmapStore::open`] can return.
pub fn open_memory_production_stack(
    memories: Vec<Memory>,
    path: &Path,
) -> Result<MemoryProductionStack, DurabilityError> {
    GenericMmapStore::<Memory, CategoryField, AccessCountField>::open(memories, path)
}

/// Reopen from the files alone — the blob, the insert log, and the
/// mmap file — the way a server restarted on the same directory does.
///
/// # Errors
///
/// Everything [`GenericMmapStore::open_portable`] can return.
pub fn open_memory_production_stack_portable(
    path: &Path,
) -> Result<MemoryProductionStack, DurabilityError> {
    GenericMmapStore::<Memory, CategoryField, AccessCountField>::open_portable(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generic::query::{AllIds, FilterEq, GetById, Insert, ScanField, UpdateField};
    use crate::test_support::fresh_temp_dir;

    pub(crate) fn memory(n: u128, category: &str, sensitive: bool) -> Memory {
        Memory {
            id: Uuid::from_u128(n),
            content: format!("memory {n}"),
            category: category.into(),
            tags: vec!["t".into(), format!("n{n}")],
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

    fn sample() -> Vec<Memory> {
        vec![
            memory(1, "general", false),
            memory(2, "preference", false),
            memory(3, "general", true),
        ]
    }

    /// `MEM` acceptance criterion 1: every read and the one write work
    /// on the stack, and an insert with every field — a `StrList`
    /// included — survives a reopen from the files alone.
    #[test]
    fn create_get_filter_scan_update_insert_and_portable_reopen() {
        let dir = fresh_temp_dir("generic_memory").unwrap();
        let path = dir.join("memories.mmap");
        {
            let mut store = create_memory_production_stack(sample(), &path).unwrap();
            let got = GetById::<Memory>::get(&store, Uuid::from_u128(3)).unwrap();
            assert_eq!(got.content, "memory 3");
            assert!(got.sensitive);
            let mut general =
                FilterEq::<Memory, CategoryField>::filter_eq(&store, &"general".into());
            general.sort();
            assert_eq!(general, vec![Uuid::from_u128(1), Uuid::from_u128(3)]);
            UpdateField::<Memory, AccessCountField>::update(&mut store, Uuid::from_u128(1), 7)
                .unwrap();
            let mut counts = ScanField::<Memory, AccessCountField>::scan(&store);
            counts.sort_unstable();
            assert_eq!(counts, vec![0, 0, 7]);

            let mut new = memory(4, "decision", false);
            new.tags = vec!["important".into()];
            new.metadata_json = r#"{"source_url":"https://example"}"#.into();
            Insert::<Memory>::insert(&mut store, new.clone()).unwrap();
            assert_eq!(GetById::<Memory>::get(&store, new.id), Some(new));
            assert_eq!(AllIds::<Memory>::all_ids(&store).len(), 4);
        }
        let reopened = open_memory_production_stack_portable(&path).unwrap();
        let got = GetById::<Memory>::get(&reopened, Uuid::from_u128(4)).unwrap();
        assert_eq!(got.tags, vec!["important".to_string()]);
        assert_eq!(got.metadata_json, r#"{"source_url":"https://example"}"#);
        assert_eq!(
            GetById::<Memory>::get(&reopened, Uuid::from_u128(1))
                .unwrap()
                .access_count,
            7,
            "the slot survived"
        );
        assert_eq!(
            FilterEq::<Memory, CategoryField>::filter_eq(&reopened, &"decision".into()),
            vec![Uuid::from_u128(4)]
        );
    }
}
